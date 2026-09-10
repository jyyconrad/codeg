use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::app_error::AppCommandError;
use crate::web::event_bridge::{emit_event, EventEmitter};

pub const TOOLBOX_PROGRESS_EVENT: &str = "toolbox://progress";

static JOBS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register(job_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = jobs().lock() {
        map.insert(job_id.to_string(), Arc::clone(&flag));
    }
    flag
}

pub fn cancel(job_id: &str) -> bool {
    jobs()
        .lock()
        .ok()
        .and_then(|map| map.get(job_id).cloned())
        .map(|flag| {
            flag.store(true, Ordering::SeqCst);
            true
        })
        .unwrap_or(false)
}

pub fn finish(job_id: &str) {
    if let Ok(mut map) = jobs().lock() {
        map.remove(job_id);
    }
}

pub fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

pub fn cancelled_error() -> AppCommandError {
    AppCommandError::invalid_input("Cancelled")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolboxProgress {
    pub job_id: String,
    pub kind: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

pub fn emit_progress(
    emitter: &EventEmitter,
    job_id: &str,
    kind: &str,
    bytes_done: u64,
    bytes_total: u64,
) {
    emit_event(
        emitter,
        TOOLBOX_PROGRESS_EVENT,
        ToolboxProgress {
            job_id: job_id.to_string(),
            kind: kind.to_string(),
            bytes_done,
            bytes_total,
        },
    );
}
