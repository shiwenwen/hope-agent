use serde::{Deserialize, Serialize};

use super::{get_deploy_progress, helpers::*, STATUS_CACHE_TTL_SECS, STATUS_LOCK};

// ── Public Status ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearxngDockerStatus {
    pub docker_installed: bool,
    /// Docker CLI exists but daemon is not running
    pub docker_not_running: bool,
    pub container_exists: bool,
    pub container_running: bool,
    pub port: Option<u16>,
    pub health_ok: bool,
    /// A deploy operation is currently in progress
    pub deploying: bool,
    /// Current deploy step (if deploying)
    pub deploy_step: Option<String>,
    /// Deploy log lines accumulated so far (if deploying)
    pub deploy_logs: Vec<String>,
    /// Real search returned results (not just 200 OK)
    pub search_ok: bool,
    /// Number of results from the test search
    pub search_result_count: usize,
    /// Engines that failed during the test search (e.g. ["google: access denied", "brave: timeout"])
    pub unresponsive_engines: Vec<String>,
}

/// Gather full status of Docker + SearXNG container.
/// Uses a Mutex + short TTL cache to prevent concurrent calls and redundant search tests.
pub async fn status() -> SearxngDockerStatus {
    // Check cache under lock, but release lock before expensive status_inner() call
    {
        let guard = STATUS_LOCK.lock().await;
        if let Some((ts, ref cached)) = *guard {
            if ts.elapsed().as_secs() < STATUS_CACHE_TTL_SECS {
                return cached.clone();
            }
        }
    }
    // Lock released here — only one task will typically compute, others will see stale cache or wait

    let result = status_inner().await;

    // Re-acquire and update cache (double-check to avoid overwriting a fresher result)
    let mut guard = STATUS_LOCK.lock().await;
    if let Some((ts, ref cached)) = *guard {
        if ts.elapsed().as_secs() < STATUS_CACHE_TTL_SECS {
            return cached.clone();
        }
    }
    *guard = Some((std::time::Instant::now(), result.clone()));
    result
}

async fn status_inner() -> SearxngDockerStatus {
    let (installed, daemon_running) = docker_status().await;
    let (deploying, deploy_step, deploy_logs) = get_deploy_progress();
    let empty_status = SearxngDockerStatus {
        docker_installed: false,
        docker_not_running: false,
        container_exists: false,
        container_running: false,
        port: None,
        health_ok: false,
        deploying,
        deploy_step: deploy_step.clone(),
        deploy_logs: deploy_logs.clone(),
        search_ok: false,
        search_result_count: 0,
        unresponsive_engines: vec![],
    };

    if !installed {
        return empty_status;
    }
    if !daemon_running {
        return SearxngDockerStatus {
            docker_installed: true,
            docker_not_running: true,
            ..empty_status
        };
    }

    let (container_exists, container_running) = inspect_container().await;
    let port = if container_exists {
        inspect_port().await
    } else {
        None
    };
    let health_ok = if container_running {
        if let Some(p) = port {
            health_check(p, 2, 1).await
        } else {
            false
        }
    } else {
        false
    };

    // Real search test: verify results are returned and report engine health
    let (search_ok, search_result_count, unresponsive_engines) = if health_ok {
        if let Some(p) = port {
            search_test(p).await
        } else {
            (false, 0, vec![])
        }
    } else {
        (false, 0, vec![])
    };

    SearxngDockerStatus {
        docker_installed: true,
        docker_not_running: false,
        container_exists,
        container_running,
        port,
        health_ok,
        deploying,
        deploy_step,
        deploy_logs,
        search_ok,
        search_result_count,
        unresponsive_engines,
    }
}
