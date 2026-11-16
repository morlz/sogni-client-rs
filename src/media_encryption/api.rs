use std::{
    fmt,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::{Mutex, MutexGuard};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    DecryptedMedia, DecryptedMediaReader, MediaEncryptionError as E, MediaEncryptionHeader,
    MediaKey, ProjectEncryption, codec,
    recipient::{AccountKey, unwrap},
    remote,
};
use crate::{
    Error, EventReceiver, Result,
    auth::{AuthManager, RequestSession},
    event::EventBus,
    transport::ApiClient,
};

mod key_fetch;

const KEY_WAIT: Duration = Duration::from_secs(5);
const KEY_RETRY: Duration = Duration::from_secs(300);
pub(crate) const MEDIA_CAPABILITY: &str = "ciphertext-v1";

/// Controls whether the SDK automatically obtains the account key and
/// encrypts new projects on models and networks that support it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MediaEncryptionMode {
    /// No automatic encryption or key requests. Explicit encryption can use
    /// a provided account key.
    #[default]
    Off,
    Auto,
}

/// Account key management and access to encrypted project media.
#[derive(Clone)]
pub struct MediaEncryptionApi {
    client: Arc<ApiClient>,
}

impl fmt::Debug for MediaEncryptionApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaEncryptionApi").finish_non_exhaustive()
    }
}

pub(crate) struct MediaKeyState {
    auth: AuthManager,
    mode: MediaEncryptionMode,
    declare: bool,
    state: Mutex<Keyring>,
    events: EventBus,
}

struct Keyring {
    session: u64,
    available: bool,
    lifetime: CancellationToken,
    keys: Vec<AccountKey>,
    fetch: Option<watch::Receiver<bool>>,
    retry_at: Option<Instant>,
    turned_off: bool,
}

impl Keyring {
    fn new(session: u64) -> Self {
        Self {
            session,
            available: false,
            lifetime: CancellationToken::new(),
            keys: Vec::new(),
            fetch: None,
            retry_at: None,
            turned_off: false,
        }
    }
}

impl MediaKeyState {
    pub(super) fn key_for(
        &self,
        header: &MediaEncryptionHeader,
        access: &MediaAccess,
    ) -> Result<MediaKey> {
        let state = self.current();
        access.check()?;
        if state.keys.is_empty() {
            return Err(E::Locked.into());
        }
        Ok(unwrap(&state.keys, &header.recipient)?)
    }

    pub(crate) fn new(auth: AuthManager, mode: MediaEncryptionMode, declare: bool) -> Self {
        Self {
            state: Mutex::new(Keyring::new(auth.version().session)),
            events: EventBus::default(),
            auth,
            mode,
            declare,
        }
    }

    // This also covers a request that starts before the session listener
    // has woken up. No key or availability flag crosses account sessions.
    fn current(&self) -> MutexGuard<'_, Keyring> {
        let mut state = self.state.lock();
        let session = self.auth.version().session;
        if session != state.session {
            state.lifetime.cancel();
            if state.available {
                self.events.emit("availabilityChanged", json!(false));
            }
            if !state.keys.is_empty() {
                self.events.emit("locked", Value::Null);
            }
            *state = Keyring::new(session);
        }
        state
    }

    pub(crate) fn declared(&self, session: u64) -> bool {
        if !self.declare || self.mode != MediaEncryptionMode::Auto {
            return false;
        }
        let state = self.current();
        state.session == session && !state.keys.is_empty() && self.auth.is_authenticated()
    }
}

/// Covers a key operation and keeps fetches and readers bound to the account
/// and the same key installation, even when the handle is kept by a caller.
#[derive(Clone)]
pub(crate) struct MediaAccess {
    session: RequestSession,
    lifetime: CancellationToken,
}

impl MediaAccess {
    pub(crate) fn check(&self) -> Result<()> {
        if self.lifetime.is_cancelled() || self.session.check().is_err() {
            return Err(E::Locked.into());
        }
        Ok(())
    }

    pub(crate) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        let mut session = self.session.clone();
        tokio::select! {
            biased;
            () = self.lifetime.cancelled() => Err(E::Locked.into()),
            () = session.changed() => Err(E::Locked.into()),
            value = work => { self.check()?; value },
        }
    }
}

