use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::process::Command;

const CONTAINER_NAME: &str = "opencomputer-searxng";
const IMAGE: &str = "searxng/searxng";
const DEFAULT_HOST_PORT: u16 = 8080;
const SEARXNG_DIR_NAME: &str = "searxng";

const LOG_CAT: &str = "docker";
const LOG_SRC: &str = "SearXNG";

/// Write to AppLogger (SQLite + file). Falls back to log::info! if logger unavailable.
fn app_log(level: &str, message: &str, details: Option<String>) {
    if let Some(logger) = crate::get_logger() {
        logger.log(level, LOG_CAT, LOG_SRC, message, details, None, None);
    }
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
}

/// Gather full status of Docker + SearXNG container.
pub async fn status() -> SearxngDockerStatus {
    let (installed, daemon_running) = docker_status().await;
    if !installed {
        return SearxngDockerStatus {
            docker_installed: false,
            docker_not_running: false,
            container_exists: false,
            container_running: false,
            port: None,
            health_ok: false,
        };
    }
    if !daemon_running {
        return SearxngDockerStatus {
            docker_installed: true,
            docker_not_running: true,
            container_exists: false,
            container_running: false,
            port: None,
            health_ok: false,
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

    SearxngDockerStatus {
        docker_installed: true,
        docker_not_running: false,
        container_exists,
        container_running,
        port,
        health_ok,
    }
}

// ── Deploy ───────────────────────────────────────────────────────

/// Pull image, start container, inject config, health-check.
/// Returns the accessible URL (e.g. "http://localhost:8080").
/// `on_progress` is called with a step description for UI feedback.
pub async fn deploy<F>(on_progress: F) -> Result<String>
where
    F: Fn(&str),
{
    // 1. Check Docker daemon
    on_progress("checking_docker");
    if !docker_available().await {
        error("Docker daemon is not running", "deploy aborted");
        anyhow::bail!("Docker daemon is not running. Please start Docker Desktop first.");
    }
    info("Docker daemon is available");

    // 2. Pull image
    on_progress("pulling_image");
    info(&format!("Pulling image {}", IMAGE));
    let pull = Command::new("docker")
        .args(["pull", IMAGE])
        .output()
        .await
        .context("Failed to run docker pull")?;
    if !pull.status.success() {
        let stderr = String::from_utf8_lossy(&pull.stderr).to_string();
        error("docker pull failed", &stderr);
        anyhow::bail!("docker pull failed: {}", stderr);
    }
    info("Image pulled successfully");

    // 3. Remove stale container if exists
    on_progress("removing_old");
    let rm_out = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .output()
        .await;
    if let Ok(ref o) = rm_out {
        if o.status.success() {
            info("Removed old container");
        }
    }

    // 4. Prepare config directory & settings.yml
    on_progress("injecting_config");
    let config_dir = prepare_searxng_config().await?;
    let config_dir_str = config_dir.to_string_lossy().to_string();
    info(&format!("Config directory: {}", config_dir_str));

    // 5. Find available port
    let port = find_available_port().await;
    info(&format!("Selected port {}", port));

    // 6. Start container with volume-mounted settings.yml
    on_progress("starting_container");
    info(&format!("Starting container on port {}", port));
    let run = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            CONTAINER_NAME,
            "-p",
            &format!("{}:8080", port),
            "-v",
            &format!("{}:/etc/searxng/settings.yml:ro", config_dir.join("settings.yml").to_string_lossy()),
            "-e",
            "SEARXNG_BASE_URL=http://localhost:8080",
            IMAGE,
        ])
        .output()
        .await
        .context("Failed to run docker run")?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr).to_string();
        error("docker run failed", &stderr);
        anyhow::bail!("docker run failed: {}", stderr);
    }
    let container_id = String::from_utf8_lossy(&run.stdout).trim().to_string();
    info(&format!("Container started ({})", &container_id[..12.min(container_id.len())]));

    // 8. Health check (up to 30s)
    on_progress("health_check");
    info(&format!("Waiting for health check on port {}...", port));
    if !health_check(port, 30, 1).await {
        let logs = fetch_container_logs(50).await;
        error("Health check timed out", &logs);
        anyhow::bail!("Health check timed out (30s). Container logs:\n{}", logs);
    }

    let url = format!("http://localhost:{}", port);
    on_progress("done");
    info(&format!("Deployed successfully at {}", url));
    Ok(url)
}

// ── Lifecycle ────────────────────────────────────────────────────

pub async fn start() -> Result<()> {
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
            app_log("warn", "Container started but not yet healthy, frontend will poll", None);
        }
    }
    Ok(())
}

pub async fn stop() -> Result<()> {
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
        .args([
            "inspect",
            "--format",
            "{{.State.Running}}",
            CONTAINER_NAME,
        ])
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
/// Generates a random secret_key to avoid the default "ultrasecretkey" crash.
/// Returns the config directory path.
async fn prepare_searxng_config() -> Result<PathBuf> {
    let dir = searxng_config_dir()?;
    tokio::fs::create_dir_all(&dir).await
        .context("Failed to create SearXNG config directory")?;

    let secret = generate_secret_key();
    let settings_path = dir.join("settings.yml");
    let config = format!(
        r#"use_default_settings: true
server:
  secret_key: "{}"
  limiter: false
search:
  formats:
    - html
    - json
"#,
        secret
    );
    tokio::fs::write(&settings_path, config).await
        .context("Failed to write SearXNG settings.yml")?;
    info(&format!("Wrote settings.yml to {}", settings_path.display()));
    Ok(dir)
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

/// Poll the SearXNG JSON endpoint until it responds 200.
async fn health_check(port: u16, max_attempts: u32, interval_secs: u64) -> bool {
    let url = format!(
        "http://127.0.0.1:{}/search?q=test&format=json",
        port
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for attempt in 1..=max_attempts {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info(&format!("Health check passed (attempt {})", attempt));
                return true;
            }
            Ok(resp) => {
                app_log("debug", &format!("Health check attempt {} — status {}", attempt, resp.status()), None);
            }
            Err(e) => {
                app_log("debug", &format!("Health check attempt {} — {}", attempt, e), None);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
    false
}
