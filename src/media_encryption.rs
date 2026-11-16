use std::fmt;

use bytes::Bytes;
use zeroize::Zeroizing;

mod api;
#[cfg(test)]
mod auto_tests;
mod codec;
mod project;
mod recipient;
mod remote;
#[cfg(test)]
mod tests;

pub(crate) use api::{MEDIA_CAPABILITY, MediaKeyState};
pub use api::{MediaEncryptionApi, MediaEncryptionMode};
pub use codec::{MediaEncryptionHeader, decrypt_media, encrypt_media, is_encrypted_media};
pub(crate) use project::{ProjectEncryption, redact_media_keys};
pub use remote::DecryptedMediaReader;

/// Encryption failures distinguish unavailable keys from malformed or changed
/// media. No error includes keys or a signed media URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaEncryptionError {
    #[error("media is not Sogni-encrypted")]
    NotEncrypted,
    #[error("encrypted media version {0} is not supported")]
    UnsupportedVersion(u8),
    #[error("invalid encrypted media: {0}")]
    Malformed(&'static str),
    #[error("encrypted media failed authentication")]
    AuthenticationFailed,
    #[error("the media key is locked or its account session ended")]
    Locked,
    #[error("this media was encrypted for a different account key")]
    WrongAccountKey,
    #[error("this media has no account recipient; its project key is required")]
    NoRecipient,
    #[error("media encryption is not supported: {0}")]
    Unsupported(&'static str),
}

/// A 32-byte media key. Its bytes are cleared on drop and are never printed.
#[derive(Clone)]
pub struct MediaKey(Zeroizing<[u8; 32]>);

impl MediaKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub(super) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl TryFrom<&[u8]> for MediaKey {
    type Error = MediaEncryptionError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self::from_bytes(bytes.try_into().map_err(|_| {
            MediaEncryptionError::Malformed("a media key must contain 32 bytes")
        })?))
    }
}

impl fmt::Debug for MediaKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaKey").finish_non_exhaustive()
    }
}

/// Media bytes and the MIME type before encryption (or from the download if
/// it was already plaintext).
pub struct DecryptedMedia {
    pub data: Bytes,
    pub content_type: String,
    pub header: Option<MediaEncryptionHeader>,
}

impl fmt::Debug for DecryptedMedia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecryptedMedia")
            .field("bytes", &self.data.len())
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}
