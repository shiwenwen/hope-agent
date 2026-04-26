//! In-memory cancellation handles for async tool jobs.
//!
//! The DB is the durable source of job status; this registry is only a
//! best-effort bridge to the currently running future in this process.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

static CANCELS: LazyLock<Mutex<HashMap<String, CancellationToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register_job(job_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    register_job_token(job_id, token.clone());
    token
}

pub fn register_job_token(job_id: &str, token: CancellationToken) {
    let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    map.insert(job_id.to_string(), token);
}

pub fn cancel_job(job_id: &str) -> bool {
    let map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(token) = map.get(job_id) {
        token.cancel();
        true
    } else {
        false
    }
}

pub fn remove_job(job_id: &str) {
    let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    map.remove(job_id);
}
