use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

static CANCELS: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn register(job_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    map.insert(job_id.to_string(), flag.clone());
    flag
}

pub(crate) fn cancel(job_id: &str) -> bool {
    let map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(flag) = map.get(job_id) {
        flag.store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub(crate) fn remove(job_id: &str) {
    let mut map = CANCELS.lock().unwrap_or_else(|p| p.into_inner());
    map.remove(job_id);
}
