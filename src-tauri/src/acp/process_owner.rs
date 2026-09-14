//! OS-level owners spawned by an in-process session (shell children, MCP).
//!
//! `child_pid` on `AgentConnection` stays 0 for native sessions — this registry
//! is the shutdown backstop for those trees. A published `Exited` status is not
//! proof the OS has reaped the pid; only a wait/ESRCH check is.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct ProcessOwnerRegistry {
    pids: HashSet<u32>,
}

impl ProcessOwnerRegistry {
    pub fn new() -> Self {
        Self {
            pids: HashSet::new(),
        }
    }

    pub fn register(&mut self, pid: u32) {
        if pid != 0 {
            self.pids.insert(pid);
        }
    }

    pub fn unregister(&mut self, pid: u32) {
        self.pids.remove(&pid);
    }

    pub fn pids(&self) -> Vec<u32> {
        self.pids.iter().copied().collect()
    }

    pub fn take_pids(&mut self) -> Vec<u32> {
        self.pids.drain().collect()
    }
}

/// Recover from a poisoned mutex so panic cleanup can still read pids.
pub fn lock_owners(owners: &Mutex<ProcessOwnerRegistry>) -> MutexGuard<'_, ProcessOwnerRegistry> {
    owners
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// True while `pid` still names a live process.
pub fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
        windows_pid_is_alive(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

#[cfg(windows)]
fn windows_pid_is_alive(pid: u32) -> bool {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        fn GetExitCodeProcess(handle: *mut std::ffi::c_void, code: *mut u32) -> i32;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// Hard-kill each pid's process tree, then poll until they are gone or `timeout`
/// elapses. Returns pids that are still alive — never report success for those.
pub async fn force_kill_and_reap(pids: &[u32], timeout: Duration) -> Vec<u32> {
    let mut leftover: Vec<u32> = pids.iter().copied().filter(|pid| *pid != 0).collect();
    leftover.sort_unstable();
    leftover.dedup();
    if leftover.is_empty() {
        return leftover;
    }
    let to_kill = leftover.clone();
    let _ = tokio::task::spawn_blocking(move || {
        for pid in to_kill {
            kill_tree_signal(pid, "SIGKILL");
        }
    })
    .await;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        leftover.retain(|&pid| pid_is_alive(pid));
        if leftover.is_empty() || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    leftover.retain(|&pid| pid_is_alive(pid));
    leftover
}

pub(crate) fn kill_tree_signal(pid: u32, signal: &str) {
    if pid == 0 {
        return;
    }
    let config = kill_tree::Config {
        signal: signal.to_string(),
        ..Default::default()
    };
    if let Err(err) = kill_tree::blocking::kill_tree_with_config(pid, &config) {
        tracing::debug!("[ACP] native kill pid={pid} signal={signal}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_pid_is_never_registered() {
        let mut reg = ProcessOwnerRegistry::new();
        reg.register(0);
        assert!(reg.pids().is_empty());
    }

    #[test]
    fn unregister_and_take_clear_the_set() {
        let mut reg = ProcessOwnerRegistry::new();
        reg.register(7);
        reg.register(9);
        reg.unregister(7);
        let left = reg.take_pids();
        assert_eq!(left, vec![9]);
        assert!(reg.pids().is_empty());
    }
}
