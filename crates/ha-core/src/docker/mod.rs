mod deploy;
mod helpers;
mod lifecycle;
mod proxy;
mod status;

pub use deploy::*;
pub use lifecycle::*;
pub use status::*;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const CONTAINER_NAME: &str = "hope-agent-searxng";
pub(crate) const IMAGE: &str = "searxng/searxng";
pub(crate) const DEFAULT_HOST_PORT: u16 = 8080;
const SEARXNG_DIR_NAME: &str = "searxng";

/// EventBus channel name for deploy progress frames. Subscribed by both
/// the Tauri webview and the HTTP `/ws/events` forwarder.
pub const EVENT_SEARXNG_DEPLOY_PROGRESS: &str = "searxng:deploy_progress";

/// Prevent concurrent deploy/start/stop/remove operations.
pub(crate) static DEPLOYING: AtomicBool = AtomicBool::new(false);

/// Shared deploy progress snapshot: (current_step, log_lines). Read by
/// `status()` so a late-joining UI gets a snapshot without replaying
/// EventBus history.
pub(crate) static DEPLOY_PROGRESS: std::sync::LazyLock<std::sync::Mutex<DeployProgressSnapshot>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(DeployProgressSnapshot::default()));

#[derive(Default, Clone)]
pub(crate) struct DeployProgressSnapshot {
    pub step: Option<String>,
    pub logs: Vec<String>,
}

/// One frame of deploy progress emitted via `on_progress` callback.
/// Untagged so the wire form stays `{"step": "..."}` or `{"log": "..."}`,
/// matching what the frontend `SearxngDocker` panel parses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeployProgress {
    Step { step: String },
    Log { log: String },
}

/// Prevent concurrent status() calls; cache recent result to avoid redundant search tests.
pub(crate) static STATUS_LOCK: std::sync::LazyLock<
    tokio::sync::Mutex<Option<(std::time::Instant, SearxngDockerStatus)>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));
/// Status cache TTL — skip search_test if last result is fresh enough.
pub(crate) const STATUS_CACHE_TTL_SECS: u64 = 5;

const LOG_CAT: &str = "docker";
const LOG_SRC: &str = "SearXNG";

/// Write to AppLogger (SQLite + file). Falls back to log::info! if logger unavailable.
pub(crate) fn app_log(level: &str, message: &str, details: Option<String>) {
    if let Some(logger) = crate::get_logger() {
        logger.log(level, LOG_CAT, LOG_SRC, message, details, None, None);
    }
}

pub(crate) fn get_deploy_progress() -> (bool, Option<String>, Vec<String>) {
    let deploying = DEPLOYING.load(Ordering::SeqCst);
    if !deploying {
        return (false, None, vec![]);
    }
    let guard = DEPLOY_PROGRESS.lock().unwrap_or_else(|e| {
        app_warn!(
            "docker",
            "deploy",
            "DEPLOY_PROGRESS lock poisoned, recovering"
        );
        e.into_inner()
    });
    (true, guard.step.clone(), guard.logs.clone())
}

pub(crate) fn info(msg: &str) {
    app_log("info", msg, None);
}

pub(crate) fn error(msg: &str, details: &str) {
    app_log("error", msg, Some(details.to_string()));
}
