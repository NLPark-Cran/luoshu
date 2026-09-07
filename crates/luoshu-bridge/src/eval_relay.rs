//! One-shot result writeback for `luoshu_browser_eval` (ADR 003 deferred:
//! "browser_eval 返回值").
//!
//! Tauri's `eval` is fire-and-forget on Linux WebKitGTK, so the bridge wraps
//! the agent's script in a harness that POSTs the completion value back to
//! the loopback bridge (`POST /eval-result`). The writeback is authorized by
//! a **one-time random nonce** generated per eval call — the bridge's own
//! bearer token is never injected into the page context (a hostile page
//! could otherwise sniff it and gain full bridge access). A leaked nonce
//! only lets the page spoof the result of its own eval, which it could
//! compute anyway.

use std::collections::HashMap;
use std::sync::Mutex;

use rand::Rng;
use serde_json::Value;
use tokio::sync::oneshot;

fn random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

pub struct EvalResultStore {
    /// eval id → (nonce, sender)
    pending: Mutex<HashMap<String, (String, oneshot::Sender<Value>)>>,
}

impl Default for EvalResultStore {
    fn default() -> Self {
        Self::new()
    }
}

impl EvalResultStore {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Register a pending eval. Returns (id, nonce, receiver).
    pub fn create(&self) -> (String, String, oneshot::Receiver<Value>) {
        let id = random_hex(8);
        let nonce = random_hex(32);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), (nonce.clone(), tx));
        (id, nonce, rx)
    }

    /// Deliver a result. The nonce must match and each id resolves at most
    /// once. Returns false on unknown id / wrong nonce / already resolved.
    pub fn resolve(&self, id: &str, nonce: &str, value: Value) -> bool {
        let entry = self.pending.lock().unwrap().remove(id);
        match entry {
            Some((expected, tx)) if expected == nonce => tx.send(value).is_ok(),
            Some((expected, tx)) => {
                // Wrong nonce: put the entry back so the legit page can still
                // deliver its result.
                self.pending
                    .lock()
                    .unwrap()
                    .insert(id.to_string(), (expected, tx));
                false
            }
            None => false,
        }
    }

    /// Drop a pending entry (caller timed out).
    pub fn cancel(&self, id: &str) {
        self.pending.lock().unwrap().remove(id);
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn resolve_happy_path() {
        let store = EvalResultStore::new();
        let (id, nonce, rx) = store.create();
        assert!(store.resolve(&id, &nonce, json!({"ok": true, "value": 42})));
        assert_eq!(rx.await.unwrap()["value"], 42);
        assert_eq!(store.pending_count(), 0);
        // One-time: a second resolve fails.
        assert!(!store.resolve(&id, &nonce, json!(1)));
    }

    #[tokio::test]
    async fn wrong_nonce_does_not_consume() {
        let store = EvalResultStore::new();
        let (id, nonce, rx) = store.create();
        assert!(!store.resolve(&id, "wrong", json!(1)));
        assert_eq!(store.pending_count(), 1);
        assert!(store.resolve(&id, &nonce, json!(2)));
        assert_eq!(rx.await.unwrap(), 2);
    }

    #[test]
    fn cancel_drops_entry() {
        let store = EvalResultStore::new();
        let (id, nonce, _rx) = store.create();
        store.cancel(&id);
        assert!(!store.resolve(&id, &nonce, json!(1)));
    }
}
