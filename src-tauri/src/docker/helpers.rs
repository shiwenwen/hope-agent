use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::process::Command;

use super::{app_log, info, CONTAINER_NAME, DEFAULT_HOST_PORT, SEARXNG_DIR_NAME};

// ── Internal helpers ─────────────────────────────────────────────

/// Returns (cli_installed, daemon_running).
pub(super) async fn docker_status() -> (bool, bool) {
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
pub(super) async fn docker_available() -> bool {
    let (_, running) = docker_status().await;
    running
}

/// Returns (exists, running).
pub(super) async fn inspect_container() -> (bool, bool) {
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
pub(super) async fn inspect_port() -> Option<u16> {
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
pub(super) async fn is_port_available(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_ok()
}

/// Find an available port starting from DEFAULT_HOST_PORT.
pub(super) async fn find_available_port() -> u16 {
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
pub(super) async fn prepare_searxng_config() -> Result<PathBuf> {
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
    let proxy_section = super::proxy::resolve_proxy_for_container()
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
pub(super) async fn fetch_container_logs(tail: u32) -> String {
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

/// Perform a real search and verify results are returned.
/// Returns (search_ok, result_count, unresponsive_engines).
pub(super) async fn search_test(port: u16) -> (bool, usize, Vec<String>) {
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
pub(super) async fn health_check(port: u16, max_attempts: u32, interval_secs: u64) -> bool {
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
