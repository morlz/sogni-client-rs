use ring::aead::{Aad, Nonce};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{
    MediaEncryptionError as E, MediaKey,
    codec::{aes_key, derive, random},
};

const SALT: &[u8] = b"sogni-media-encryption";
const RECIPIENT_AAD: &[u8] = b"sogni-media-recipient-v1";
const INPUT_KEY_AAD: &[u8] = b"sogni-media-input-key-v1";
const MAX_CONTENT_KEYED_INPUT: usize = 100 * 1024 * 1024;

type Result<T> = std::result::Result<T, E>;

#[derive(Clone)]
pub(super) struct AccountKey {
    secret: MediaKey,
    check_key: Zeroizing<[u8; 32]>,
    input_key: Zeroizing<[u8; 32]>,
}

impl AccountKey {
    pub(super) fn new(secret: MediaKey) -> Result<Self> {
        Ok(Self {
            check_key: derive(secret.as_bytes(), SALT, b"key-check-secret-v1")?,
            input_key: derive(secret.as_bytes(), SALT, b"input-secret-v1")?,
            secret,
        })
    }

    fn check(&self, nonce: &[u8; 12]) -> Result<Zeroizing<[u8; 8]>> {
        derive(self.check_key.as_ref(), nonce, b"key-check-v1")
    }

    pub(super) fn wrap(&self, key: &MediaKey, nonce: [u8; 12]) -> Result<[u8; 69]> {
        let mut result = [0; 69];
        result[0] = 1;
        result[1..9].copy_from_slice(self.check(&nonce)?.as_ref());
        result[9..21].copy_from_slice(&nonce);
        seal_key(&self.secret, key, nonce, RECIPIENT_AAD, &mut result[21..])?;
        Ok(result)
    }

    pub(super) fn input_material(
        &self,
        content: &[u8],
        content_type: &str,
    ) -> Result<InputMaterial> {
        let material = if content.len() > MAX_CONTENT_KEYED_INPUT {
            Zeroizing::new(random::<83>()?)
        } else {
            let digest = Sha256::digest(content);
            let info = [b"input-media-v1\0".as_slice(), content_type.as_bytes()].concat();
            derive(self.input_key.as_ref(), &digest, &info)?
        };
        Ok(InputMaterial {
            key: MediaKey::try_from(&material[..32])?,
            salt: material[32..64].try_into().expect("32 bytes"),
            nonce_prefix: material[64..71].try_into().expect("7 bytes"),
            recipient_nonce: material[71..83].try_into().expect("12 bytes"),
        })
    }
}

pub(super) struct InputMaterial {
    pub(super) key: MediaKey,
    pub(super) salt: [u8; 32],
    pub(super) nonce_prefix: [u8; 7],
    pub(super) recipient_nonce: [u8; 12],
}

pub(super) fn unwrap(accounts: &[AccountKey], record: &[u8]) -> Result<MediaKey> {
    if record.is_empty() {
        return Err(E::NoRecipient);
    }
    if record.len() != 69 || record[0] != 1 {
        return Err(E::Malformed("unrecognized recipient record"));
    }
    let nonce = record[9..21].try_into().expect("12 bytes");
    for account in accounts {
        let expected = account.check(&nonce)?;
        if bool::from(record[1..9].ct_eq(expected.as_ref())) {
            let aes = aes_key(account.secret.as_bytes())?;
            let mut wrapped = Zeroizing::new([0u8; 48]);
            wrapped.copy_from_slice(&record[21..]);
            let opened = aes
                .open_in_place(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(RECIPIENT_AAD),
                    &mut *wrapped,
                )
                .map_err(|_| E::AuthenticationFailed)?;
            return MediaKey::try_from(&*opened);
        }
    }
    Err(E::WrongAccountKey)
}

pub(super) fn wrap_input(
    project: &MediaKey,
    input: &MediaKey,
    nonce: [u8; 12],
) -> Result<[u8; 60]> {
    let mut result = [0; 60];
    result[..12].copy_from_slice(&nonce);
    seal_key(project, input, nonce, INPUT_KEY_AAD, &mut result[12..])?;
    Ok(result)
}

fn seal_key(
    key: &MediaKey,
    inner: &MediaKey,
    nonce: [u8; 12],
    aad: &[u8],
    output: &mut [u8],
) -> Result<()> {
    let aes = aes_key(key.as_bytes())?;
    let mut sealed = Zeroizing::new(*inner.as_bytes());
    let tag = aes
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut *sealed,
        )
        .map_err(|_| E::AuthenticationFailed)?;
    output[..32].copy_from_slice(sealed.as_ref());
    output[32..].copy_from_slice(tag.as_ref());
    Ok(())
}
