use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::{Value, json};

use crate::{EventReceiver, event::EventBus};

#[derive(Clone, Debug)]
pub struct CurrentAccount {
    inner: Arc<RwLock<Value>>,
    events: EventBus,
}

impl Default for CurrentAccount {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(account_defaults())),
            events: EventBus::default(),
        }
    }
}

impl CurrentAccount {
    #[must_use]
    pub fn snapshot(&self) -> Value {
        self.inner.read().clone()
    }

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.string("walletAddress").is_some()
    }

    #[must_use]
    pub fn network_status(&self) -> String {
        self.string("networkStatus")
            .unwrap_or_else(|| "disconnected".into())
    }

    #[must_use]
    pub fn network(&self) -> Option<String> {
        self.string("network")
    }

    #[must_use]
    pub fn balance(&self) -> Value {
        self.inner
            .read()
            .get("balance")
            .cloned()
            .unwrap_or(Value::Null)
    }

    #[must_use]
    pub fn wallet_address(&self) -> Option<String> {
        self.string("walletAddress")
    }

    #[must_use]
    pub fn username(&self) -> Option<String> {
        self.string("username")
    }

    #[must_use]
    pub fn email(&self) -> Option<String> {
        self.string("email")
    }

    #[must_use]
    pub fn subscription(&self) -> Option<Value> {
        self.inner
            .read()
            .get("subscription")
            .cloned()
            .filter(|value| !value.is_null())
    }

    #[must_use]
    pub fn free_spark_locked(&self) -> Option<bool> {
        self.inner
            .read()
            .get("freeSparkLocked")
            .and_then(Value::as_bool)
    }

    #[must_use]
    pub fn free_spark_unlock_path(&self) -> Option<String> {
        self.inner
            .read()
            .get("freeSparkUnlockPath")
            .and_then(Value::as_str)
            .filter(|path| matches!(*path, "trial" | "purchase"))
            .map(ToOwned::to_owned)
    }

    #[must_use]
    pub fn is_unlimited(&self) -> bool {
        let guard = self.inner.read();
        let subscription = guard.get("subscription").and_then(Value::as_object);
        subscription.is_some_and(|subscription| {
            subscription.get("active").and_then(Value::as_bool) == Some(true)
                && matches!(
                    subscription.get("tier").and_then(Value::as_str),
                    Some("unlimited" | "unlimited_pro")
                )
        })
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.events.subscribe()
    }

    fn string(&self, key: &str) -> Option<String> {
        self.inner
            .read()
            .get(key)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    pub(super) fn update(&self, delta: Value) {
        let Some(delta) = delta.as_object() else {
            return;
        };
        let mut guard = self.inner.write();
        let target = guard.as_object_mut().expect("account state is an object");
        let keys = delta.keys().cloned().collect::<Vec<_>>();
        target.extend(delta.clone());
        self.events.emit("updated", json!(keys));
    }

    pub(super) fn clear(&self) {
        let defaults = account_defaults();
        let mut current = self.inner.write();
        if *current == defaults {
            return;
        }
        *current = defaults;
        drop(current);
        self.events.emit("updated", json!(["*"]));
    }

    #[cfg(test)]
    pub(super) fn state_retention_probe(&self) -> std::sync::Weak<RwLock<Value>> {
        Arc::downgrade(&self.inner)
    }
}

fn account_defaults() -> Value {
    json!({
        "networkStatus": "disconnected",
        "network": Value::Null,
        "balance": {
            "sogni": {"credit": "0", "debit": "0", "net": "0", "settled": "0"},
            "spark": {"credit": "0", "debit": "0", "net": "0", "settled": "0", "premiumCredit": "0"},
        },
        "walletAddress": Value::Null,
        "username": Value::Null,
        "email": Value::Null,
        "subscription": Value::Null,
        "freeSparkLocked": Value::Null,
        "freeSparkUnlockPath": Value::Null,
    })
}
