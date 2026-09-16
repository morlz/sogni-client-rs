use ethers_core::types::transaction::eip712::TypedData;
use ethers_signers::{LocalWallet, Signer};
use pbkdf2::pbkdf2_hmac;
use serde_json::{Map, Value, json};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use super::AccountApi;
use crate::{ApiError, Error, Result};

const VERIFYING_CONTRACT: &str = "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC";

impl AccountApi {
    pub(super) fn current_wallet(&self, password: &str) -> Result<LocalWallet> {
        let username = self.current.username().ok_or_else(|| {
            Error::InvalidInput("current account username is unavailable; call me() first".into())
        })?;
        let expected = self.current.wallet_address().ok_or_else(|| {
            Error::InvalidInput(
                "current account wallet address is unavailable; call me() first".into(),
            )
        })?;
        let wallet = wallet(&username, password)?;
        let address = format!("{:#x}", wallet.address());
        if !address.eq_ignore_ascii_case(&expected) {
            return Err(ApiError::new(
                400,
                json!({"status": "error", "message": "Incorrect password", "errorCode": 0}),
            )
            .into());
        }
        Ok(wallet)
    }

    pub(super) async fn sign_named(
        &self,
        wallet: &LocalWallet,
        primary_type: &str,
        fields: Value,
        message: Value,
    ) -> Result<String> {
        let mut types = Map::new();
        types.insert(primary_type.to_owned(), fields);
        let typed: TypedData = serde_json::from_value(json!({
            "types": types,
            "primaryType": primary_type,
            "domain": self.eip712_domain(),
            "message": message,
        }))?;
        wallet
            .sign_typed_data(&typed)
            .await
            .map(|signature| format!("0x{signature}"))
            .map_err(|error| Error::Protocol(format!("failed to sign EIP-712 payload: {error}")))
    }

    fn eip712_domain(&self) -> Value {
        json!({
            "name": if self.testnet { "Sogni-testnet" } else { "Sogni AI" },
            "version": "1",
            "chainId": if self.testnet { 84532 } else { 8453 },
            "verifyingContract": VERIFYING_CONTRACT,
        })
    }
}

pub(super) fn wallet(username: &str, password: &str) -> Result<LocalWallet> {
    let mut private_key = [0_u8; 32];
    let secret = Zeroizing::new(format!("{}{password}", username.to_lowercase()));
    pbkdf2_hmac::<Sha256>(
        secret.as_bytes(),
        b"sogni-salt-value",
        10_000,
        &mut private_key,
    );
    let wallet = LocalWallet::from_bytes(&private_key)
        .map_err(|error| Error::Protocol(format!("failed to derive wallet: {error}")));
    private_key.zeroize();
    wallet
}

pub(super) async fn sign_dynamic(wallet: &LocalWallet, permit: &Value) -> Result<String> {
    let types = permit
        .get("types")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Protocol("permit response has no types".into()))?;
    let primary_type = types
        .keys()
        .find(|name| name.as_str() != "EIP712Domain")
        .ok_or_else(|| Error::Protocol("permit response has no primary EIP-712 type".into()))?;
    let typed: TypedData = serde_json::from_value(json!({
        "types": permit.get("types"),
        "primaryType": primary_type,
        "domain": permit.get("domain"),
        "message": permit.get("message"),
    }))?;
    wallet
        .sign_typed_data(&typed)
        .await
        .map(|signature| format!("0x{signature}"))
        .map_err(|error| Error::Protocol(format!("failed to sign EIP-712 permit: {error}")))
}

pub(super) fn amount_to_wei(amount: &str) -> Result<String> {
    let value = amount.trim();
    let negative = value.starts_with('-');
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || !fraction.chars().all(|character| character.is_ascii_digit())
    {
        return Err(Error::InvalidInput(
            "amount must be a base-10 decimal with at most 18 decimal places".into(),
        ));
    }
    if fraction.trim_end_matches('0').len() > 18 {
        return Err(Error::InvalidInput(
            "amount has more than 18 decimal places".into(),
        ));
    }
    let whole = if whole.is_empty() {
        0
    } else {
        whole
            .parse::<u128>()
            .map_err(|_| Error::InvalidInput("amount is too large or malformed".into()))?
    };
    let mut padded = fraction.chars().take(18).collect::<String>();
    while padded.len() < 18 {
        padded.push('0');
    }
    let fraction = padded.parse::<u128>().unwrap_or(0);
    let wei = whole
        .checked_mul(10_u128.pow(18))
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| Error::InvalidInput("amount is too large".into()))?;
    Ok(if negative {
        format!("-{wei}")
    } else {
        wei.to_string()
    })
}
