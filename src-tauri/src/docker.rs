use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Command;

const CONTAINER_NAME: &str = "opencomputer-searxng";
const IMAGE: &str = "searxng/searxng";
const DEFAULT_HOST_PORT: u16 = 8080;
const SEARXNG_DIR_NAME: &str = "searxng";

/// Prevent concurrent deploy/start/stop/remove operations.
static DEPLOYING: AtomicBool = AtomicBool::new(false);

/// Shared deploy progress: (current_step, log_lines). Readable by any UI.
static DEPLOY_PROGRESS: std::sync::LazyLock<std::sync::Mutex<DeployProgress>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(DeployProgress::default()));

#[derive(Default, Clone)]
struct DeployProgress {
    step: Option<String>,
    logs: Vec<String>,
}

/// Prevent concurrent status() calls; cache recent result to avoid redundant search tests.
static STATUS_LOCK: std::sync::LazyLock<
    tokio::sync::Mutex<Option<(std::time::Instant, SearxngDockerStatus)>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));
/// Status cache TTL — skip search_test if last result is fresh enough.
const STATUS_CACHE_TTL_SECS: u64 = 5;

const LOG_CAT: &str = "docker";
const LOG_SRC: &str = "SearXNG";

/// Write to AppLogger (SQLite + file). Falls back to log::info! if logger unavailable.
fn app_log(level: &str, message: &str, details: Option<String>) {
    if let Some(logger) = crate::get_logger() {
        logger.log(level, LOG_CAT, LOG_SRC, message, details, None, None);
    }
}

fn get_deploy_progress() -> (bool, Option<String>, Vec<String>) {
    let deploying = DEPLOYING.load(Ordering::SeqCst);
    if !deploying {
        return (false, None, vec![]);
    }
    let guard = DEPLOY_PROGRESS.lock().unwrap_or_else(|e| e.into_inner());
    (true, guard.step.clone(), guard.logs.clone())
}

fn info(msg: &str) {
    app_log("info", msg, None);
}

fn error(msg: &str, details: &str) {
    app_log("error", msg, Some(details.to_string()));
}

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
    let mut guard = STATUS_LOCK.lock().await;

    // Return cached result if fresh enough
    if let Some((ts, ref cached)) = *guard {
        if ts.elapsed().as_secs() < STATUS_CACHE_TTL_SECS {
            return cached.clone();
        }
    }

    let result = status_inner().await;
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

// ── Deploy ───────────────────────────────────────────────────────

/// Pull image, start container, inject config, health-check.
/// Returns the accessible URL (e.g. "http://127.0.0.1:8080").
/// `on_progress` is called with JSON messages: `{"step":"..."}` or `{"log":"..."}`.
pub async fn deploy<F>(on_progress: F) -> Result<String>
where
    F: Fn(&str),
{
    // Prevent concurrent deploy operations
    if DEPLOYING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        anyhow::bail!("A deploy operation is already in progress");
    }
    let result = deploy_inner(&on_progress).await;
    DEPLOYING.store(false, Ordering::SeqCst);
    // Clear shared progress after completion
    if let Ok(mut p) = DEPLOY_PROGRESS.lock() {
        *p = DeployProgress::default();
    }
    // Invalidate status cache so next poll picks up new state
    if let Ok(mut guard) = STATUS_LOCK.try_lock() {
        *guard = None;
    }
    result
}

