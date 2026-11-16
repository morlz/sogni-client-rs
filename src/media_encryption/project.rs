use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use super::{
    MediaEncryptionError as E, MediaKey,
    api::MediaAccess,
    codec::{DEFAULT_SEGMENT_LOG2, VERSION, random, seal},
    recipient::{AccountKey, wrap_input},
};
use crate::{
    Result,
    utils::{b64_json_decode, b64_json_encode},
};

const MAX_INPUT_KEYS: usize = 32;

pub(crate) struct ProjectEncryption {
    access: MediaAccess,
    project: MediaKey,
    account: AccountKey,
    recipient: [u8; 69],
    input_keys: Vec<(MediaKey, [u8; 60])>,
    encrypt_inputs: bool,
}

impl ProjectEncryption {
    pub(super) fn new(
        access: MediaAccess,
        account: AccountKey,
        encrypt_inputs: bool,
    ) -> Result<Self> {
        access.check()?;
        let project = MediaKey::from_bytes(random()?);
        let recipient = account.wrap(&project, random()?)?;
        Ok(Self {
            access,
            project,
            account,
            recipient,
            input_keys: Vec::new(),
            encrypt_inputs,
        })
    }

    pub(crate) fn check(&self) -> Result<()> {
        self.access.check()
    }

    pub(crate) fn access(&self) -> MediaAccess {
        self.access.clone()
    }

    pub(crate) fn seal_input(&mut self, data: Bytes, content_type: Option<&str>) -> Result<Bytes> {
        self.check()?;
        if !self.encrypt_inputs {
            return Ok(data);
        }
        let content_type = content_type.unwrap_or_default();
        let material = self.account.input_material(&data, content_type)?;
        if !self
            .input_keys
            .iter()
            .any(|(key, _)| bool::from(key.as_bytes().ct_eq(material.key.as_bytes())))
        {
            if self.input_keys.len() == MAX_INPUT_KEYS {
                return Err(E::Unsupported(
                    "an encrypted project accepts at most 32 different input files",
                )
                .into());
            }
            let wrapped = wrap_input(&self.project, &material.key, random()?)?;
            self.input_keys.push((material.key.clone(), wrapped));
        }
        let recipient = self.account.wrap(&material.key, material.recipient_nonce)?;
        let bytes = seal(
            &data,
            &material.key,
            content_type,
            &recipient,
            DEFAULT_SEGMENT_LOG2,
            material.salt,
            material.nonce_prefix,
        )?;
        self.check()?;
        Ok(bytes)
    }

    pub(crate) fn finish(&self, wire: &mut Value) -> Result<()> {
        self.check()?;
        let mut encryption = json!({
            "version":VERSION, "key":STANDARD.encode(self.project.as_bytes()),
            "recipient":STANDARD.encode(self.recipient)
        });
        if !self.input_keys.is_empty() {
            encryption["inputKeys"] = json!(
                self.input_keys
                    .iter()
                    .map(|(_, key)| STANDARD.encode(key))
                    .collect::<Vec<_>>()
            );
        }
        wire["mediaEncryption"] = encryption;
        Ok(())
    }
}

/// Keep public events, debug snapshots and recovered records from exposing
/// request keys. Redaction is never applied to an outgoing request or replay.
pub(crate) fn redact_media_keys(value: &mut Value) {
    fn scrub(value: &mut Value, encoded: bool) -> bool {
        match value {
            Value::Object(fields) => {
                let mut changed = false;
                for (name, value) in fields {
                    if name == "mediaEncryption" && value.is_object() {
                        *value = Value::Bool(true);
                        changed = true;
                    } else if name == "clientRequestData" && !encoded {
                        if let Some(mut request) =
                            value.as_str().and_then(|text| b64_json_decode(text).ok())
                        {
                            if scrub(&mut request, true) {
                                *value =
                                    b64_json_encode(&request).map_or(Value::Null, Value::String);
                                changed = true;
                            }
                        } else {
                            changed |= scrub(value, encoded);
                        }
                    } else {
                        changed |= scrub(value, encoded);
                    }
                }
                changed
            }
            Value::Array(entries) => {
                let mut changed = false;
                for entry in entries {
                    changed |= scrub(entry, encoded);
                }
                changed
            }
            _ => false,
        }
    }
    scrub(value, false);
}
