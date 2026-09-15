mod session;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use parking_lot::{Mutex as ParkingMutex, RwLock};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use url::Url;

use self::session::socket_manager;
use super::{HttpClients, RestClient};
use crate::{
    Attribution, ClientConfig, Error, Network, Result, auth::AuthManager, event::EventBus,
    utils::b64_json_encode,
};

const PROTOCOL_VERSION: &str = "3.0.0";
const SWITCH_CONNECTION: u16 = 4015;
const RECONNECT_BASE_DELAY: f64 = 1.0;
const RECONNECT_MAX_DELAY: f64 = 15.0;

enum SocketCommand {
    Send {
        message: Message,
        response: oneshot::Sender<Result<()>>,
        session: u64,
    },
}

#[derive(Clone)]
pub(super) struct SocketTransport {
    inner: Arc<SocketInner>,
}

struct SocketInner {
    url: Url,
    auth: AuthManager,
    app_id: String,
    app_source: Option<String>,
    attribution: Attribution,
    network: RwLock<Network>,
    subscriptions: RwLock<BTreeMap<String, bool>>,
    rest: RestClient,
    events: EventBus,
    commands: mpsc::Sender<SocketCommand>,
    command_receiver: Mutex<Option<mpsc::Receiver<SocketCommand>>>,
    connected: watch::Sender<bool>,
    authenticated: AtomicBool,
    session: AtomicU64,
    task: ParkingMutex<Option<tokio::task::JoinHandle<()>>>,
    cancel: CancellationToken,
    closed: AtomicBool,
    connect_timeout: Duration,
    proxy: Option<super::proxy::SocksProxy>,
}

impl SocketTransport {
    pub(super) fn new(
        config: &ClientConfig,
        auth: AuthManager,
        http: HttpClients,
        events: EventBus,
    ) -> Result<Self> {
        let mut socket_rest_url = config.socket_endpoint.clone();
        socket_rest_url
            .set_scheme(if matches!(socket_rest_url.scheme(), "ws" | "http") {
                "http"
            } else {
                "https"
            })
            .map_err(|()| Error::InvalidInput("invalid socket endpoint scheme".into()))?;
        socket_rest_url.set_query(None);
        socket_rest_url.set_fragment(None);
        let rest = RestClient::new(socket_rest_url, auth.clone(), http, config.request_timeout);
        let (commands, command_receiver) = mpsc::channel(256);
        let (connected, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(SocketInner {
                url: config.socket_endpoint.clone(),
                auth,
                app_id: config.app_id.clone(),
                app_source: config
                    .app_source
                    .as_ref()
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty()),
                attribution: config.attribution.clone(),
                network: RwLock::new(config.network),
                subscriptions: RwLock::new(config.socket_event_subscriptions.clone()),
                rest,
                events,
                commands,
                command_receiver: Mutex::new(Some(command_receiver)),
                connected,
                authenticated: AtomicBool::new(false),
                session: AtomicU64::new(0),
                task: ParkingMutex::new(None),
                cancel: CancellationToken::new(),
                closed: AtomicBool::new(false),
                connect_timeout: config.connect_timeout,
                proxy: config
                    .proxy_url
                    .as_deref()
                    .map(super::proxy::SocksProxy::parse)
                    .transpose()?,
            }),
        })
    }

    pub(super) fn is_connected(&self) -> bool {
        *self.inner.connected.borrow()
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.is_connected()
            && self.inner.auth.is_authenticated()
            && self.inner.session.load(Ordering::Acquire) == self.inner.auth.version().session
            && self.inner.authenticated.load(Ordering::Acquire)
    }

    pub(super) fn network(&self) -> Network {
        *self.inner.network.read()
    }

    pub(super) fn cancel(&self) {
        self.inner.cancel.cancel();
    }

    pub(super) fn abort(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.cancel.cancel();
        if let Some(task) = self.inner.task.lock().take() {
            task.abort();
        }
        self.inner.connected.send_replace(false);
        self.inner.authenticated.store(false, Ordering::Release);
        self.inner.events.emit(
            "disconnected",
            json!({"code":1000,"reason":"Client disconnected"}),
        );
    }

    pub(super) async fn start(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let needs_start = self
            .inner
            .task
            .lock()
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished);
        if !needs_start {
            return Ok(());
        }
        let receiver = self
            .inner
            .command_receiver
            .lock()
            .await
            .take()
            .ok_or(Error::Closed)?;
        let inner = self.inner.clone();
        let task = tokio::spawn(async move {
            socket_manager(inner, receiver).await;
        });
        *self.inner.task.lock() = Some(task);
        Ok(())
    }

    pub(super) async fn send(&self, message_type: &str, data: &Value) -> Result<()> {
        self.start().await?;
        if !self.inner.auth.is_authenticated() {
            return Err(Error::InvalidInput(
                "authentication is required for WebSocket operations".into(),
            ));
        }
        let envelope = json!({"type": message_type, "data": b64_json_encode(data)?});
        let text = serde_json::to_string(&envelope)?;
        let (response, waiter) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + self.inner.connect_timeout;
        tokio::time::timeout_at(
            deadline,
            self.inner.commands.send(SocketCommand::Send {
                message: Message::Text(text.into()),
                response,
                session: self.inner.auth.version().session,
            }),
        )
        .await
        .map_err(|_| Error::Timeout("waiting for WebSocket connection".into()))?
        .map_err(|_| Error::Closed)?;
        tokio::time::timeout_at(deadline, waiter)
            .await
            .map_err(|_| Error::Timeout("waiting for WebSocket connection".into()))?
            .map_err(|_| Error::Closed)??;
        Ok(())
    }

    pub(super) async fn get(&self, path: &str, query: Option<&Value>) -> Result<Value> {
        self.inner.rest.get(path, query).await
    }

    pub(super) async fn switch_network(&self, network: Network) -> Result<Network> {
        let mut events = self.inner.events.subscribe();
        self.send("changeNetwork", &json!(network.as_str())).await?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = events.recv().await.map_err(|_| Error::Closed)?;
                if event.name == "changeNetwork" {
                    let value = event
                        .data
                        .get("network")
                        .and_then(Value::as_str)
                        .or_else(|| event.data.as_str());
                    match value {
                        Some("fast") => break Ok::<Network, Error>(Network::Fast),
                        Some("relaxed") => break Ok::<Network, Error>(Network::Relaxed),
                        _ => {}
                    }
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout("waiting for network change".into()))??;
        *self.inner.network.write() = result;
        Ok(result)
    }

    pub(super) async fn set_subscriptions(
        &self,
        subscriptions: BTreeMap<String, bool>,
    ) -> Result<()> {
        let payload = json!({"subscriptions": subscriptions});
        self.send("setSocketEventSubscriptions", &payload).await
    }

    pub(super) async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner.cancel.cancel();
        let task = self.inner.task.lock().take();
        if let Some(task) = task {
            let _ = task.await;
        }
        self.inner.connected.send_replace(false);
        self.inner.authenticated.store(false, Ordering::Release);
        Ok(())
    }
}