impl MediaEncryptionApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.client.media_keys.events.subscribe()
    }

    #[must_use]
    pub fn available(&self) -> bool {
        self.client.media_keys.current().available
    }

    pub(crate) fn automatic(&self) -> bool {
        self.client.media_keys.mode == MediaEncryptionMode::Auto
    }

    #[must_use]
    pub fn is_encrypted(&self, bytes: &[u8]) -> bool {
        codec::is_encrypted_media(bytes)
    }

    pub async fn is_unlocked(&self) -> bool {
        self.ensure_key().await;
        !self.client.media_keys.current().keys.is_empty()
    }

    /// Install a key for the current account without making a network request
    /// for a key. Previous keys may be supplied to open results after rotation.
    pub fn set_account_key(&self, key: MediaKey) -> Result<()> {
        self.set_account_keys(key, Vec::new())
    }

    pub fn set_account_keys(&self, current: MediaKey, previous: Vec<MediaKey>) -> Result<()> {
        let access = self.access();
        if !self.client.is_authenticated() {
            return Err(E::Locked.into());
        }
        let keys = std::iter::once(current)
            .chain(previous)
            .map(AccountKey::new)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        {
            let mut state = self.client.media_keys.current();
            access.check()?;
            state.lifetime.cancel();
            let available = state.available;
            let mut next = Keyring::new(state.session);
            next.available = available;
            next.keys = keys;
            *state = next;
            self.client.media_keys.events.emit("unlocked", Value::Null);
        }
        self.announce();
        Ok(())
    }

    /// Drop keys and cancel pending reads. In automatic mode a later
    /// operation may obtain a new account key.
    pub fn lock(&self) {
        let mut state = self.client.media_keys.current();
        state.lifetime.cancel();
        let mut next = Keyring::new(state.session);
        next.available = state.available;
        if !state.keys.is_empty() {
            self.client.media_keys.events.emit("locked", Value::Null);
        }
        *state = next;
    }

    /// Decrypt a downloaded container, using an explicit project key or the
    /// account's current and previous keys.
    pub async fn decrypt(&self, data: &[u8], key: Option<&MediaKey>) -> Result<DecryptedMedia> {
        let access = self.access();
        let header = MediaEncryptionHeader::parse(data)?;
        let owned;
        let key = if let Some(key) = key {
            key
        } else {
            owned = access.run(self.resolve_key(&header)).await?;
            &owned
        };
        let data = codec::open(data, header, key)?;
        access.check()?;
        Ok(data)
    }

    /// Download without account credentials. Return plaintext as downloaded
    /// unless `require_encrypted` is set.
    pub async fn fetch_decrypted(
        &self,
        url: Url,
        require_encrypted: bool,
    ) -> Result<DecryptedMedia> {
        let access = self.access();
        let (data, content_type) = access.run(self.client.rest.download_media(url)).await?;
        if codec::is_encrypted_media(&data) {
            return access.run(self.decrypt(&data, None)).await;
        }
        if require_encrypted {
            return Err(E::NotEncrypted.into());
        }
        Ok(DecryptedMedia {
            data,
            content_type,
            header: None,
        })
    }

    /// Open an encrypted file at a server supporting HTTP byte ranges.
    pub async fn open_decrypted(&self, url: Url) -> Result<DecryptedMediaReader> {
        let access = self.access();
        access
            .run(async {
                self.ensure_key().await;
                Ok(())
            })
            .await?;
        remote::open(
            self.client.media_keys.clone(),
            self.client.rest.clone(),
            access,
            url,
        )
        .await
    }

    pub(super) async fn resolve_key(&self, header: &MediaEncryptionHeader) -> Result<MediaKey> {
        self.ensure_key().await;
        let state = self.client.media_keys.current();
        if state.keys.is_empty() {
            return Err(E::Locked.into());
        }
        Ok(unwrap(&state.keys, &header.recipient)?)
    }

    pub(crate) async fn can_encrypt_automatically(&self) -> bool {
        self.automatic()
            && self.available()
            && tokio::time::timeout(KEY_WAIT, self.is_unlocked())
                .await
                .unwrap_or(false)
            && self.available()
    }

    pub(crate) async fn start_project(&self, encrypt_inputs: bool) -> Result<ProjectEncryption> {
        let access = self.access();
        access
            .run(async {
                self.ensure_key().await;
                Ok(())
            })
            .await?;
        let account = {
            let state = self.client.media_keys.current();
            access.check()?;
            state.keys.first().cloned().ok_or(E::Locked)?
        };
        ProjectEncryption::new(access, account, encrypt_inputs)
    }

    fn access(&self) -> MediaAccess {
        let state = self.client.media_keys.current();
        let session = self.client.rest.request_session();
        let lifetime = if session.id() != state.session {
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            cancelled
        } else {
            state.lifetime.clone()
        };
        MediaAccess { session, lifetime }
    }

    fn announce(&self) {
        let access = self.access();
        let session = self.client.auth_session();
        if !self.client.media_keys.declared(session) || !self.client.is_socket_connected() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            if client.media_keys.declared(session) {
                let _ = access
                    .run(client.send_socket_tracked_in_session(
                        "mediaCapability",
                        &json!({"sogniMedia": MEDIA_CAPABILITY}),
                        session,
                    ))
                    .await;
            }
        });
    }

    pub(crate) fn listen(&self) {
        let mut auth = self.client.media_keys.auth.subscribe_session();
        let mut events = self.client.subscribe_scoped();
        let owner = Arc::downgrade(&self.client);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = auth.changed() => {
                        if changed.is_err() { return; }
                        let Some(client) = owner.upgrade() else { return; };
                        drop(client.media_keys.current());
                    }
                    event = events.recv() => match event {
                        Ok(event) => {
                            let Some(client) = owner.upgrade() else { return; };
                            let available = match event.event.name.as_str() {
                                "authenticated" => event.event.data.get("mediaEncryptionAvailable").and_then(Value::as_bool) == Some(true),
                                "mediaEncryptionAvailabilityUpdated" => event.event.data.as_bool() == Some(true),
                                _ => continue,
                            };
                            let mut state = client.media_keys.current();
                            if event.session != Some(state.session) { continue; }
                            if available != state.available {
                                state.available = available;
                                client.media_keys.events.emit("availabilityChanged", json!(available));
                            }
                            drop(state);
                            if available { let _ = Self { client }.queue_fetch(); }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        });
    }
}
