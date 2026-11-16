use super::*;
use crate::MediaEncryptionError as E;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE};

impl RestClient {
    pub(crate) async fn download_media(&self, url: Url) -> Result<(Bytes, String)> {
        validate_url(&url)?;
        let response = self
            .media_client(&url)
            .await?
            .get(url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        Ok((bytes, content_type))
    }

    /// A strict, bounded request for [start, end) bytes. Only a matching
    /// partial response can be interpreted as encrypted framing.
    pub(crate) async fn media_range(
        &self,
        url: &Url,
        start: u64,
        end: u64,
        expected_total: Option<u64>,
    ) -> Result<(Vec<u8>, u64)> {
        validate_url(url)?;
        let response = self
            .media_client_without_redirects(url)
            .await?
            .get(url.clone())
            .header(RANGE, format!("bytes={start}-{}", end - 1))
            .header(ACCEPT_ENCODING, "identity")
            .timeout(self.timeout)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(CONTENT_ENCODING)
                .is_some_and(|value| value != "identity")
        {
            return Err(E::Unsupported("encrypted playback requires HTTP byte ranges").into());
        }
        let value = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or(E::Unsupported("encrypted playback requires Content-Range"))?;
        let (first, last, total) =
            parse_range(value).ok_or(E::Malformed("invalid Content-Range"))?;
        if start != first
            || last >= total
            || first > last
            || last + 1 != end.min(total)
            || expected_total.is_some_and(|size| total != size)
        {
            return Err(E::Malformed("invalid media byte range").into());
        }
        let length = usize::try_from(last - first + 1)
            .map_err(|_| E::Malformed("media byte range is too large"))?;
        let mut body = Vec::with_capacity(length);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(reqwest::Error::without_url)?;
            if chunk.len() > length - body.len() {
                return Err(E::Malformed("media response exceeds its byte range").into());
            }
            body.extend_from_slice(&chunk);
        }
        if body.len() != length {
            return Err(E::Malformed("media byte range is truncated").into());
        }
        Ok((body, total))
    }
}

fn parse_range(value: &str) -> Option<(u64, u64, u64)> {
    let (segment, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = segment.split_once('-')?;
    let parse = |value: &str| {
        value
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| value.parse::<u64>().ok())
            .flatten()
    };
    Some((parse(first)?, parse(last)?, parse(total)?))
}

fn validate_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::InvalidInput(
            "media URL must be HTTP(S) without credentials".into(),
        ));
    }
    Ok(())
}
