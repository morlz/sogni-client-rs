use super::typed_data::{TypedData, keccak256};
use ethabi::Address;
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use zeroize::Zeroizing;

/// A private signer; no key material is exposed through the SDK API or Debug.
pub(in crate::account) struct LocalWallet {
    secret: Zeroizing<[u8; 32]>,
    address: Address,
}

struct EphemeralSecret(SecretKey);

impl Drop for EphemeralSecret {
    fn drop(&mut self) {
        self.0.non_secure_erase();
    }
}

impl LocalWallet {
    pub(in crate::account) fn from_bytes(bytes: &[u8; 32]) -> Result<Self, secp256k1::Error> {
        let key = EphemeralSecret(SecretKey::from_byte_array(*bytes)?);
        let context = Secp256k1::new();
        let public = PublicKey::from_secret_key(&context, &key.0).serialize_uncompressed();
        Ok(Self {
            secret: Zeroizing::new(*bytes),
            address: Address::from_slice(&keccak256(&public[1..])[12..]),
        })
    }

    pub(in crate::account) fn address(&self) -> Address {
        self.address
    }

    pub(super) async fn sign_typed_data(&self, data: &TypedData) -> Result<String, String> {
        let hash = data
            .eip712_signing_hash()
            .map_err(|error| error.to_string())?;
        let key = EphemeralSecret(
            SecretKey::from_byte_array(*self.secret).map_err(|error| error.to_string())?,
        );
        // The context is randomized by secp256k1's rand/std features. RFC6979
        // signatures remain deterministic and use Ethereum's low-S form.
        let signature = Secp256k1::new().sign_ecdsa_recoverable(Message::from_digest(hash), &key.0);
        let (recovery, compact) = signature.serialize_compact();
        let mut encoded = [0_u8; 65];
        encoded[..64].copy_from_slice(&compact);
        encoded[64] = i32::from(recovery) as u8 + 27;
        Ok(hex::encode(encoded))
    }
}
