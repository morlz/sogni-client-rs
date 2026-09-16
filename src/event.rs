use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

pub type EventReceiver = broadcast::Receiver<Event>;

/// Private transport provenance; it is not part of public events or wire data.
#[derive(Clone, Debug)]
pub(crate) struct ScopedEvent {
    pub(crate) event: Event,
    pub(crate) session: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<Event>,
    scoped_sender: broadcast::Sender<ScopedEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1_024)
    }
}

impl EventBus {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        let (scoped_sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            scoped_sender,
        }
    }

    pub(crate) fn emit(&self, name: impl Into<String>, data: Value) {
        self.emit_with_session(name, data, None);
    }

    pub(crate) fn emit_scoped(&self, name: impl Into<String>, data: Value, session: u64) {
        self.emit_with_session(name, data, Some(session));
    }

    fn emit_with_session(&self, name: impl Into<String>, data: Value, session: Option<u64>) {
        let event = Event {
            name: name.into(),
            data,
        };
        let _ = self.sender.send(event.clone());
        let _ = self.scoped_sender.send(ScopedEvent { event, session });
    }

    pub(crate) fn subscribe(&self) -> EventReceiver {
        self.sender.subscribe()
    }

    pub(crate) fn subscribe_scoped(&self) -> broadcast::Receiver<ScopedEvent> {
        self.scoped_sender.subscribe()
    }
}
