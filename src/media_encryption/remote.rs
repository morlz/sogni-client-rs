use std::{fmt, ops::Range, sync::Arc};

use bytes::Bytes;
use futures_util::{Stream, TryStreamExt};
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use super::{
    MediaEncryptionHeader, MediaKeyState,
    api::MediaAccess,
    codec::{Framing, MAX_HEADER_BYTES, TAG_BYTES},
};
use crate::{Error, Result, transport::RestClient};

/// Authenticated plaintext byte ranges from a remote encrypted container.
/// Drop a stream to cancel its current request; closing the reader cancels
/// every request. Locking or changing accounts also invalidates the reader.
pub struct DecryptedMediaReader {
    access: MediaAccess,
    rest: RestClient,
    url: Url,
    header: MediaEncryptionHeader,
    framing: Framing,
    total: u64,
    keys: Arc<MediaKeyState>,
    closed: CancellationToken,
}

impl fmt::Debug for DecryptedMediaReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecryptedMediaReader")
            .field("size", &self.size())
            .field("content_type", &self.content_type())
            .finish_non_exhaustive()
    }
}

pub(super) async fn open(
    keys: Arc<MediaKeyState>,
    rest: RestClient,
    access: MediaAccess,
    url: Url,
) -> Result<DecryptedMediaReader> {
    let (prefix, total) = access
        .run(rest.media_range(&url, 0, MAX_HEADER_BYTES as u64, None))
        .await?;
    let header = MediaEncryptionHeader::parse(&prefix)?;
    let framing = Framing::new(&header, total)?;
    let reader = DecryptedMediaReader {
        access,
        rest,
        url,
        header,
        framing,
        total,
        keys,
        closed: CancellationToken::new(),
    };
    // Verify the header and total length before exposing metadata or ranges.
    reader.segment(0).await?.zeroize();
    if reader.framing.last > 0 {
        reader.segment(reader.framing.last).await?.zeroize();
    }
    reader.check()?;
    Ok(reader)
}

impl DecryptedMediaReader {
    #[must_use]
    pub fn size(&self) -> u64 {
        self.framing.plaintext_len
    }

    #[must_use]
    pub fn content_type(&self) -> &str {
        &self.header.content_type
    }

    pub fn close(&self) {
        self.closed.cancel();
    }

    pub async fn read(&self, range: Range<u64>) -> Result<Bytes> {
        self.validate_range(&range)?;
        let len = usize::try_from(range.end - range.start)
            .map_err(|_| Error::InvalidInput("decrypted byte range is too large".into()))?;
        let mut bytes = Zeroizing::new(Vec::new());
        bytes
            .try_reserve_exact(len)
            .map_err(|_| Error::InvalidInput("decrypted byte range is too large".into()))?;
        let stream = self.stream(range);
        futures_util::pin_mut!(stream);
        while let Some(chunk) = stream.try_next().await? {
            bytes.extend_from_slice(&chunk);
        }
        self.check()?;
        Ok(Bytes::from(std::mem::take(&mut *bytes)))
    }

    pub fn stream(&self, range: Range<u64>) -> impl Stream<Item = Result<Bytes>> + '_ {
        async_stream::try_stream! {
            self.validate_range(&range)?;
            let mut offset = range.start;
            let segment_size = self.header.segment_size() as u64;
            while offset < range.end {
                let plaintext = Zeroizing::new(self.segment((offset / segment_size) as u32).await?);
                let from = (offset % segment_size) as usize;
                let count = (range.end - offset).min((plaintext.len() - from) as u64) as usize;
                let chunk = Bytes::copy_from_slice(&plaintext[from..from + count]);
                self.check()?;
                offset += count as u64;
                yield chunk;
            }
            self.check()?;
        }
    }

    fn validate_range(&self, range: &Range<u64>) -> Result<()> {
        self.check()?;
        if range.start > range.end || range.end > self.size() {
            return Err(Error::InvalidInput(
                "decrypted byte range is outside the media".into(),
            ));
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if self.closed.is_cancelled() {
            return Err(Error::Closed);
        }
        self.access.check()
    }

    async fn segment(&self, index: u32) -> Result<Vec<u8>> {
        self.check()?;
        let length = (self.header.segment_size() + TAG_BYTES) as u64;
        let start = self.header.len() as u64 + u64::from(index) * length;
        let request = self.rest.media_range(
            &self.url,
            start,
            (start + length).min(self.total),
            Some(self.total),
        );
        let result = tokio::select! {
            biased;
            () = self.closed.cancelled() => return Err(Error::Closed),
            result = self.access.run(request) => result,
        };
        self.check()?;
        let (data, _) = result?;
        let key = self
            .header
            .key(&self.keys.key_for(&self.header, &self.access)?)?;
        let mut plaintext = Zeroizing::new(self.header.open_segment(
            &key,
            index,
            index == self.framing.last,
            data,
        )?);
        self.check()?;
        Ok(std::mem::take(&mut *plaintext))
    }
}
