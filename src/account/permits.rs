use serde_json::{Value, json};

use crate::{ApiError, Result};

const WITHDRAW_CONTRACT: &str = "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC";
const BASE_TOKEN: &str = "0xE014d2A4da6E450f21b5050120D291e63c8940FD";
const ETHERLINK_TOKEN: &str = "0x2767Fc1048cd0ae4fd952eeF09853301c9DC7173";

fn network(testnet: bool, provider: &str) -> (u64, Option<&'static str>) {
    match (testnet, provider == "etherlink") {
        (false, false) => (8453, Some(BASE_TOKEN)),
        (false, true) => (42793, Some(ETHERLINK_TOKEN)),
        (true, false) => (84532, None),
        (true, true) => (11155111, None),
    }
}

// JSON integers and ASCII decimal strings are the public permit representation.
// Parsing to u64 is sufficient for the bounded chain IDs and expiry timestamps.
fn integer(value: &Value) -> Option<u64> {
    if let Some(value) = value.as_str() {
        return (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| value.parse().ok())
            .flatten();
    }
    value.as_u64().or_else(|| {
        let number = value.as_f64()?;
        (number >= 0.0 && number.fract() == 0.0 && number < u64::MAX as f64)
            .then_some(number as u64)
    })
}

fn address(value: &Value, expected: &str) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn fields(permit: &Value, name: &str, expected: &[(&str, &str)]) -> bool {
    let Some(types) = permit["types"].as_object().filter(|types| types.len() == 1) else {
        return false;
    };
    let Some(actual) = types.get(name).and_then(Value::as_array) else {
        return false;
    };
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(field, (name, kind))| {
            field.as_object().is_some_and(|field| field.len() == 2)
                && field["name"] == *name
                && field["type"] == *kind
        })
}

fn checked(matches: bool) -> Result<()> {
    if matches {
        return Ok(());
    }
    Err(ApiError::new(
        400,
        json!({"status":"error", "errorCode":0,
            "message":"Unexpected signing request; nothing was signed"}),
    )
    .into())
}

pub(super) fn withdraw(permit: &Value, payload: &Value, testnet: bool) -> Result<()> {
    let provider = payload["provider"].as_str().unwrap_or_default();
    let (chain, _) = network(testnet, provider);
    let domain = &permit["domain"];
    let message = &permit["message"];
    checked(
        domain.as_object().is_some_and(|domain| domain.len() == 4)
            && domain["name"] == if testnet { "Sogni-testnet" } else { "Sogni AI" }
            && domain["version"] == "1"
            && integer(&domain["chainId"]) == Some(chain)
            && address(&domain["verifyingContract"], WITHDRAW_CONTRACT)
            && fields(
                permit,
                "WithdrawTokenV2",
                &[
                    ("walletAddress", "address"),
                    ("amount", "string"),
                    ("provider", "string"),
                    ("nonce", "string"),
                ],
            )
            && message.is_object()
            && message["walletAddress"] == payload["walletAddress"]
            && message["amount"] == payload["amount"]
            && message["provider"] == payload["provider"]
            && message["nonce"].is_string(),
    )
}

pub(super) fn approve(
    permit: &Value,
    wallet: &str,
    provider: &str,
    testnet: bool,
    now: u64,
) -> Result<()> {
    let (chain, token) = network(testnet, provider);
    let domain = &permit["domain"];
    let message = &permit["message"];
    checked(
        domain.is_object()
            && integer(&domain["chainId"]) == Some(chain)
            && token.is_none_or(|token| address(&domain["verifyingContract"], token))
            && fields(
                permit,
                "Permit",
                &[
                    ("owner", "address"),
                    ("spender", "address"),
                    ("value", "uint256"),
                    ("nonce", "uint256"),
                    ("deadline", "uint256"),
                ],
            )
            && message.is_object()
            && message["owner"] == wallet
            && integer(&message["deadline"])
                .is_some_and(|deadline| deadline <= now.saturating_add(30 * 86_400)),
    )
}

#[cfg(test)]
mod tests;
