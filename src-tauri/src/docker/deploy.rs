use anyhow::{Context, Result};
use std::sync::atomic::Ordering;
use tokio::process::Command;

use super::{
    helpers::*, info, CONTAINER_NAME, DEPLOYING, DEPLOY_PROGRESS, DeployProgress, IMAGE,
    STATUS_LOCK,
};

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
    if let Some(proxy_url) = super::proxy::resolve_proxy_for_container() {
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
