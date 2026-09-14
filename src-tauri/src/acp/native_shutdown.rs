//! Independent shutdown token for an in-process agent session.
//!
//! Map cleanup stays on `ConnectionCleanupGuard`. Resource owners (terminals,
//! MCP clients) register here so disconnect can wait for them after the
//! connection leaves the manager map.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::acp::process_owner::{
    force_kill_and_reap, kill_tree_signal, lock_owners, ProcessOwnerRegistry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCleanupState {
    Pending,
    Complete,
    Failed,
}

pub struct NativeShutdownHandle {
    shutdown: CancellationToken,
    state_tx: watch::Sender<NativeCleanupState>,
    state_rx: watch::Receiver<NativeCleanupState>,
    owners: Arc<Mutex<ProcessOwnerRegistry>>,
}

impl NativeShutdownHandle {
    pub fn new() -> Arc<Self> {
        let (state_tx, state_rx) = watch::channel(NativeCleanupState::Pending);
        Arc::new(Self {
            shutdown: CancellationToken::new(),
            state_tx,
            state_rx,
            owners: Arc::new(Mutex::new(ProcessOwnerRegistry::new())),
        })
    }

    pub fn signal_shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.shutdown.is_cancelled()
    }

    pub fn token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub async fn cancelled(&self) {
        self.shutdown.cancelled().await;
    }

    pub fn subscribe_cleanup(&self) -> watch::Receiver<NativeCleanupState> {
        self.state_rx.clone()
    }

    pub fn cleanup_state(&self) -> NativeCleanupState {
        *self.state_rx.borrow()
    }

    pub fn mark_complete(&self) {
        let _ = self.state_tx.send(NativeCleanupState::Complete);
    }

    pub fn mark_failed(&self) {
        let _ = self.state_tx.send(NativeCleanupState::Failed);
    }

    pub fn owners(&self) -> Arc<Mutex<ProcessOwnerRegistry>> {
        Arc::clone(&self.owners)
    }

    /// Immediate SIGKILL of every registered pid. Used from `Drop` so a panicking
    /// worker still reaps SIGTERM-immune children without awaiting.
    pub fn kill_owners_now(&self) -> bool {
        let pids = lock_owners(&self.owners).take_pids();
        let had_pids = !pids.is_empty();
        for pid in pids {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            kill_tree_signal(pid, "SIGKILL");
        }
        had_pids
    }

    /// SIGTERM-immune children still die on the force-kill path. Leftover live
    /// pids are returned so the caller can report `cleanup_failed`.
    pub async fn force_kill_owners(&self, timeout: Duration) -> Vec<u32> {
        let pids = lock_owners(&self.owners).pids();
        let leftover = force_kill_and_reap(&pids, timeout).await;
        {
            let mut owners = lock_owners(&self.owners);
            for pid in &pids {
                if !leftover.contains(pid) {
                    owners.unregister(*pid);
                }
            }
        }
        leftover
    }
}
