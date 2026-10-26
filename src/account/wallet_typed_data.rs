//! EIP-712 structured encoding using ethabi's public Solidity value encoder.
//! https://eips.ethereum.org/EIPS/eip-712

use std::collections::{BTreeMap, BTreeSet};

use ethabi::{
    ParamType, Token,
    token::{LenientTokenizer, Tokenizer},
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha3::{Digest, Keccak256};

#[derive(Clone, Deserialize)]
struct Field {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

type Types = BTreeMap<String, Vec<Field>>;

#[derive(Deserialize)]
pub(super) struct TypedData {
    types: Types,
    #[serde(rename = "primaryType")]
    primary_type: String,
    domain: Map<String, Value>,
    message: Map<String, Value>,
}

pub(super) fn keccak256(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

impl TypedData {
    pub(super) fn eip712_signing_hash(&self) -> Result<[u8; 32], String> {
        // EIP-712 domains use canonical field order and omit absent fields.
        let domain_fields = [
            ("name", "string"),
            ("version", "string"),
            ("chainId", "uint256"),
            ("verifyingContract", "address"),
            ("salt", "bytes32"),
        ]
        .into_iter()
        .filter(|(name, _)| self.domain.get(*name).is_some_and(|v| !v.is_null()))
        .map(|(name, kind)| Field {
            name: name.into(),
            kind: kind.into(),
        })
        .collect();
        let mut domain_types = Types::new();
        domain_types.insert("EIP712Domain".into(), domain_fields);
        let domain = hash_struct("EIP712Domain", &self.domain, &domain_types, 0)?;
        let mut preimage = vec![0x19, 0x01];
        preimage.extend_from_slice(&domain);
        if self.primary_type != "EIP712Domain" {
            preimage.extend_from_slice(&hash_struct(
                &self.primary_type,
                &self.message,
                &self.types,
                0,
            )?);
        }
        Ok(keccak256(&preimage))
    }
}

fn encode_type(name: &str, types: &Types) -> Result<String, String> {
    let mut pending = vec![name];
    let mut dependencies = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !dependencies.insert(current) {
            continue;
        }
        let fields = types
            .get(current)
            .ok_or_else(|| format!("undefined EIP-712 type {current}"))?;
        for field in fields {
            let element = field.kind.split('[').next().unwrap_or("");
            if types.contains_key(element) {
                pending.push(element);
            } else {
                primitive_type(element)?;
            }
            // Validate metadata even for empty arrays, whose elements are not
            // visited while encoding the message value.
            for dimension in field.kind.split('[').skip(1) {
                let dimension = dimension
                    .strip_suffix(']')
                    .ok_or("invalid EIP-712 array type")?;
                if !dimension.is_empty()
                    && dimension.parse::<usize>().ok().filter(|n| *n > 0).is_none()
                {
                    return Err("invalid EIP-712 array size".into());
                }
            }
        }
    }
    dependencies.remove(name);
    let mut encoded = String::new();
    for current in std::iter::once(name).chain(dependencies) {
        encoded.push_str(current);
        encoded.push('(');
        for (index, field) in types[current].iter().enumerate() {
            if index != 0 {
                encoded.push(',');
            }
            encoded.push_str(&field.kind);
            encoded.push(' ');
            encoded.push_str(&field.name);
        }
        encoded.push(')');
    }
    Ok(encoded)
}

fn hash_struct(
    name: &str,
    value: &Map<String, Value>,
    types: &Types,
    depth: usize,
) -> Result<[u8; 32], String> {
    if depth > 128 {
        return Err("EIP-712 value nesting is too deep".into());
    }
    let mut encoded = keccak256(encode_type(name, types)?.as_bytes()).to_vec();
    for field in &types[name] {
        let value = value.get(&field.name).unwrap_or(&Value::Null);
        encoded.extend_from_slice(&encode_word(&field.kind, value, types, depth + 1)?);
    }
    Ok(keccak256(&encoded))
}

fn encode_word(kind: &str, value: &Value, types: &Types, depth: usize) -> Result<[u8; 32], String> {
    if depth > 128 {
        return Err("EIP-712 value nesting is too deep".into());
    }
    if let Some((element, suffix)) = kind.rsplit_once('[') {
        let length = suffix
            .strip_suffix(']')
            .ok_or("invalid EIP-712 array type")?;
        let values = value.as_array().ok_or("EIP-712 array value is required")?;
        if !length.is_empty() && length.parse::<usize>().ok() != Some(values.len()) {
            return Err("EIP-712 fixed array length does not match its type".into());
        }
        let mut encoded = Vec::new();
        for value in values {
            encoded.extend_from_slice(&encode_word(element, value, types, depth + 1)?);
        }
        return Ok(keccak256(&encoded));
    }
    if types.contains_key(kind) {
        if value.is_null() {
            return Ok([0; 32]);
        }
        return hash_struct(
            kind,
            value
                .as_object()
                .ok_or("EIP-712 struct value is required")?,
            types,
            depth + 1,
        );
    }
    if kind == "string" {
        return Ok(keccak256(
            value
                .as_str()
                .ok_or("EIP-712 string value is required")?
                .as_bytes(),
        ));
    }
    let param = primitive_type(kind)?;
    if matches!(
        param,
        ParamType::Array(_) | ParamType::FixedArray(_, _) | ParamType::Tuple(_)
    ) {
        return Err("unsupported EIP-712 field type".into());
    }
    let input = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned);
    let token = match &param {
        ParamType::Uint(bits) | ParamType::Int(bits) => {
            integer_token(&input, *bits, matches!(param, ParamType::Int(_)))?
        }
        ParamType::Bool => Token::Bool(value.as_bool().ok_or("EIP-712 bool value is required")?),
        ParamType::Bytes | ParamType::FixedBytes(_) if value.is_array() => {
            let bytes = value
                .as_array()
                .ok_or("EIP-712 bytes are required")?
                .iter()
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or("invalid EIP-712 byte")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let ParamType::FixedBytes(length) = param {
                if bytes.len() != length {
                    return Err("EIP-712 fixed bytes length does not match its type".into());
                }
                Token::FixedBytes(bytes)
            } else {
                Token::Bytes(bytes)
            }
        }
        ParamType::Address | ParamType::Bytes | ParamType::FixedBytes(_) => {
            let input = input.strip_prefix("0x").unwrap_or(&input);
            LenientTokenizer::tokenize(&param, input).map_err(|error| error.to_string())?
        }
        _ => return Err("unsupported EIP-712 field type".into()),
    };
    if let Token::Bytes(bytes) = &token {
        return Ok(keccak256(bytes));
    }
    let encoded = ethabi::encode(&[token]);
    encoded
        .try_into()
        .map_err(|_| "EIP-712 primitive must encode to one word".into())
}

fn primitive_type(name: &str) -> Result<ParamType, String> {
    // ABI's legacy unknown-name-as-enum fallback is not an EIP-712 type.
    match name {
        "address" => return Ok(ParamType::Address),
        "bool" => return Ok(ParamType::Bool),
        "string" => return Ok(ParamType::String),
        "bytes" => return Ok(ParamType::Bytes),
        _ => {}
    }
    for (prefix, signed) in [("uint", false), ("int", true)] {
        if let Some(width) = name.strip_prefix(prefix) {
            let bits = width
                .parse::<usize>()
                .map_err(|_| "invalid EIP-712 integer type")?;
            if !(8..=256).contains(&bits) || !bits.is_multiple_of(8) {
                return Err("invalid EIP-712 integer type".into());
            }
            return Ok(if signed {
                ParamType::Int(bits)
            } else {
                ParamType::Uint(bits)
            });
        }
    }
    if let Some(width) = name.strip_prefix("bytes") {
        let bytes = width
            .parse::<usize>()
            .map_err(|_| "invalid EIP-712 bytes type")?;
        if (1..=32).contains(&bytes) {
            return Ok(ParamType::FixedBytes(bytes));
        }
    }
    Err("undefined EIP-712 field type".into())
}

fn integer_token(input: &str, bits: usize, signed: bool) -> Result<Token, String> {
    if bits == 0 || bits > 256 || !bits.is_multiple_of(8) {
        return Err("invalid EIP-712 integer type".into());
    }
    let negative = input.starts_with('-');
    let digits = input.strip_prefix('-').unwrap_or(input);
    let value = if let Some(hex) = digits.strip_prefix("0x") {
        ethabi::Uint::from_str_radix(hex, 16).map_err(|error| error.to_string())?
    } else {
        ethabi::Uint::from_dec_str(digits).map_err(|error| error.to_string())?
    };
    if !signed {
        if negative || value.bits() > bits {
            return Err("EIP-712 unsigned integer is out of range".into());
        }
        return Ok(Token::Uint(value));
    }
    let boundary = ethabi::Uint::one() << (bits - 1);
    if negative && value > boundary || !negative && value >= boundary {
        return Err("EIP-712 signed integer is out of range".into());
    }
    Ok(Token::Int(if negative {
        (!value).overflowing_add(ethabi::Uint::one()).0
    } else {
        value
    }))
}