async fn deploy_inner<F>(on_progress: &F) -> Result<String>
where
    F: Fn(&str),
{
    // Clear previous progress
    if let Ok(mut p) = DEPLOY_PROGRESS.lock() {
        *p = DeployProgress::default();
    }

    let step = |s: &str| {
        on_progress(&format!(r#"{{"step":"{}"}}"#, s));
        if let Ok(mut p) = DEPLOY_PROGRESS.lock() {
            p.step = Some(s.to_string());
        }
    };
    let log = |msg: &str| {
        on_progress(&format!(
            r#"{{"log":"{}"}}"#,
            msg.replace('\\', "\\\\").replace('"', "\\\"")
        ));
        if let Ok(mut p) = DEPLOY_PROGRESS.lock() {
            p.logs.push(msg.to_string());
            // Keep last 100 lines
            let len = p.logs.len();
            if len > 100 {
                p.logs.drain(..len - 100);
            }
        }
    };

    // 1. Check Docker daemon
    step("checking_docker");
    if !docker_available().await {
        log("ERROR: Docker daemon is not running");
        anyhow::bail!("Docker daemon is not running. Please start Docker Desktop first.");
    }
    log("Docker daemon is available");

    // 2. Pull image (stream output)
    step("pulling_image");
    log(&format!("docker pull {}", IMAGE));
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut child = Command::new("docker")
            .args(["pull", IMAGE])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn docker pull")?;

        // Read stdout lines (pull progress)
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() {
                    log(&trimmed);
                }
            }
        }

        let status = child.wait().await.context("docker pull process failed")?;
        if !status.success() {
            let err_msg = if let Some(err) = stderr {
                let mut buf = String::new();
                let mut reader = BufReader::new(err);
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut buf).await;
                buf
            } else {
                "unknown error".to_string()
            };
            log(&format!("ERROR: {}", err_msg));
            anyhow::bail!("docker pull failed: {}", err_msg);
        }
    }
    log("Image pulled successfully");

    // 3. Remove stale container if exists
    step("removing_old");
    log(&format!("docker rm -f {}", CONTAINER_NAME));
    let rm_out = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .output()
        .await;
    if let Ok(ref o) = rm_out {
        if o.status.success() {
            log("Removed old container");
        }
    }

    // 4. Prepare config directory & settings.yml
    step("injecting_config");
    let config_dir = prepare_searxng_config().await?;
    log(&format!(
        "Config: {}",
        config_dir.join("settings.yml").display()
    ));

    // 5. Find available port
    let port = find_available_port().await;
    log(&format!("Selected port: {}", port));

    // 6. Start container with volume-mounted settings.yml + proxy env
    step("starting_container");

    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        CONTAINER_NAME.to_string(),
        "-p".to_string(),
        format!("{}:8080", port),
        "-v".to_string(),
        format!(
            "{}:/etc/searxng/settings.yml:ro",
            config_dir.join("settings.yml").to_string_lossy()
        ),
        "-e".to_string(),
        "SEARXNG_BASE_URL=http://localhost:8080".to_string(),
    ];

    // Inject proxy env vars so SearXNG's upstream engines can reach Google etc.
    if let Some(proxy_url) = resolve_proxy_for_container() {
        log(&format!("Proxy: {}", proxy_url));
        for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            args.push("-e".to_string());
            args.push(format!("{}={}", var, proxy_url));
        }
    }

    args.push(IMAGE.to_string());

    log(&format!(
        "docker run -d --name {} -p {}:8080 ...",
        CONTAINER_NAME, port
    ));
    let run = Command::new("docker")
        .args(&args)
        .output()
        .await
        .context("Failed to run docker run")?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr).to_string();
        log(&format!("ERROR: {}", stderr));
        anyhow::bail!("docker run failed: {}", stderr);
    }
    let container_id = String::from_utf8_lossy(&run.stdout).trim().to_string();
    let short_id = &container_id[..12.min(container_id.len())];
    log(&format!("Container started ({})", short_id));

    // 7. Health check (up to 30s)
    step("health_check");
    log(&format!("Waiting for health check on port {}...", port));
    if !health_check(port, 30, 1).await {
        let logs = fetch_container_logs(50).await;
        log(&format!("ERROR: Health check timed out. Logs:\n{}", logs));
        anyhow::bail!("Health check timed out (30s). Container logs:\n{}", logs);
    }
    log("Health check passed");

    let url = format!("http://127.0.0.1:{}", port);
    step("done");
    log(&format!("Deployed at {}", url));
    Ok(url)
}

// ── Lifecycle ────────────────────────────────────────────────────

