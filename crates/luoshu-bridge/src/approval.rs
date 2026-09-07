//! One-time approval tokens for dangerous shell commands.
//!
//! Flow (v1, see docs/003-mvp-architecture.md):
//! 1. Agent calls `luoshu_shell` with a dangerous command and no token.
//! 2. Bridge creates a pending approval (command + 6-digit one-time token,
//!    5-minute TTL) and notifies the shell UI, which shows a dialog with the
//!    command and the token.
//! 3. The tool call fails with "approval required". The *human* reads the
//!    token from the dialog and relays it to the agent.
//! 4. Agent retries with `approval_token`; bridge validates (token must match
//!    the exact command string), consumes it, and executes.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::Rng;

const APPROVAL_TTL: Duration = Duration::from_secs(300);

struct PendingApproval {
    command: String,
    created_at: Instant,
}

pub struct ApprovalStore {
    pending: Mutex<HashMap<String, PendingApproval>>,
}

/// Events the bridge emits into the host app (Tauri shell).
pub trait BridgeEventSink: Send + Sync {
    fn on_approval_requested(&self, command: &str, token: &str);
}

/// No-op sink for tests / headless use.
pub struct NullSink;
impl BridgeEventSink for NullSink {
    fn on_approval_requested(&self, _command: &str, _token: &str) {}
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Create a pending approval; returns the 6-digit one-time token.
    pub fn create(&self, command: &str) -> String {
        let token: String = {
            let mut rng = rand::rng();
            (0..6).map(|_| rng.random_range(b'0'..=b'9') as char).collect()
        };
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|_, p| p.created_at.elapsed() < APPROVAL_TTL);
        pending.insert(
            token.clone(),
            PendingApproval {
                command: command.to_string(),
                created_at: Instant::now(),
            },
        );
        token
    }

    /// Validate + consume a token. One-time: success removes the entry.
    pub fn consume(&self, token: &str, command: &str) -> bool {
        let mut pending = self.pending.lock().unwrap();
        match pending.get(token) {
            Some(p) if p.command == command && p.created_at.elapsed() < APPROVAL_TTL => {
                pending.remove(token);
                true
            }
            Some(p) if p.created_at.elapsed() >= APPROVAL_TTL => {
                pending.remove(token);
                false
            }
            _ => false,
        }
    }

    /// Drop a pending token (user pressed "deny" in the UI).
    pub fn revoke(&self, token: &str) {
        self.pending.lock().unwrap().remove(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_one_time_and_command_bound() {
        let store = ApprovalStore::new();
        let token = store.create("rm -rf /tmp/x");
        assert_eq!(token.len(), 6);
        // Wrong command → refused, token still pending.
        assert!(!store.consume(&token, "rm -rf /tmp/y"));
        // Correct command → ok once.
        assert!(store.consume(&token, "rm -rf /tmp/x"));
        // Second use → refused (consumed).
        assert!(!store.consume(&token, "rm -rf /tmp/x"));
    }

    #[test]
    fn revoke_removes_token() {
        let store = ApprovalStore::new();
        let token = store.create("shutdown now");
        store.revoke(&token);
        assert!(!store.consume(&token, "shutdown now"));
    }
}
