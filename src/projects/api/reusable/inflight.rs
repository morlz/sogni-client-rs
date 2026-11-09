use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Weak},
};

use tokio::sync::Mutex;

use super::SavedUpload;

type SessionSlots<K, V> = HashMap<(u64, K), Weak<Mutex<V>>>;

pub(super) struct ScopedSlots<K, V>(parking_lot::Mutex<SessionSlots<K, V>>);

impl<K, V> Default for ScopedSlots<K, V> {
    fn default() -> Self {
        Self(parking_lot::Mutex::new(HashMap::new()))
    }
}

impl<K: Eq + Hash, V: Default> ScopedSlots<K, V> {
    pub(super) fn get(&self, session: u64, key: K) -> Arc<Mutex<V>> {
        let mut slots = self.0.lock();
        slots.retain(|_, slot| slot.strong_count() != 0);
        let slot = slots.entry((session, key)).or_default();
        if let Some(active) = slot.upgrade() {
            return active;
        }
        let active = Arc::new(Mutex::new(V::default()));
        *slot = Arc::downgrade(&active);
        active
    }
}

pub(super) struct SavedInUse {
    pub(super) asset: SavedUpload,
    _slot: Arc<Mutex<Option<SavedUpload>>>,
}

impl SavedInUse {
    pub(super) fn new(asset: SavedUpload, slot: Arc<Mutex<Option<SavedUpload>>>) -> Self {
        Self { asset, _slot: slot }
    }
}
