use anyhow::Result;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, LogsOptions, RemoveContainerOptions,
    WaitContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::process::Command;

const DEFAULT_SANDBOX_IMAGE: &str = "debian:bookworm-slim";

// ── Sandbox Configuration ─────────────────────────────────────────
fn default_network_none() -> String {
    "none".to_string()
}
fn default_pids_limit() -> Option<i64> {
    Some(256)
}
fn default_tmpfs() -> Vec<String> {
    vec![
        "/tmp:size=64M".to_string(),
        "/var/tmp:size=32M".to_string(),
        "/run:size=16M".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub image: String,
    /// Memory limit in bytes (default 512MB)
    pub memory_limit: Option<i64>,
    /// CPU limit as number of CPUs (default 1.0)
    pub cpu_limit: Option<f64>,
    /// Mount root filesystem as read-only (default: true)
    #[serde(default = "crate::default_true")]
    pub read_only: bool,
    /// Network mode: "none", "bridge", "host" (default: "none")
    #[serde(default = "default_network_none")]
    pub network_mode: String,
    /// Drop all Linux capabilities (default: true)
    #[serde(default = "crate::default_true")]
    pub cap_drop_all: bool,
    /// Prevent gaining new privileges (default: true)
    #[serde(default = "crate::default_true")]
    pub no_new_privileges: bool,
    /// PID limit inside container (default: 256)
    #[serde(default = "default_pids_limit")]
    pub pids_limit: Option<i64>,
    /// tmpfs mounts for writable temp dirs when read_only is enabled
    #[serde(default = "default_tmpfs")]
    pub tmpfs: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_SANDBOX_IMAGE.to_string(),
            memory_limit: Some(512 * 1024 * 1024), // 512MB
            cpu_limit: Some(1.0),
            read_only: true,
            network_mode: "none".to_string(),
            cap_drop_all: true,
            no_new_privileges: true,
            pids_limit: Some(256),
            tmpfs: default_tmpfs(),
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

pub fn save_sandbox_config(config: &SandboxConfig) -> Result<()> {
    let path = sandbox_config_path()?;
    let data = serde_json::to_string_pretty(config)?;
    std::fs::write(path, data)?;
    Ok(())
}

// ── Environment Variable Sanitization ─────────────────────────────

/// Patterns that match sensitive environment variable names (checked against uppercased key).
const SENSITIVE_ENV_PATTERNS: &[&str] = &[
    "API_KEY",
    "API_SECRET",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "PRIVATE_KEY",
    "ACCESS_KEY",
    "AWS_SECRET",
    "AWS_ACCESS",
    "AWS_SESSION",
    "OPENAI_API",
    "ANTHROPIC_API",
    "AZURE_",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GITLAB_TOKEN",
    "DATABASE_URL",
    "REDIS_URL",
    "MONGO_URI",
];

/// Safe env vars that are always allowed regardless of pattern matching.
const SAFE_ENV_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "USER", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "SHELL", "TMPDIR", "TZ",
    "HOSTNAME", "COLUMNS", "LINES",
];

fn is_env_sensitive(key: &str) -> bool {
    let upper = key.to_uppercase();
    // Never block explicitly safe vars
    if SAFE_ENV_ALLOWLIST.iter().any(|s| upper == *s) {
        return false;
    }
    SENSITIVE_ENV_PATTERNS.iter().any(|pat| upper.contains(pat))
}

/// Sanitize environment variables, blocking sensitive keys.
/// Returns the filtered list and logs blocked vars.
fn sanitize_env(env_map: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut result = Vec::new();
    let mut blocked_count = 0u32;
    for (key, val) in env_map {
        if is_env_sensitive(key) {
            app_warn!(
                "sandbox",
                "env",
                "Blocked sensitive env var from sandbox: {}",
                key
            );
            blocked_count += 1;
            continue;
        }
        if let Some(v) = val.as_str() {
            result.push(format!("{}={}", key, v));
        }
    }
    if blocked_count > 0 {
        app_info!(
            "sandbox",
            "env",
            "Blocked {} sensitive env var(s) from sandbox",
            blocked_count
        );
    }
    result
}

// ── Mount Path Validation ─────────────────────────────────────────

/// Paths that must never be bind-mounted into the sandbox.
const BLOCKED_MOUNT_PATHS: &[&str] = &[
    "/etc",
    "/proc",
    "/sys",
    "/dev",
    "/boot",
    "/root",
    "/var/run/docker.sock",
    "/var/run/docker",
    "/private/var/run/docker.sock",
    "/run/docker.sock",
];

/// Validate that a host path is safe to bind-mount into the sandbox.
fn validate_bind_mount(host_path: &std::path::Path) -> Result<()> {
    let canonical = host_path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("Cannot resolve path '{}': {}", host_path.display(), e))?;
    let path_str = canonical.to_string_lossy();

    // Block root filesystem mount
    if canonical == std::path::Path::new("/") {
        return Err(anyhow::anyhow!(
            "Sandbox security: mounting root filesystem is not allowed"
        ));
    }

    // Block system-critical paths
    for blocked in BLOCKED_MOUNT_PATHS {
        if path_str.as_ref() == *blocked || path_str.starts_with(&format!("{}/", blocked)) {
            return Err(anyhow::anyhow!(
                "Sandbox security: mounting '{}' is not allowed (blocked path: {})",
                host_path.display(),
                blocked
            ));
        }
    }

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

    app_info!("sandbox", "docker", "Pulling Docker image: {}", image);

    let (repo, tag) = if let Some(idx) = image.rfind(':') {
        (&image[..idx], &image[idx + 1..])
    } else {
        (image, "latest")
    };

    let options = CreateImageOptions {
        from_image: Some(repo.to_string()),
        tag: Some(tag.to_string()),
        ..Default::default()
    };

    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = info.status {
                    app_debug!("sandbox", "docker", "Pull: {}", status);
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

    // Build environment variables (with sanitization)
    let env_vec: Vec<String> = if let Some(env_map) = env {
        sanitize_env(env_map)
    } else {
        Vec::new()
    };

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
    let host_cwd = std::path::Path::new(cwd).canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "Cannot resolve sandbox working directory '{}': {}. Ensure the path exists.",
            cwd,
            e
        )
    })?;

    // Validate bind mount path
    validate_bind_mount(&host_cwd)?;

    let bind_mount = format!("{}:/workspace", host_cwd.display());

    // Build host config with resource limits and security hardening
    let mut host_config = HostConfig {
        binds: Some(vec![bind_mount]),
        // Security: read-only root filesystem
        readonly_rootfs: Some(config.read_only),
        // Security: network isolation
        network_mode: Some(config.network_mode.clone()),
        // Security: drop all capabilities
        cap_drop: if config.cap_drop_all {
            Some(vec!["ALL".to_string()])
        } else {
            None
        },
        // Security: prevent privilege escalation
        security_opt: if config.no_new_privileges {
            Some(vec!["no-new-privileges".to_string()])
        } else {
            None
        },
        // Security: PID limit
        pids_limit: config.pids_limit,
        // tmpfs mounts for writable temp dirs when root is read-only
        tmpfs: if config.read_only && !config.tmpfs.is_empty() {
            let mut map = HashMap::new();
            for entry in &config.tmpfs {
                let parts: Vec<&str> = entry.splitn(2, ':').collect();
                let mount_point = parts[0].to_string();
                let options = parts.get(1).unwrap_or(&"").to_string();
                map.insert(mount_point, options);
            }
            Some(map)
        } else {
            None
        },
        ..Default::default()
    };
    if let Some(mem) = config.memory_limit {
        host_config.memory = Some(mem);
    }
    if let Some(cpus) = config.cpu_limit {
        host_config.nano_cpus = Some((cpus * 1_000_000_000.0) as i64);
    }

    // Create container
    let container_config = ContainerCreateBody {
        image: Some(config.image.clone()),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            command.to_string(),
        ]),
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

    let container_name = format!(
        "opencomputer-sandbox-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("tmp")
    );

    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(container_name.clone()),
                platform: String::new(),
            }),
            container_config,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create container: {}", e))?;

    let container_id = container.id.clone();

    // Start container
    if let Err(e) = docker.start_container(&container_id, None).await {
        // Synchronously clean up the failed container before returning error
        if let Err(cleanup_err) = cleanup_container(&docker, &container_id).await {
            app_warn!(
                "sandbox",
                "docker",
                "Failed to cleanup container {}: {}",
                crate::truncate_utf8(&container_id, 12),
                cleanup_err
            );
        }
        return Err(anyhow::anyhow!("Failed to start container: {}", e));
    }

    app_info!(
        "sandbox",
        "docker",
        "Sandbox container started: {} (image: {}, read_only: {}, network: {}, cap_drop_all: {}, command: {})",
        crate::truncate_utf8(&container_id, 12),
        config.image,
        config.read_only,
        config.network_mode,
        config.cap_drop_all,
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
            app_warn!("sandbox", "docker", "Container wait error: {}", e);
            // Try to stop and cleanup
            if let Err(stop_err) = docker.stop_container(&container_id, None).await {
                app_warn!(
                    "sandbox",
                    "docker",
                    "Failed to stop container {}: {}",
                    crate::truncate_utf8(&container_id, 12),
                    stop_err
                );
            }
            if let Err(cleanup_err) = cleanup_container(&docker, &container_id).await {
                app_warn!(
                    "sandbox",
                    "docker",
                    "Failed to cleanup container {}: {}",
                    crate::truncate_utf8(&container_id, 12),
                    cleanup_err
                );
            }
            return Err(anyhow::anyhow!("Container execution failed: {}", e));
        }
        Err(_) => {
            // Timeout — kill the container
            app_warn!(
                "sandbox",
                "docker",
                "Sandbox container timed out after {}s, killing...",
                timeout_secs
            );
            let _ = docker.stop_container(&container_id, None).await;
            (-1, true)
        }
    };

    // Collect logs
    let (stdout, stderr) = collect_logs(&docker, &container_id).await?;

    // Cleanup container
    if let Err(e) = cleanup_container(&docker, &container_id).await {
        app_warn!(
            "sandbox",
            "docker",
            "Failed to cleanup container {}: {}",
            crate::truncate_utf8(&container_id, 12),
            e
        );
    }

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
        condition: "not-running".to_string(),
    };

    let mut stream = docker.wait_container(container_id, Some(options));
    if let Some(result) = stream.next().await {
        return result
            .map(|response| response.status_code)
            .map_err(|e| anyhow::anyhow!("Wait error: {}", e));
    }

    Err(anyhow::anyhow!("Container wait stream ended unexpectedly"))
}

/// Collect stdout and stderr logs from a container.
async fn collect_logs(docker: &Docker, container_id: &str) -> Result<(String, String)> {
    let options = LogsOptions {
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
                app_warn!("sandbox", "docker", "Error reading container logs: {}", e);
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
    app_info!(
        "sandbox",
        "docker",
        "Sandbox container removed: {}",
        crate::truncate_utf8(&container_id, 12)
    );
    Ok(())
}

// ── Tauri Commands ────────────────────────────────────────────────

pub async fn get_sandbox_config() -> Result<SandboxConfig, String> {
    load_sandbox_config().map_err(|e| e.to_string())
}

pub async fn set_sandbox_config(config: SandboxConfig) -> Result<(), String> {
    save_sandbox_config(&config).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub installed: bool,
    pub running: bool,
}

pub async fn check_sandbox_available() -> DockerStatus {
    // Check if docker CLI exists
    let cli_installed = Command::new("docker")
        .args(["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if !cli_installed {
        return DockerStatus {
            installed: false,
            running: false,
        };
    }

    // Check if daemon is running
    let daemon_running = check_docker_available().await;

    DockerStatus {
        installed: true,
        running: daemon_running,
    }
}
