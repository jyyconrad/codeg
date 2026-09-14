//! Per-turn cancellation token and unique TurnComplete.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::agent::hook::PermissionDecision;

struct TurnInner {
    id: u64,
    token: CancellationToken,
    completed: bool,
    pending: HashMap<String, oneshot::Sender<PermissionDecision>>,
}

/// Single owner of turn identity, cancel token, and permission resolve-once.
pub struct TurnCoordinator {
    inner: Mutex<TurnInner>,
}

impl TurnCoordinator {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(TurnInner {
                id: 0,
                token: CancellationToken::new(),
                completed: true,
                pending: HashMap::new(),
            }),
        }
    }

    /// Start a new turn with a fresh cancellation token. Previous pending
    /// permission senders are dropped (cancelled).
    pub fn begin(&self) -> (u64, CancellationToken) {
        let mut inner = self.inner.lock().expect("turn coordinator");
        inner.id = inner.id.saturating_add(1);
        inner.token = CancellationToken::new();
        inner.completed = false;
        inner.pending.clear();
        (inner.id, inner.token.clone())
    }

    pub fn current_id(&self) -> u64 {
        self.inner.lock().expect("turn coordinator").id
    }

    pub fn is_current(&self, turn_id: u64) -> bool {
        let inner = self.inner.lock().expect("turn coordinator");
        inner.id == turn_id && !inner.completed
    }

    pub fn token(&self) -> CancellationToken {
        self.inner.lock().expect("turn coordinator").token.clone()
    }

    pub fn cancel_current(&self) {
        let inner = self.inner.lock().expect("turn coordinator");
        inner.token.cancel();
    }

    pub fn register_permission(
        &self,
        request_id: String,
        reply: oneshot::Sender<PermissionDecision>,
    ) {
        let mut inner = self.inner.lock().expect("turn coordinator");
        if inner.completed {
            let _ = reply.send(PermissionDecision::Cancel {
                reason: "turn already complete".into(),
            });
            return;
        }
        inner.pending.insert(request_id, reply);
    }

    /// Resolve one pending permission. Unknown / late ids are ignored.
    pub fn resolve(&self, request_id: &str, option_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("turn coordinator");
        let Some(reply) = inner.pending.remove(request_id) else {
            return false;
        };
        let decision = match option_id {
            "allow-once" | "allow_once" | "allow" => PermissionDecision::Allow,
            "reject-once" | "reject_once" | "reject" => PermissionDecision::Reject {
                reason: "user rejected".into(),
            },
            other => PermissionDecision::Reject {
                reason: format!("unsupported permission option `{other}`"),
            },
        };
        reply.send(decision).is_ok()
    }

    /// Cancel every still-pending permission. Returns the request ids so the
    /// supervisor can emit one `PermissionResolved` each.
    pub fn drain_cancelled(&self) -> Vec<String> {
        let mut inner = self.inner.lock().expect("turn coordinator");
        let pending = std::mem::take(&mut inner.pending);
        let mut ids = Vec::with_capacity(pending.len());
        for (id, reply) in pending {
            let _ = reply.send(PermissionDecision::Cancel {
                reason: "cancelled".into(),
            });
            ids.push(id);
        }
        ids
    }

    /// Unique TurnComplete gate. Returns true only for the first finisher of
    /// this `turn_id`.
    pub fn try_finish(&self, turn_id: u64) -> bool {
        let mut inner = self.inner.lock().expect("turn coordinator");
        if inner.completed || inner.id != turn_id {
            return false;
        }
        inner.completed = true;
        inner.pending.clear();
        true
    }
}

impl Default for TurnCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_issues_a_fresh_token_on_the_next_begin() {
        let turn = TurnCoordinator::new();
        let (id1, token1) = turn.begin();
        turn.cancel_current();
        assert!(token1.is_cancelled());
        assert!(turn.try_finish(id1));
        let (id2, token2) = turn.begin();
        assert_ne!(id1, id2);
        assert!(!token2.is_cancelled());
        assert!(!std::ptr::eq(&token1, &token2));
    }

    #[test]
    fn turn_complete_is_unique() {
        let turn = TurnCoordinator::new();
        let (id, _) = turn.begin();
        assert!(turn.try_finish(id));
        assert!(!turn.try_finish(id));
        assert!(!turn.is_current(id));
    }
}
