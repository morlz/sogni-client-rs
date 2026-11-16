use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use zeroize::Zeroizing;

impl MediaEncryptionApi {
    pub(super) async fn ensure_key(&self) {
        let Some(mut waiting) = self.queue_fetch() else {
            return;
        };
        if !*waiting.borrow_and_update() {
            let _ = waiting.changed().await;
        }
    }

    pub(super) fn queue_fetch(&self) -> Option<watch::Receiver<bool>> {
        let mut state = self.client.media_keys.current();
        if !state.keys.is_empty()
            || !self.automatic()
            || !state.available
            || state.turned_off
            || !self.client.is_authenticated()
            || state.retry_at.is_some_and(|at| Instant::now() < at)
        {
            return None;
        }
        if let Some(fetch) = &state.fetch {
            return Some(fetch.clone());
        }
        let session = self.client.rest.request_session();
        if session.id() != state.session {
            return None;
        }
        let access = MediaAccess {
            session,
            lifetime: state.lifetime.clone(),
        };
        let (finished, waiting) = watch::channel(false);
        state.fetch = Some(waiting.clone());
        let this = self.clone();
        tokio::spawn(async move {
            this.fetch_keys(access).await;
            finished.send_replace(true);
        });
        Some(waiting)
    }

    async fn fetch_keys(&self, access: MediaAccess) {
        let outcome = access
            .run(async {
                let mut result = self.client.rest.get("/v1/account/media-key", None).await?;
                let keys = result
                    .get_mut("data")
                    .ok_or(E::Malformed("missing account keys"))?;
                parse_keys(keys)
            })
            .await;
        let mut state = self.client.media_keys.current();
        if access.check().is_err() {
            return;
        }
        state.fetch = None;
        let success = match outcome {
            Ok(keys) => {
                state.retry_at = None;
                state.keys = keys;
                self.client.media_keys.events.emit("unlocked", Value::Null);
                true
            }
            Err(Error::Api(error)) if error.status == 409 => {
                state.turned_off = true;
                false
            }
            Err(_) => {
                state.retry_at = Some(Instant::now() + KEY_RETRY);
                false
            }
        };
        drop(state);
        if success {
            self.announce();
        }
    }
}

fn parse_keys(payload: &mut Value) -> Result<Vec<AccountKey>> {
    if payload.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(E::Malformed("unsupported account key response").into());
    }
    let current = payload
        .get_mut("current")
        .ok_or(E::Malformed("missing current account key"))?;
    let mut keys = vec![parse_key(current)?];
    if let Some(previous) = payload.get_mut("previous").and_then(Value::as_array_mut) {
        for previous in previous {
            keys.push(parse_key(previous)?);
        }
    }
    Ok(keys)
}

fn parse_key(record: &mut Value) -> Result<AccountKey> {
    let Some(Value::String(encoded)) = record
        .as_object_mut()
        .and_then(|record| record.remove("key"))
    else {
        return Err(E::Malformed("missing account key").into());
    };
    let encoded = Zeroizing::new(encoded);
    let bytes = Zeroizing::new(
        STANDARD
            .decode(encoded.as_bytes())
            .map_err(|_| E::Malformed("invalid account key encoding"))?,
    );
    Ok(AccountKey::new(MediaKey::try_from(bytes.as_slice())?)?)
}
