use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const CONTAINER_NAME: &str = "opencomputer-searxng";
const IMAGE: &str = "searxng/searxng";
const DEFAULT_HOST_PORT: u16 = 8080;

// ── Public Status ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearxngDockerStatus {
    pub docker_installed: bool,
    pub container_exists: bool,
    pub container_running: bool,
    pub port: Option<u16>,
    pub health_ok: bool,
}

/// Gather full status of Docker + SearXNG container.
pub async fn status() -> SearxngDockerStatus {
    let docker_installed = docker_available().await;
    if !docker_installed {
        return SearxngDockerStatus {
            docker_installed: false,
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
            health_check(p, 1, 1).await
        } else {
            false
        }
    } else {
        false
    };

    SearxngDockerStatus {
        docker_installed,
        container_exists,
        container_running,
        port,
        health_ok,
    }
}

// ── Deploy ───────────────────────────────────────────────────────

/// Pull image, start container, inject config, health-check.
/// Returns the accessible URL (e.g. "http://localhost:8080").
pub async fn deploy() -> Result<String> {
    // 1. Pull image
    log::info!("SearXNG Docker: pulling image {}", IMAGE);
    let pull = Command::new("docker")
        .args(["pull", IMAGE])
        .output()
        .await
        .context("Failed to run docker pull")?;
    if !pull.status.success() {
        let stderr = String::from_utf8_lossy(&pull.stderr);
        anyhow::bail!("docker pull failed: {}", stderr);
    }

    // 2. Remove stale container if exists
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .output()
        .await;

    // 3. Find available port
    let port = find_available_port().await;

    // 4. Start container
    log::info!("SearXNG Docker: starting container on port {}", port);
    let run = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            CONTAINER_NAME,
            "-p",
            &format!("{}:8080", port),
            "-e",
            "SEARXNG_BASE_URL=http://localhost:8080",
            IMAGE,
        ])
        .output()
        .await
        .context("Failed to run docker run")?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        anyhow::bail!("docker run failed: {}", stderr);
    }

    // 5. Wait a moment for container init, then inject settings.yml
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    inject_searxng_config().await?;

    // 6. Restart to pick up new config
    let restart = Command::new("docker")
        .args(["restart", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to restart container")?;
    if !restart.status.success() {
        let stderr = String::from_utf8_lossy(&restart.stderr);
        anyhow::bail!("docker restart failed: {}", stderr);
    }

    // 7. Health check (up to 20s)
    log::info!("SearXNG Docker: waiting for health check...");
    if !health_check(port, 20, 1).await {
        anyhow::bail!("SearXNG container started but health check timed out after 20s");
    }

    let url = format!("http://localhost:{}", port);
    log::info!("SearXNG Docker: deployed successfully at {}", url);
    Ok(url)
}

// ── Lifecycle ────────────────────────────────────────────────────

pub async fn start() -> Result<()> {
    let out = Command::new("docker")
        .args(["start", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to start container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("docker start failed: {}", stderr);
    }
    // Wait for ready
    if let Some(port) = inspect_port().await {
        health_check(port, 10, 1).await;
    }
    Ok(())
}

pub async fn stop() -> Result<()> {
    let out = Command::new("docker")
        .args(["stop", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to stop container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("docker stop failed: {}", stderr);
    }
    Ok(())
}

pub async fn remove() -> Result<()> {
    let out = Command::new("docker")
        .args(["rm", "-f", CONTAINER_NAME])
        .output()
        .await
        .context("Failed to remove container")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("docker rm failed: {}", stderr);
    }
    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────

async fn docker_available() -> bool {
    Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
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

/// Inject settings.yml into the running container.
async fn inject_searxng_config() -> Result<()> {
    let config = r#"use_default_settings: true
server:
  limiter: false
search:
  formats:
    - html
    - json
"#;
    let script = format!(
        "cat > /etc/searxng/settings.yml <<'SEARXNG_CONF'\n{}SEARXNG_CONF",
        config
    );
    let out = Command::new("docker")
        .args(["exec", CONTAINER_NAME, "sh", "-c", &script])
        .output()
        .await
        .context("Failed to inject SearXNG config")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("Failed to write SearXNG settings.yml: {}", stderr);
    }
    Ok(())
}

/// Poll the SearXNG JSON endpoint until it responds 200.
async fn health_check(port: u16, max_attempts: u32, interval_secs: u64) -> bool {
    let url = format!(
        "http://localhost:{}/search?q=test&format=json",
        port
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for _ in 0..max_attempts {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
    false
}
