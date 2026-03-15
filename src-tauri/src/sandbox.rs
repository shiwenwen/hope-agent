use anyhow::Result;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::HostConfig;
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

const DEFAULT_SANDBOX_IMAGE: &str = "ubuntu:22.04";

// ── Sandbox Configuration ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub image: String,
    /// Memory limit in bytes (default 512MB)
    pub memory_limit: Option<i64>,
    /// CPU limit as number of CPUs (default 1.0)
    pub cpu_limit: Option<f64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_SANDBOX_IMAGE.to_string(),
            memory_limit: Some(512 * 1024 * 1024), // 512MB
            cpu_limit: Some(1.0),
        }
    }
}

pub struct SandboxResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub timed_out: bool,
}

// ── Configuration Persistence ─────────────────────────────────────

fn sandbox_config_path() -> Result<std::path::PathBuf> {
    Ok(crate::paths::root_dir()?.join("sandbox.json"))
}

pub fn load_sandbox_config() -> Result<SandboxConfig> {
    let path = sandbox_config_path()?;
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(SandboxConfig::default())
    }
}

#[allow(dead_code)]
pub fn save_sandbox_config(config: &SandboxConfig) -> Result<()> {
    let path = sandbox_config_path()?;
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(path, data)?;
    Ok(())
}

// ── Docker Operations ─────────────────────────────────────────────

/// Check if Docker is available and running.
pub async fn check_docker_available() -> bool {
    match Docker::connect_with_local_defaults() {
        Ok(docker) => docker.ping().await.is_ok(),
        Err(_) => false,
    }
}

/// Ensure the specified image is available locally, pulling if needed.
async fn ensure_image(docker: &Docker, image: &str) -> Result<()> {
    // Check if image exists locally
    if docker.inspect_image(image).await.is_ok() {
        return Ok(());
    }

    log::info!("Pulling Docker image: {}", image);

    let (repo, tag) = if let Some(idx) = image.rfind(':') {
        (&image[..idx], &image[idx + 1..])
    } else {
        (image, "latest")
    };

    let options = CreateImageOptions {
        from_image: repo,
        tag,
        ..Default::default()
    };

    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = info.status {
                    log::debug!("Pull: {}", status);
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to pull image '{}': {}", image, e));
            }
        }
    }

    Ok(())
}

/// Execute a command inside a Docker container.
///
/// Lifecycle: create container → start → wait (with timeout) → collect logs → remove.
pub async fn exec_in_sandbox(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
) -> Result<SandboxResult> {
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| anyhow::anyhow!("Cannot connect to Docker: {}. Is Docker running?", e))?;

    // Ensure image is available
    ensure_image(&docker, &config.image).await?;

    // Build environment variables
    let mut env_vec: Vec<String> = Vec::new();
    if let Some(env_map) = env {
        for (key, val) in env_map {
            if let Some(v) = val.as_str() {
                env_vec.push(format!("{}={}", key, v));
            }
        }
    }

    // Resolve current UID:GID to avoid permission issues on mounted volumes
    let user = {
        #[cfg(unix)]
        {
            format!("{}:{}", unsafe { libc::getuid() }, unsafe {
                libc::getgid()
            })
        }
        #[cfg(not(unix))]
        {
            String::new()
        }
    };

    // Resolve absolute path for the working directory mount
    let host_cwd = std::path::Path::new(cwd)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(cwd));
    let bind_mount = format!("{}:/workspace", host_cwd.display());

    // Build host config with resource limits
    let mut host_config = HostConfig {
        binds: Some(vec![bind_mount]),
        ..Default::default()
    };
    if let Some(mem) = config.memory_limit {
        host_config.memory = Some(mem);
    }
    if let Some(cpus) = config.cpu_limit {
        host_config.nano_cpus = Some((cpus * 1_000_000_000.0) as i64);
    }

    // Create container
    let container_config = Config {
        image: Some(config.image.clone()),
        cmd: Some(vec!["sh".to_string(), "-c".to_string(), command.to_string()]),
        working_dir: Some("/workspace".to_string()),
        env: if env_vec.is_empty() {
            None
        } else {
            Some(env_vec)
        },
        user: if user.is_empty() { None } else { Some(user) },
        host_config: Some(host_config),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };

    let container_name = format!("opencomputer-sandbox-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("tmp"));

    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: &container_name,
                platform: None,
            }),
            container_config,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create container: {}", e))?;

    let container_id = container.id.clone();

    // Start container
    docker
        .start_container::<String>(&container_id, None)
        .await
        .map_err(|e| {
            // Schedule cleanup on start failure
            let docker_clone = docker.clone();
            let cid = container_id.clone();
            tokio::spawn(async move {
                let _ = cleanup_container(&docker_clone, &cid).await;
            });
            anyhow::anyhow!("Failed to start container: {}", e)
        })?;

    log::info!(
        "Sandbox container started: {} (image: {}, command: {})",
        &container_id[..12],
        config.image,
        command
    );

    // Wait for container to finish (with timeout)
    let wait_result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        wait_for_container(&docker, &container_id),
    )
    .await;

    let (exit_code, timed_out) = match wait_result {
        Ok(Ok(code)) => (code, false),
        Ok(Err(e)) => {
            log::warn!("Container wait error: {}", e);
            // Try to stop and cleanup
            let _ = docker
                .stop_container(&container_id, None)
                .await;
            let _ = cleanup_container(&docker, &container_id).await;
            return Err(anyhow::anyhow!("Container execution failed: {}", e));
        }
        Err(_) => {
            // Timeout — kill the container
            log::warn!(
                "Sandbox container timed out after {}s, killing...",
                timeout_secs
            );
            let _ = docker
                .stop_container(&container_id, None)
                .await;
            (-1, true)
        }
    };

    // Collect logs
    let (stdout, stderr) = collect_logs(&docker, &container_id).await?;

    // Cleanup container
    let _ = cleanup_container(&docker, &container_id).await;

    Ok(SandboxResult {
        stdout,
        stderr,
        exit_code,
        timed_out,
    })
}

/// Wait for a container to exit and return its exit code.
async fn wait_for_container(docker: &Docker, container_id: &str) -> Result<i64> {
    let options = WaitContainerOptions {
        condition: "not-running",
    };

    let mut stream = docker.wait_container(container_id, Some(options));
    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                return Ok(response.status_code);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Wait error: {}", e));
            }
        }
    }

    Err(anyhow::anyhow!("Container wait stream ended unexpectedly"))
}

/// Collect stdout and stderr logs from a container.
async fn collect_logs(docker: &Docker, container_id: &str) -> Result<(String, String)> {
    let options = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        follow: false,
        ..Default::default()
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut stream = docker.logs(container_id, Some(options));

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => match output {
                bollard::container::LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(&message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            },
            Err(e) => {
                log::warn!("Error reading container logs: {}", e);
                break;
            }
        }
    }

    Ok((stdout, stderr))
}

/// Remove a container (force + remove volumes).
async fn cleanup_container(docker: &Docker, container_id: &str) -> Result<()> {
    docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                v: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to remove container: {}", e))?;
    log::info!("Sandbox container removed: {}", &container_id[..12.min(container_id.len())]);
    Ok(())
}
