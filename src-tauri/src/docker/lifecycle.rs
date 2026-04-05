use anyhow::{Context, Result};
use std::sync::atomic::Ordering;
use tokio::process::Command;

use super::{app_log, error, helpers::*, info, CONTAINER_NAME, DEPLOYING};

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
