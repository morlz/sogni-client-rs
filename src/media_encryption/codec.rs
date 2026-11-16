use bytes::Bytes;
use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    hkdf,
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

use super::{DecryptedMedia, MediaEncryptionError as E, MediaKey};

type Result<T> = std::result::Result<T, E>;
const MAGIC: &[u8; 8] = b"SOGNIE2E";
pub(super) const VERSION: u8 = 1;
pub(super) const DEFAULT_SEGMENT_LOG2: u8 = 20;
pub(super) const TAG_BYTES: usize = 16;
const HEADER_PREFIX: usize = 50;
const MAX_RECIPIENT: usize = 512;
pub(super) const MAX_HEADER_BYTES: usize = HEADER_PREFIX + 255 + 2 + MAX_RECIPIENT;

/// Parsed version 1 header. The content type and recipient are authenticated
/// when a segment is successfully decrypted.
#[derive(Clone, Debug)]
pub struct MediaEncryptionHeader {
    pub version: u8,
    pub segment_size_log2: u8,
    pub content_type: String,
    pub recipient: Vec<u8>,
    salt: [u8; 32],
    nonce_prefix: [u8; 7],
    encoded: Vec<u8>,
}

impl MediaEncryptionHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if !is_encrypted_media(bytes) {
            return Err(E::NotEncrypted);
        }
        let prefix = bytes
            .get(..HEADER_PREFIX)
            .ok_or(E::Malformed("truncated header"))?;
        if prefix[8] != VERSION {
            return Err(E::UnsupportedVersion(prefix[8]));
        }
        let segment_size_log2 = prefix[9];
        validate_segment_size(segment_size_log2)?;
        let content_end = HEADER_PREFIX + usize::from(prefix[49]);
        let length = bytes
            .get(content_end..content_end + 2)
            .ok_or(E::Malformed("truncated header"))?;
        let recipient_length = u16::from_be_bytes([length[0], length[1]]) as usize;
        if recipient_length > MAX_RECIPIENT {
            return Err(E::Malformed("recipient is too long"));
        }
        let header_end = content_end + 2 + recipient_length;
        let encoded = bytes
            .get(..header_end)
            .ok_or(E::Malformed("truncated recipient"))?;
        let content_type = std::str::from_utf8(&encoded[HEADER_PREFIX..content_end])
            .map_err(|_| E::Malformed("invalid content type"))?
            .trim_start_matches('\u{feff}')
            .to_owned();
        Ok(Self {
            version: VERSION,
            segment_size_log2,
            content_type,
            recipient: encoded[content_end + 2..].to_vec(),
            salt: prefix[10..42].try_into().expect("32 bytes"),
            nonce_prefix: prefix[42..49].try_into().expect("7 bytes"),
            encoded: encoded.to_vec(),
        })
    }

    pub(super) fn len(&self) -> usize {
        self.encoded.len()
    }

    pub(super) fn segment_size(&self) -> usize {
        1 << self.segment_size_log2
    }

    pub(super) fn key(&self, project_key: &MediaKey) -> Result<LessSafeKey> {
        let derived = derive::<32>(project_key.as_bytes(), &self.salt, &self.encoded)?;
        aes_key(derived.as_ref())
    }

    pub(super) fn open_segment(
        &self,
        key: &LessSafeKey,
        index: u32,
        last: bool,
        mut data: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let nonce = segment_nonce(self.nonce_prefix, index, last);
        let len = key
            .open_in_place(nonce, Aad::empty(), &mut data)
            .map_err(|_| E::AuthenticationFailed)?
            .len();
        data.truncate(len);
        Ok(data)
    }
}