pub async fn start() -> Result<()> {
    if DEPLOYING.load(Ordering::SeqCst) {
        anyhow::bail!("A deploy operation is in progress");
    }
    // Refresh the mounted config before starting so proxy changes in settings
    // take effect without requiring a full redeploy.
    prepare_searxng_config().await?;
    info("Starting container...");
    let out = Command::new("docker")
        .args(["start", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to start container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error("docker start failed", &stderr);
        anyhow::bail!("docker start failed: {}", stderr);
    }
    info("Container started, waiting for ready...");
    // Brief wait then health check — don't block too long, frontend will poll status
    if let Some(port) = inspect_port().await {
        if health_check(port, 5, 1).await {
            info("Started and healthy");
        } else {
            // Not fatal — container is running, just not ready yet
            app_log(
                "warn",
                "Container started but not yet healthy, frontend will poll",
                None,
            );
        }
    }
    Ok(())
}

pub async fn stop() -> Result<()> {
    if DEPLOYING.load(Ordering::SeqCst) {
        anyhow::bail!("A deploy operation is in progress");
    }
    info("Stopping container...");
    let out = Command::new("docker")
        .args(["stop", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to stop container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error("docker stop failed", &stderr);
        anyhow::bail!("docker stop failed: {}", stderr);
    }
    info("Container stopped");
    Ok(())
}

pub async fn remove() -> Result<()> {
    if DEPLOYING.load(Ordering::SeqCst) {
        anyhow::bail!("A deploy operation is in progress");
    }
    info("Removing container...");
    let out = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to remove container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        error("docker rm failed", &stderr);
        anyhow::bail!("docker rm failed: {}", stderr);
    }
    info("Container removed");
    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────

/// Returns (cli_installed, daemon_running).
async fn docker_status() -> (bool, bool) {
    // Check if docker CLI exists
    let version = Command::new("docker")
        .args(["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    let cli_installed = version.map(|s| s.success()).unwrap_or(false);
    if !cli_installed {
        return (false, false);
    }
    // Check if daemon is running
    let info = Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    let daemon_running = info.map(|s| s.success()).unwrap_or(false);
    (true, daemon_running)
}

/// Quick check: is docker daemon responsive?
async fn docker_available() -> bool {
    let (_, running) = docker_status().await;
    running
}

/// Returns (exists, running).
async fn inspect_container() -> (bool, bool) {
    let out = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", CONTAINER_NAME])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, text == "true")
        }
        _ => (false, false),
    }
}

/// Parse the host port from docker inspect.
async fn inspect_port() -> Option<u16> {
    let out = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{(index (index .NetworkSettings.Ports \"8080/tcp\") 0).HostPort}}",
            CONTAINER_NAME,
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    text.parse::<u16>().ok()
}

/// Check if a TCP port is available.
async fn is_port_available(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_ok()
}

/// Find an available port starting from DEFAULT_HOST_PORT.
async fn find_available_port() -> u16 {
    for port in DEFAULT_HOST_PORT..DEFAULT_HOST_PORT + 10 {
        if is_port_available(port).await {
            return port;
        }
    }
    // Fallback: let OS pick
    DEFAULT_HOST_PORT
}

/// Generate a random hex string for SearXNG secret_key.
fn generate_secret_key() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", seed)
}

/// SearXNG config directory under ~/.opencomputer/searxng/
fn searxng_config_dir() -> Result<PathBuf> {
    let dir = crate::paths::root_dir()?.join(SEARXNG_DIR_NAME);
    Ok(dir)
}

/// Write settings.yml to local disk for volume mounting.
/// Reuses the existing secret_key when present, otherwise generates a random
/// one to avoid the default "ultrasecretkey" crash.
/// Returns the config directory path.
async fn prepare_searxng_config() -> Result<PathBuf> {
    let dir = searxng_config_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .context("Failed to create SearXNG config directory")?;

    let settings_path = dir.join("settings.yml");
    let secret = load_existing_secret_key(&settings_path)
        .await
        .unwrap_or_else(generate_secret_key);

    // Build outgoing proxy config if available
    // SearXNG uses its own network module and does NOT read HTTP_PROXY env vars.
    // Must configure via settings.yml outgoing.proxies.
    let proxy_section = resolve_proxy_for_container()
        .map(|url| {
            format!(
                r#"outgoing:
  proxies:
    all://:
      - {}
  request_timeout: 10.0
"#,
                url
            )
        })
        .unwrap_or_default();

    let config = format!(
        r#"use_default_settings: true
server:
  secret_key: "{}"
  limiter: false
search:
  formats:
    - html
    - json
{}"#,
        secret, proxy_section
    );
    tokio::fs::write(&settings_path, config)
        .await
        .context("Failed to write SearXNG settings.yml")?;
    info(&format!(
        "Wrote settings.yml to {}",
        settings_path.display()
    ));
    Ok(dir)
}

async fn load_existing_secret_key(settings_path: &std::path::Path) -> Option<String> {
    let existing = tokio::fs::read_to_string(settings_path).await.ok()?;
    existing.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix("secret_key:")?.trim();
        let value = value.trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
}

/// Fetch recent container logs for diagnostics.
async fn fetch_container_logs(tail: u32) -> String {
    let out = Command::new("docker")
        .args(["logs", "--tail", &tail.to_string(), CONTAINER_NAME])
        .output()
        .await;
    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stdout.is_empty() && !stderr.is_empty() {
                format!("[stdout]\n{}\n[stderr]\n{}", stdout.trim(), stderr.trim())
            } else if !stderr.is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            }
        }
        Err(e) => format!("(failed to fetch logs: {})", e),
    }
}

/// Resolve the proxy URL to inject into the Docker container.
/// - Custom mode: use the configured URL
/// - System mode: env vars → macOS scutil --proxy fallback
/// - None mode: no proxy
/// All localhost/127.0.0.1 addresses are rewritten to host.docker.internal.
fn resolve_proxy_for_container() -> Option<String> {
    if !crate::provider::load_store()
        .map(|s| s.web_search.searxng_docker_use_proxy)
        .unwrap_or(true)
    {
        return None;
    }

    let config = crate::provider::load_proxy_config();
    let raw_url = match config.mode {
        crate::provider::ProxyMode::Custom => config.url.filter(|u| !u.is_empty()),
        crate::provider::ProxyMode::System => {
            // 1. Try env vars first
            std::env::var("HTTPS_PROXY")
                .ok()
                .or_else(|| std::env::var("HTTP_PROXY").ok())
                .or_else(|| std::env::var("ALL_PROXY").ok())
                .or_else(|| std::env::var("https_proxy").ok())
                .or_else(|| std::env::var("http_proxy").ok())
                .or_else(|| std::env::var("all_proxy").ok())
                .filter(|u| !u.is_empty())
                // 2. Fallback: read macOS system proxy (Shadowrocket, ClashX, etc.)
                .or_else(detect_macos_system_proxy)
        }
        crate::provider::ProxyMode::None => return None,
    };
    raw_url.map(|u| {
        // Docker containers can't reach host's 127.0.0.1; use special DNS name
        u.replace("127.0.0.1", "host.docker.internal")
            .replace("localhost", "host.docker.internal")
    })
}

/// Read macOS system proxy via `scutil --proxy`.
/// Returns e.g. `Some("http://127.0.0.1:1082")`.
#[cfg(target_os = "macos")]
fn detect_macos_system_proxy() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);

    // Parse "HTTPSEnable : 1", "HTTPSProxy : 127.0.0.1", "HTTPSPort : 1082"
    // Prefer HTTPS proxy, fallback to HTTP proxy
    for prefix in ["HTTPS", "HTTP"] {
        let enabled = text
            .lines()
            .find(|l| l.trim().starts_with(&format!("{}Enable", prefix)))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim() == "1")
            .unwrap_or(false);
        if !enabled {
            continue;
        }

        let host = text
            .lines()
            .find(|l| {
                l.trim().starts_with(&format!("{}Proxy", prefix))
                    && !l.contains("Enable")
                    && !l.contains("Port")
            })
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let port = text
            .lines()
            .find(|l| l.trim().starts_with(&format!("{}Port", prefix)))
            .and_then(|l| l.split(':').nth(1))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        if let (Some(h), Some(p)) = (host, port) {
            let url = format!("http://{}:{}", h, p);
            info(&format!("Detected macOS system proxy: {}", url));
            return Some(url);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_system_proxy() -> Option<String> {
    None
}

/// Perform a real search and verify results are returned.
/// Returns (search_ok, result_count, unresponsive_engines).
async fn search_test(port: u16) -> (bool, usize, Vec<String>) {
    let url = format!(
        "http://127.0.0.1:{}/search?q=test&format=json&categories=general",
        port
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .no_proxy()
        .build()
        .unwrap_or_default();

    let resp = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            app_log("warn", &format!("Search test HTTP {}", r.status()), None);
            return (false, 0, vec![]);
        }
        Err(e) => {
            app_log("warn", &format!("Search test request failed: {}", e), None);
            return (false, 0, vec![]);
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            app_log(
                "warn",
                &format!("Search test JSON parse failed: {}", e),
                None,
            );
            return (false, 0, vec![]);
        }
    };

    let result_count = body
        .get("results")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let unresponsive: Vec<String> = body
        .get("unresponsive_engines")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let a = item.as_array()?;
                    let engine = a.first()?.as_str()?;
                    let reason = a.get(1).and_then(|v| v.as_str()).unwrap_or("unknown");
                    Some(format!("{}: {}", engine, reason))
                })
                .collect()
        })
        .unwrap_or_default();

    let search_ok = result_count > 0;
    if search_ok {
        info(&format!(
            "Search test passed: {} results, {} unresponsive engines",
            result_count,
            unresponsive.len()
        ));
    } else {
        app_log(
            "warn",
            &format!(
                "Search test returned 0 results, unresponsive: {:?}",
                unresponsive
            ),
            None,
        );
    }

    (search_ok, result_count, unresponsive)
}

/// Poll the SearXNG JSON endpoint until it responds 200.
async fn health_check(port: u16, max_attempts: u32, interval_secs: u64) -> bool {
    let url = format!("http://127.0.0.1:{}/search?q=test&format=json", port);
    // SearXNG is local — no proxy needed
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .no_proxy()
        .build()
        .unwrap_or_default();

    for attempt in 1..=max_attempts {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info(&format!("Health check passed (attempt {})", attempt));
                return true;
            }
            Ok(resp) => {
                app_log(
                    "debug",
                    &format!(
                        "Health check attempt {} — status {}",
                        attempt,
                        resp.status()
                    ),
                    None,
                );
            }
            Err(e) => {
                app_log(
                    "debug",
                    &format!("Health check attempt {} — {}", attempt, e),
                    None,
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
    false
}
