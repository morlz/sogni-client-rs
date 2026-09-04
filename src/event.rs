use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    pub data: Value,
}

pub type EventReceiver = broadcast::Receiver<Event>;

#[derive(Clone, Debug)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1_024)
    }
}

impl EventBus {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub(crate) fn emit(&self, name: impl Into<String>, data: Value) {
        let _ = self.sender.send(Event {
            name: name.into(),
            data,
        });
    }

    pub(crate) fn subscribe(&self) -> EventReceiver {
        self.sender.subscribe()
    }
}