#[must_use]
pub fn is_encrypted_media(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Encrypt bytes with the given project key. The optional recipient record
/// lets the corresponding account open the resulting file.
pub fn encrypt_media(
    data: &[u8],
    key: &MediaKey,
    content_type: &str,
    recipient: &[u8],
) -> Result<Bytes> {
    seal(
        data,
        key,
        content_type,
        recipient,
        DEFAULT_SEGMENT_LOG2,
        random()?,
        random()?,
    )
}

/// Decrypt a complete container with its project key. A segment is released
/// only after its authentication tag is checked.
pub fn decrypt_media(data: &[u8], key: &MediaKey) -> Result<DecryptedMedia> {
    let header = MediaEncryptionHeader::parse(data)?;
    open(data, header, key)
}

pub(super) fn open(
    data: &[u8],
    header: MediaEncryptionHeader,
    key: &MediaKey,
) -> Result<DecryptedMedia> {
    let body = &data[header.len()..];
    let key = header.key(key)?;
    let framing = Framing::new(&header, data.len() as u64)?;
    let mut plaintext = Vec::with_capacity(framing.plaintext_len as usize);
    for (index, segment) in body.chunks(header.segment_size() + TAG_BYTES).enumerate() {
        let index = index as u32; // Framing has already bounded the segment count.
        plaintext.extend(header.open_segment(
            &key,
            index,
            index == framing.last,
            segment.to_vec(),
        )?);
    }
    Ok(DecryptedMedia {
        data: Bytes::from(plaintext),
        content_type: header.content_type.clone(),
        header: Some(header),
    })
}

pub(super) fn seal(
    data: &[u8],
    key: &MediaKey,
    content_type: &str,
    recipient: &[u8],
    segment_log2: u8,
    salt: [u8; 32],
    nonce_prefix: [u8; 7],
) -> Result<Bytes> {
    validate_segment_size(segment_log2)?;
    let content_length =
        u8::try_from(content_type.len()).map_err(|_| E::Malformed("content type is too long"))?;
    if recipient.len() > MAX_RECIPIENT {
        return Err(E::Malformed("recipient is too long"));
    }
    let mut encoded = Vec::new();
    encoded.extend_from_slice(MAGIC);
    encoded.extend([VERSION, segment_log2]);
    encoded.extend(salt);
    encoded.extend(nonce_prefix);
    encoded.push(content_length);
    encoded.extend_from_slice(content_type.as_bytes());
    encoded.extend((recipient.len() as u16).to_be_bytes());
    encoded.extend_from_slice(recipient);
    let derived = derive::<32>(key.as_bytes(), &salt, &encoded)?;
    let file_key = aes_key(derived.as_ref())?;
    let size = 1 << segment_log2;
    let segments = data.len().div_ceil(size).max(1);
    let last = u32::try_from(segments - 1).map_err(|_| E::Malformed("too many segments"))?;
    for index in 0..segments {
        // Nonces bind the index and final-segment marker. An empty container
        // still has one authenticated segment.
        let start = index * size;
        let plaintext = &data[start..(start + size).min(data.len())];
        let offset = encoded.len();
        encoded.extend_from_slice(plaintext);
        let nonce = segment_nonce(nonce_prefix, index as u32, index == last as usize);
        let tag = file_key
            .seal_in_place_separate_tag(nonce, Aad::empty(), &mut encoded[offset..])
            .map_err(|_| E::AuthenticationFailed)?;
        encoded.extend_from_slice(tag.as_ref());
    }
    Ok(Bytes::from(encoded))
}

pub(super) struct Framing {
    pub(super) plaintext_len: u64,
    pub(super) last: u32,
}

impl Framing {
    pub(super) fn new(header: &MediaEncryptionHeader, file_length: u64) -> Result<Self> {
        let body = file_length
            .checked_sub(header.len() as u64)
            .filter(|length| *length >= TAG_BYTES as u64)
            .ok_or(E::Malformed("no authenticated content"))?;
        let size = (header.segment_size() + TAG_BYTES) as u64;
        let segments = body.div_ceil(size);
        let final_len = body - (segments - 1) * size;
        if final_len < TAG_BYTES as u64 {
            return Err(E::Malformed("partial final segment"));
        }
        let last = u32::try_from(segments - 1).map_err(|_| E::Malformed("too many segments"))?;
        Ok(Self {
            plaintext_len: body - segments * TAG_BYTES as u64,
            last,
        })
    }
}

fn validate_segment_size(log2: u8) -> Result<()> {
    (16..=24)
        .contains(&log2)
        .then_some(())
        .ok_or(E::Malformed("unsupported segment size"))
}

fn segment_nonce(prefix: [u8; 7], index: u32, last: bool) -> Nonce {
    let mut bytes = [0; 12];
    bytes[..7].copy_from_slice(&prefix);
    bytes[7..11].copy_from_slice(&index.to_be_bytes());
    bytes[11] = u8::from(last);
    Nonce::assume_unique_for_key(bytes)
}

pub(super) fn aes_key(key: &[u8]) -> Result<LessSafeKey> {
    UnboundKey::new(&aead::AES_256_GCM, key)
        .map(LessSafeKey::new)
        .map_err(|_| E::Malformed("invalid AES-256 key"))
}

pub(super) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0; N];
    SystemRandom::new()
        .fill(&mut buf)
        .map_err(|_| E::Unsupported("secure randomness unavailable"))?;
    Ok(buf)
}

struct HkdfLength(usize);
impl hkdf::KeyType for HkdfLength {
    fn len(&self) -> usize {
        self.0
    }
}

pub(super) fn derive<const N: usize>(
    secret: &[u8],
    salt: &[u8],
    info: &[u8],
) -> Result<Zeroizing<[u8; N]>>
where
    [u8; N]: zeroize::Zeroize,
{
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, salt);
    let prk = salt.extract(secret);
    let info = [info];
    let okm = prk
        .expand(&info, HkdfLength(N))
        .map_err(|_| E::Malformed("HKDF output length"))?;
    let mut result = Zeroizing::new([0; N]);
    okm.fill(result.as_mut())
        .map_err(|_| E::Malformed("HKDF output length"))?;
    Ok(result)
}
