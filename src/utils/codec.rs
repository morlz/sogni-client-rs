use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde_json::Value;

use crate::{Error, Result};

#[must_use]
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub(crate) fn path_segment(value: &str) -> String {
    utf8_percent_encode(value, PATH_SEGMENT).to_string()
}

pub(crate) fn b64_json_encode(data: &Value) -> Result<String> {
    Ok(STANDARD.encode(serde_json::to_vec(data)?))
}

pub(crate) fn b64_json_decode(data: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&STANDARD.decode(data).map_err(
        |error| Error::Protocol(format!("invalid base64 WebSocket payload: {error}")),
    )?)?)
}
