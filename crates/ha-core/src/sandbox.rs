use anyhow::Result;
use bollard::models::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, LogsOptions, RemoveContainerOptions,
    WaitContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const DEFAULT_SANDBOX_IMAGE: &str = "debian:bookworm-slim";
const ISOLATED_COPY_MAX_BYTES: u64 = 512 * 1024 * 1024;
const ISOLATED_COPY_MAX_ENTRIES: u64 = 50_000;
const ISOLATED_COPY_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
    ".pytest_cache",
    "__pycache__",
];

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

pub fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
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
    "/run/docker",
];

/// Validate that a host path is safe to bind-mount into the sandbox.
fn validate_bind_mount(host_path: &std::path::Path) -> Result<()> {
    let canonical = host_path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("Cannot resolve path '{}': {}", host_path.display(), e))?;
    let path_str = canonical.to_string_lossy();

    // Block root filesystem mount
    if canonical == std::path::Path::new("/") || canonical.parent().is_none() {
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

/// Validate a canonical absolute path as interpreted by the WSL distribution.
///
/// Windows-side canonicalization cannot apply the Linux mount blocklist to WSL
/// UNC paths. `platform::path_to_wsl` resolves Linux-side symlinks first, then
/// this function enforces the same root/sensitive-path boundary on that result.
fn validate_wsl_bind_mount(wsl_path: &str) -> Result<()> {
    if !wsl_path.starts_with('/') {
        anyhow::bail!(
            "Sandbox security: WSL mount path must be absolute: {}",
            wsl_path
        );
    }
    if wsl_path == "/"
        || wsl_path.contains("//")
        || wsl_path
            .split('/')
            .any(|component| component == "." || component == "..")
    {
        anyhow::bail!(
            "Sandbox security: mounting WSL root or a non-canonical path is not allowed: {}",
            wsl_path
        );
    }
    for blocked in BLOCKED_MOUNT_PATHS {
        let is_blocked = wsl_path == *blocked
            || wsl_path
                .strip_prefix(blocked)
                .is_some_and(|suffix| suffix.starts_with('/'));
        if is_blocked {
            anyhow::bail!(
                "Sandbox security: mounting sensitive WSL path '{}' is not allowed",
                wsl_path
            );
        }
    }
    Ok(())
}

fn linux_path_is_same_or_descendant(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_wsl_docker_socket_mount(wsl_path: &str, socket_path: &str) -> Result<()> {
    // Reject mounting the socket itself, any directory containing it, or any
    // descendant of the socket path. The ancestor check is necessary for
    // rootless sockets such as /run/user/<uid>/docker.sock.
    if linux_path_is_same_or_descendant(socket_path, wsl_path)
        || linux_path_is_same_or_descendant(wsl_path, socket_path)
    {
        anyhow::bail!(
            "Sandbox security: WSL mount '{}' would expose the Docker socket",
            wsl_path
        );
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerBackend {
    Native,
    Wsl,
}

#[derive(Debug, Clone, Default)]
struct WslDockerProbe {
    wsl_installed: bool,
    distribution_installed: bool,
    docker_installed: bool,
    daemon_running: bool,
    local_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AvailableSandboxBackend {
    Native,
    Wsl { endpoint: String },
}

async fn command_succeeds(command: &mut Command) -> bool {
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(5), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

async fn native_docker_cli_installed() -> bool {
    let mut command = Command::new("docker");
    command.arg("--version");
    crate::platform::hide_console_tokio(&mut command);
    command_succeeds(&mut command).await
}

fn normalize_local_docker_endpoint(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let socket_path = raw.strip_prefix("unix://")?;
    if !socket_path.starts_with('/')
        || socket_path.contains("//")
        || socket_path.chars().any(char::is_control)
        || socket_path
            .split('/')
            .any(|component| component == "." || component == "..")
    {
        return None;
    }
    Some(format!("unix://{}", socket_path))
}

fn wsl_local_docker_command(endpoint: &str) -> Option<Command> {
    let mut command = crate::platform::wsl_command()?;
    // Prevent WSLENV-exported Docker variables from overriding the validated
    // local endpoint. Docker configuration and registry credentials remain
    // available; only daemon-selection/TLS variables are cleared.
    command.args([
        "--exec",
        "env",
        "-u",
        "DOCKER_CONTEXT",
        "-u",
        "DOCKER_HOST",
        "-u",
        "DOCKER_TLS_VERIFY",
        "-u",
        "DOCKER_CERT_PATH",
        "docker",
        "--host",
        endpoint,
    ]);
    Some(command)
}

async fn command_stdout(command: &mut Command) -> Option<String> {
    command.stderr(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

async fn configured_wsl_local_docker_endpoint() -> Option<String> {
    let mut command = crate::platform::wsl_command()?;
    // Reading context metadata does not contact the configured daemon. The
    // returned endpoint is still treated as untrusted and accepted only when
    // it is a local Unix socket.
    command.args([
        "--exec",
        "docker",
        "context",
        "inspect",
        "--format",
        "{{.Endpoints.docker.Host}}",
    ]);
    normalize_local_docker_endpoint(&command_stdout(&mut command).await?)
}

async fn canonicalize_wsl_docker_socket_path(endpoint: &str) -> Option<String> {
    let socket_path = endpoint.strip_prefix("unix://")?;
    let mut command = crate::platform::wsl_command()?;
    command.args(["--exec", "readlink", "-f", "--", socket_path]);
    let canonical = command_stdout(&mut command).await?;
    let canonical = canonical.trim();
    if !canonical.starts_with('/')
        || canonical.contains("//")
        || canonical.chars().any(char::is_control)
        || canonical
            .split('/')
            .any(|component| component == "." || component == "..")
    {
        return None;
    }
    Some(canonical.to_string())
}

async fn find_wsl_local_docker_endpoint() -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(endpoint) = configured_wsl_local_docker_endpoint().await {
        candidates.push(endpoint);
    }
    candidates.push("unix:///var/run/docker.sock".to_string());
    if let Some(uid) = wsl_numeric_id("-u").await {
        let rootless = format!("unix:///run/user/{}/docker.sock", uid);
        if !candidates.contains(&rootless) {
            candidates.push(rootless);
        }
    }

    for endpoint in candidates {
        let mut info = wsl_local_docker_command(&endpoint)?;
        info.args(["info", "--format", "{{.ServerVersion}}"]);
        if command_succeeds(&mut info).await {
            return Some(endpoint);
        }
    }
    None
}

async fn wsl_docker_probe() -> WslDockerProbe {
    let status = crate::platform::wsl_status().await;
    let mut probe = WslDockerProbe {
        wsl_installed: status.installed,
        distribution_installed: status.distribution_installed,
        ..Default::default()
    };
    if !status.distribution_installed {
        return probe;
    }

    let Some(mut version) = crate::platform::wsl_command() else {
        return probe;
    };
    version.args(["--exec", "docker", "--version"]);
    probe.docker_installed = command_succeeds(&mut version).await;
    if !probe.docker_installed {
        return probe;
    }

    probe.local_endpoint = find_wsl_local_docker_endpoint().await;
    probe.daemon_running = probe.local_endpoint.is_some();
    probe
}

async fn available_sandbox_backend() -> Option<AvailableSandboxBackend> {
    if check_docker_available().await {
        return Some(AvailableSandboxBackend::Native);
    }
    wsl_docker_probe()
        .await
        .local_endpoint
        .map(|endpoint| AvailableSandboxBackend::Wsl { endpoint })
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
async fn exec_in_native_docker(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
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
        "hope-agent-sandbox-{}",
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

    // Wait for container to finish. `timeout_secs = 0` disables the exec-level
    // timeout and lets Docker wait until the container exits naturally.
    let (exit_code, timed_out) = match wait_for_container_with_limits(
        &docker,
        &container_id,
        timeout_secs,
        cancellation_token,
    )
    .await
    {
        SandboxWaitOutcome::Exited(Ok(code)) => (code, false),
        SandboxWaitOutcome::Exited(Err(e)) => {
            app_warn!("sandbox", "docker", "Container wait error: {}", e);
            stop_and_cleanup_container(&docker, &container_id).await;
            return Err(anyhow::anyhow!("Container execution failed: {}", e));
        }
        SandboxWaitOutcome::TimedOut => {
            app_warn!(
                "sandbox",
                "docker",
                "Sandbox container timed out after {}s, killing...",
                timeout_secs
            );
            let _ = docker.stop_container(&container_id, None).await;
            (-1, true)
        }
        SandboxWaitOutcome::Cancelled => {
            app_warn!(
                "sandbox",
                "docker",
                "Sandbox container cancelled, killing {}...",
                crate::truncate_utf8(&container_id, 12)
            );
            let _ = docker.stop_container(&container_id, None).await;
            stop_and_cleanup_container(&docker, &container_id).await;
            return Err(anyhow::anyhow!("Sandbox execution cancelled"));
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

enum WslRunOutcome {
    Exited(std::io::Result<ExitStatus>),
    TimedOut,
    Cancelled,
}

async fn wsl_container_exists(endpoint: &str, container_name: &str) -> Option<bool> {
    let mut inspect = wsl_local_docker_command(endpoint)?;
    inspect.args(["container", "inspect", container_name]);
    inspect.stdout(Stdio::null()).stderr(Stdio::null());
    let inspect_status = tokio::time::timeout(Duration::from_secs(2), inspect.status())
        .await
        .ok()?
        .ok()?;
    if inspect_status.success() {
        return Some(true);
    }

    // `docker inspect` also fails when the daemon is unavailable. Confirm the
    // endpoint is responsive before treating a non-zero status as "not found".
    let mut info = wsl_local_docker_command(endpoint)?;
    info.arg("info");
    command_succeeds(&mut info).await.then_some(false)
}

async fn force_remove_wsl_container(endpoint: &str, container_name: &str) {
    // The docker client is terminated before this function is called, so it
    // cannot issue a new create request after cleanup. Retry briefly to cover a
    // create request that was already in flight when the client was killed.
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let Some(mut command) = wsl_local_docker_command(endpoint) else {
            return;
        };
        command.args(["rm", "--force", container_name]);
        command.stdout(Stdio::null()).stderr(Stdio::null());
        if matches!(
            tokio::time::timeout(Duration::from_secs(2), command.status()).await,
            Ok(Ok(status)) if status.success()
        ) {
            return;
        }
    }

    match wsl_container_exists(endpoint, container_name).await {
        Some(false) => {}
        Some(true) => app_warn!(
            "sandbox",
            "wsl_docker",
            "WSL sandbox container {} still exists after forced cleanup",
            container_name
        ),
        None => app_warn!(
            "sandbox",
            "wsl_docker",
            "Could not verify cleanup of WSL sandbox container {}",
            container_name
        ),
    }
}

async fn terminate_wsl_docker_client(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        app_warn!(
            "sandbox",
            "wsl_docker",
            "Timed out waiting for the WSL Docker client to terminate"
        );
    }
}

async fn wsl_numeric_id(flag: &str) -> Option<String> {
    let mut command = crate::platform::wsl_command()?;
    command.args(["--exec", "id", flag]);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    value.parse::<u32>().ok().map(|_| value.to_string())
}

async fn wsl_container_user() -> Option<String> {
    let (uid, gid) = tokio::join!(wsl_numeric_id("-u"), wsl_numeric_id("-g"));
    Some(format!("{}:{}", uid?, gid?))
}

async fn exec_in_wsl_docker(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
    docker_endpoint: &str,
) -> Result<SandboxResult> {
    let host_cwd = Path::new(cwd).canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "Cannot resolve sandbox working directory '{}': {}. Ensure the path exists.",
            cwd,
            e
        )
    })?;
    validate_bind_mount(&host_cwd)?;
    let wsl_cwd = crate::platform::path_to_wsl(&host_cwd)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Cannot translate sandbox working directory '{}' for WSL: {}",
                host_cwd.display(),
                e
            )
        })?
        .ok_or_else(|| anyhow::anyhow!("WSL path conversion is unavailable on this host"))?;
    validate_wsl_bind_mount(&wsl_cwd)?;
    let docker_socket_path = canonicalize_wsl_docker_socket_path(docker_endpoint)
        .await
        .ok_or_else(|| {
            anyhow::anyhow!("Cannot resolve the selected WSL Docker Unix socket safely")
        })?;
    validate_wsl_docker_socket_mount(&wsl_cwd, &docker_socket_path)?;

    let container_name = format!(
        "hope-agent-sandbox-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("tmp")
    );
    let Some(mut docker) = wsl_local_docker_command(docker_endpoint) else {
        anyhow::bail!("WSL Docker command is unavailable on this host");
    };
    let container_user = wsl_container_user().await;
    docker.args([
        "run",
        "--rm",
        "--name",
        &container_name,
        "--workdir",
        "/workspace",
        "--volume",
        &format!("{}:/workspace", wsl_cwd),
    ]);
    if let Some(user) = &container_user {
        docker.args(["--user", user]);
    }
    if config.read_only {
        docker.arg("--read-only");
    }
    docker.args(["--network", &config.network_mode]);
    if config.cap_drop_all {
        docker.args(["--cap-drop", "ALL"]);
    }
    if config.no_new_privileges {
        docker.args(["--security-opt", "no-new-privileges"]);
    }
    if let Some(limit) = config.pids_limit {
        docker.args(["--pids-limit", &limit.to_string()]);
    }
    if config.read_only {
        for tmpfs in &config.tmpfs {
            docker.args(["--tmpfs", tmpfs]);
        }
    }
    if let Some(limit) = config.memory_limit {
        docker.args(["--memory", &limit.to_string()]);
    }
    if let Some(limit) = config.cpu_limit {
        docker.args(["--cpus", &limit.to_string()]);
    }
    if let Some(env_map) = env {
        for value in sanitize_env(env_map) {
            docker.args(["--env", &value]);
        }
    }
    docker
        .arg(&config.image)
        .args(["sh", "-c", command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = docker
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to start Docker through WSL: {}", e))?;
    let stdout_task = child.stdout.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).await.map(|_| output)
        })
    });
    let stderr_task = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).await.map(|_| output)
        })
    });

    app_info!(
        "sandbox",
        "wsl_docker",
        "WSL sandbox container starting: {} (image: {}, read_only: {}, network: {}, cap_drop_all: {}, command: {})",
        container_name,
        config.image,
        config.read_only,
        config.network_mode,
        config.cap_drop_all,
        command
    );

    let outcome = match (timeout_secs, cancellation_token) {
        (0, None) => WslRunOutcome::Exited(child.wait().await),
        (0, Some(token)) => tokio::select! {
            result = child.wait() => WslRunOutcome::Exited(result),
            _ = token.cancelled() => WslRunOutcome::Cancelled,
        },
        (secs, None) => {
            let timer = tokio::time::sleep(Duration::from_secs(secs));
            tokio::pin!(timer);
            tokio::select! {
                result = child.wait() => WslRunOutcome::Exited(result),
                _ = &mut timer => WslRunOutcome::TimedOut,
            }
        }
        (secs, Some(token)) => {
            let timer = tokio::time::sleep(Duration::from_secs(secs));
            tokio::pin!(timer);
            tokio::select! {
                result = child.wait() => WslRunOutcome::Exited(result),
                _ = &mut timer => WslRunOutcome::TimedOut,
                _ = token.cancelled() => WslRunOutcome::Cancelled,
            }
        }
    };

    if matches!(outcome, WslRunOutcome::TimedOut | WslRunOutcome::Cancelled) {
        terminate_wsl_docker_client(&mut child).await;
        force_remove_wsl_container(docker_endpoint, &container_name).await;
    }

    let stdout = match stdout_task {
        Some(task) => task
            .await
            .map_err(|e| anyhow::anyhow!("WSL Docker stdout reader failed: {}", e))??,
        None => Vec::new(),
    };
    let stderr = match stderr_task {
        Some(task) => task
            .await
            .map_err(|e| anyhow::anyhow!("WSL Docker stderr reader failed: {}", e))??,
        None => Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    match outcome {
        WslRunOutcome::Exited(Ok(status)) => Ok(SandboxResult {
            stdout,
            stderr,
            exit_code: status.code().map(i64::from).unwrap_or(-1),
            timed_out: false,
        }),
        WslRunOutcome::Exited(Err(error)) => {
            Err(anyhow::anyhow!("WSL Docker execution failed: {}", error))
        }
        WslRunOutcome::TimedOut => Ok(SandboxResult {
            stdout,
            stderr,
            exit_code: -1,
            timed_out: true,
        }),
        WslRunOutcome::Cancelled => Err(anyhow::anyhow!("Sandbox execution cancelled")),
    }
}

/// Execute a command through the first responsive Docker-compatible backend.
/// On Windows this falls back to Docker Engine in the default WSL
/// distribution when no native Docker daemon is reachable.
pub async fn exec_in_sandbox(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
) -> Result<SandboxResult> {
    match available_sandbox_backend().await {
        Some(AvailableSandboxBackend::Native) => {
            exec_in_native_docker(command, cwd, env, config, timeout_secs, cancellation_token).await
        }
        Some(AvailableSandboxBackend::Wsl { endpoint }) => {
            exec_in_wsl_docker(
                command,
                cwd,
                env,
                config,
                timeout_secs,
                cancellation_token,
                &endpoint,
            )
            .await
        }
        None => Err(anyhow::anyhow!(
            "SandboxUnavailable: no responsive Docker daemon was found on the host or in WSL"
        )),
    }
}

/// Execute a command in the selected sandbox mode. `Isolated` runs against a
/// temporary copy of the working directory and deletes it afterwards; other
/// enabled modes use the configured direct Docker mount path.
pub async fn exec_in_sandbox_mode(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
    mode: crate::permission::SandboxMode,
) -> Result<SandboxResult> {
    if mode != crate::permission::SandboxMode::Isolated {
        return exec_in_sandbox(command, cwd, env, config, timeout_secs, cancellation_token).await;
    }

    let source = Path::new(cwd).canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "Cannot resolve isolated sandbox working directory '{}': {}. Ensure the path exists.",
            cwd,
            e
        )
    })?;
    validate_bind_mount(&source)?;
    let temp = tempfile::Builder::new()
        .prefix("hope-agent-sandbox-isolated-")
        .tempdir()
        .map_err(|e| anyhow::anyhow!("Failed to create isolated sandbox workspace: {}", e))?;
    prepare_isolated_workspace(
        source.clone(),
        temp.path().to_path_buf(),
        timeout_secs,
        cancellation_token.clone(),
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "Failed to prepare isolated sandbox workspace from '{}': {}",
            source.display(),
            e
        )
    })?;
    let isolated_cwd = temp.path().to_string_lossy().to_string();
    exec_in_sandbox(
        command,
        &isolated_cwd,
        env,
        config,
        timeout_secs,
        cancellation_token,
    )
    .await
}

async fn prepare_isolated_workspace(
    source: PathBuf,
    destination: PathBuf,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
) -> Result<()> {
    let limits = IsolatedCopyLimits {
        max_bytes: ISOLATED_COPY_MAX_BYTES,
        max_entries: ISOLATED_COPY_MAX_ENTRIES,
        deadline: (timeout_secs > 0).then(|| Instant::now() + Duration::from_secs(timeout_secs)),
        cancellation_token,
    };

    let stats = tokio::task::spawn_blocking(move || {
        let mut stats = IsolatedCopyStats::default();
        copy_dir_gitignore_aware_bounded(&source, &destination, &limits, &mut stats)?;
        Ok::<_, anyhow::Error>(stats)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Isolated sandbox workspace preparation panicked: {}", e))??;
    app_info!(
        "sandbox",
        "isolated",
        "Prepared isolated sandbox workspace: files={}, dirs={}, bytes={}",
        stats.files,
        stats.dirs,
        stats.bytes
    );
    Ok(())
}

struct IsolatedCopyLimits {
    max_bytes: u64,
    max_entries: u64,
    deadline: Option<Instant>,
    cancellation_token: Option<CancellationToken>,
}

#[derive(Default)]
struct IsolatedCopyStats {
    bytes: u64,
    entries: u64,
    files: u64,
    dirs: u64,
}

impl IsolatedCopyLimits {
    fn check(&self, stats: &IsolatedCopyStats) -> Result<()> {
        if let Some(token) = &self.cancellation_token {
            if token.is_cancelled() {
                anyhow::bail!("isolated sandbox workspace preparation cancelled");
            }
        }
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                anyhow::bail!("isolated sandbox workspace preparation timed out");
            }
        }
        if stats.entries > self.max_entries {
            anyhow::bail!(
                "isolated sandbox workspace has too many files/directories ({} > {}). Use workspace sandbox mode or run from a narrower working directory.",
                stats.entries,
                self.max_entries
            );
        }
        if stats.bytes > self.max_bytes {
            anyhow::bail!(
                "isolated sandbox workspace is too large to copy safely ({} MiB > {} MiB). Use workspace sandbox mode or run from a narrower working directory.",
                stats.bytes / 1024 / 1024,
                self.max_bytes / 1024 / 1024
            );
        }
        Ok(())
    }
}

fn should_skip_isolated_copy_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .map(|s| ISOLATED_COPY_EXCLUDED_DIRS.contains(&s))
        .unwrap_or(false)
}

fn find_git_root_for_ignore(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn copy_dir_gitignore_aware_bounded(
    src: &Path,
    dst: &Path,
    limits: &IsolatedCopyLimits,
    stats: &mut IsolatedCopyStats,
) -> Result<()> {
    limits.check(stats)?;
    std::fs::create_dir_all(dst)?;
    let source_root = src.to_path_buf();
    let filter_root = source_root.clone();
    let inside_git_repo = find_git_root_for_ignore(src).is_some();
    let walker = WalkBuilder::new(src)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(inside_git_repo)
        .git_exclude(inside_git_repo)
        .parents(inside_git_repo)
        .require_git(inside_git_repo)
        .follow_links(false)
        .filter_entry(move |entry| {
            if entry.path() == filter_root {
                return true;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && should_skip_isolated_copy_dir(entry.file_name()) {
                app_debug!(
                    "sandbox",
                    "isolated",
                    "Skipping generated/cache directory while preparing isolated sandbox: {}",
                    entry.path().display()
                );
                return false;
            }
            true
        })
        .build();

    for entry in walker {
        limits.check(stats)?;
        let entry = entry.map_err(|e| {
            anyhow::anyhow!(
                "Failed to walk isolated sandbox source '{}': {}",
                src.display(),
                e
            )
        })?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let src_path = entry.path();
        let rel_path = match src_path.strip_prefix(&source_root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel,
            _ => continue,
        };
        let dst_path = dst.join(rel_path);
        if file_type.is_symlink() {
            app_warn!(
                "sandbox",
                "isolated",
                "Skipping symlink while preparing isolated sandbox: {}",
                src_path.display()
            );
            continue;
        }
        if file_type.is_dir() {
            stats.entries = stats.entries.saturating_add(1);
            stats.dirs = stats.dirs.saturating_add(1);
            limits.check(stats)?;
            std::fs::create_dir_all(&dst_path)?;
        } else if file_type.is_file() {
            stats.entries = stats.entries.saturating_add(1);
            stats.files = stats.files.saturating_add(1);
            let file_size = std::fs::metadata(src_path)?.len();
            stats.bytes = stats.bytes.saturating_add(file_size);
            limits.check(stats)?;
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src_path, dst_path)?;
        } else {
            app_debug!(
                "sandbox",
                "isolated",
                "Skipping special file while preparing isolated sandbox: {}",
                src_path.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_copy_copies_regular_files_and_skips_generated_dirs() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        std::fs::write(source.path().join("keep.txt"), "keep").expect("write keep");
        std::fs::write(source.path().join(".env.example"), "documented=true")
            .expect("write hidden example");
        std::fs::create_dir_all(source.path().join("src")).expect("mkdir src");
        std::fs::write(source.path().join("src/lib.rs"), "fn main() {}").expect("write src");
        std::fs::create_dir_all(source.path().join("node_modules/pkg"))
            .expect("mkdir node_modules");
        std::fs::write(source.path().join("node_modules/pkg/index.js"), "skip")
            .expect("write skipped file");

        let limits = IsolatedCopyLimits {
            max_bytes: 1024,
            max_entries: 10,
            deadline: None,
            cancellation_token: None,
        };
        let mut stats = IsolatedCopyStats::default();
        copy_dir_gitignore_aware_bounded(source.path(), destination.path(), &limits, &mut stats)
            .expect("copy should succeed");

        assert!(destination.path().join("keep.txt").exists());
        assert!(destination.path().join(".env.example").exists());
        assert!(destination.path().join("src/lib.rs").exists());
        assert!(!destination.path().join("node_modules").exists());
    }

    #[test]
    fn isolated_copy_respects_gitignore_rules() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        std::fs::write(
            source.path().join(".gitignore"),
            "ignored.txt\nignored_dir/\n.env\n",
        )
        .expect("write gitignore");
        std::fs::write(source.path().join("keep.txt"), "keep").expect("write keep");
        std::fs::write(source.path().join("ignored.txt"), "ignore").expect("write ignored");
        std::fs::write(source.path().join(".env"), "SECRET=value").expect("write env");
        std::fs::create_dir_all(source.path().join("ignored_dir")).expect("mkdir ignored dir");
        std::fs::write(source.path().join("ignored_dir/file.txt"), "ignore")
            .expect("write ignored dir file");

        let limits = IsolatedCopyLimits {
            max_bytes: 1024,
            max_entries: 10,
            deadline: None,
            cancellation_token: None,
        };
        let mut stats = IsolatedCopyStats::default();
        copy_dir_gitignore_aware_bounded(source.path(), destination.path(), &limits, &mut stats)
            .expect("copy should succeed");

        assert!(destination.path().join(".gitignore").exists());
        assert!(destination.path().join("keep.txt").exists());
        assert!(!destination.path().join("ignored.txt").exists());
        assert!(!destination.path().join(".env").exists());
        assert!(!destination.path().join("ignored_dir").exists());
    }

    #[test]
    fn isolated_copy_uses_parent_gitignore_inside_git_repo() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        std::fs::create_dir(repo.path().join(".git")).expect("git marker");
        std::fs::write(repo.path().join(".gitignore"), "root_ignored.txt\n")
            .expect("write root gitignore");
        let source = repo.path().join("subdir");
        std::fs::create_dir(&source).expect("mkdir source");
        std::fs::write(source.join("root_ignored.txt"), "ignore").expect("write ignored");
        std::fs::write(source.join("keep.txt"), "keep").expect("write keep");
        let destination = tempfile::tempdir().expect("destination tempdir");

        let limits = IsolatedCopyLimits {
            max_bytes: 1024,
            max_entries: 10,
            deadline: None,
            cancellation_token: None,
        };
        let mut stats = IsolatedCopyStats::default();
        copy_dir_gitignore_aware_bounded(&source, destination.path(), &limits, &mut stats)
            .expect("copy should succeed");

        assert!(destination.path().join("keep.txt").exists());
        assert!(!destination.path().join("root_ignored.txt").exists());
    }

    #[test]
    fn isolated_copy_does_not_apply_parent_gitignore_outside_git_repo() {
        let parent = tempfile::tempdir().expect("parent tempdir");
        std::fs::write(parent.path().join(".gitignore"), "parent_ignored.txt\n")
            .expect("write parent gitignore");
        let source = parent.path().join("child");
        std::fs::create_dir(&source).expect("mkdir source");
        std::fs::write(source.join("parent_ignored.txt"), "keep").expect("write file");
        let destination = tempfile::tempdir().expect("destination tempdir");

        let limits = IsolatedCopyLimits {
            max_bytes: 1024,
            max_entries: 10,
            deadline: None,
            cancellation_token: None,
        };
        let mut stats = IsolatedCopyStats::default();
        copy_dir_gitignore_aware_bounded(&source, destination.path(), &limits, &mut stats)
            .expect("copy should succeed");

        assert!(destination.path().join("parent_ignored.txt").exists());
    }

    #[test]
    fn isolated_copy_enforces_size_limit() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        std::fs::write(source.path().join("too-big.txt"), "too big").expect("write file");

        let limits = IsolatedCopyLimits {
            max_bytes: 3,
            max_entries: 10,
            deadline: None,
            cancellation_token: None,
        };
        let mut stats = IsolatedCopyStats::default();
        let err = copy_dir_gitignore_aware_bounded(
            source.path(),
            destination.path(),
            &limits,
            &mut stats,
        )
        .expect_err("copy should fail on size limit");

        assert!(err.to_string().contains("too large to copy safely"));
    }

    #[test]
    fn isolated_copy_honors_cancellation() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        std::fs::write(source.path().join("file.txt"), "content").expect("write file");
        let cancellation_token = CancellationToken::new();
        cancellation_token.cancel();

        let limits = IsolatedCopyLimits {
            max_bytes: 1024,
            max_entries: 10,
            deadline: None,
            cancellation_token: Some(cancellation_token),
        };
        let mut stats = IsolatedCopyStats::default();
        let err = copy_dir_gitignore_aware_bounded(
            source.path(),
            destination.path(),
            &limits,
            &mut stats,
        )
        .expect_err("copy should fail when cancelled");

        assert!(err.to_string().contains("preparation cancelled"));
    }
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

enum SandboxWaitOutcome {
    Exited(Result<i64>),
    TimedOut,
    Cancelled,
}

async fn wait_for_container_with_limits(
    docker: &Docker,
    container_id: &str,
    timeout_secs: u64,
    cancellation_token: Option<CancellationToken>,
) -> SandboxWaitOutcome {
    match (timeout_secs, cancellation_token) {
        (0, None) => SandboxWaitOutcome::Exited(wait_for_container(docker, container_id).await),
        (0, Some(token)) => {
            tokio::select! {
                result = wait_for_container(docker, container_id) => SandboxWaitOutcome::Exited(result),
                _ = token.cancelled() => SandboxWaitOutcome::Cancelled,
            }
        }
        (secs, None) => {
            match tokio::time::timeout(
                std::time::Duration::from_secs(secs),
                wait_for_container(docker, container_id),
            )
            .await
            {
                Ok(result) => SandboxWaitOutcome::Exited(result),
                Err(_) => SandboxWaitOutcome::TimedOut,
            }
        }
        (secs, Some(token)) => {
            let timer = tokio::time::sleep(std::time::Duration::from_secs(secs));
            tokio::pin!(timer);
            tokio::select! {
                result = wait_for_container(docker, container_id) => SandboxWaitOutcome::Exited(result),
                _ = &mut timer => SandboxWaitOutcome::TimedOut,
                _ = token.cancelled() => SandboxWaitOutcome::Cancelled,
            }
        }
    }
}

async fn stop_and_cleanup_container(docker: &Docker, container_id: &str) {
    if let Err(stop_err) = docker.stop_container(container_id, None).await {
        app_warn!(
            "sandbox",
            "docker",
            "Failed to stop container {}: {}",
            crate::truncate_utf8(container_id, 12),
            stop_err
        );
    }
    if let Err(cleanup_err) = cleanup_container(docker, container_id).await {
        app_warn!(
            "sandbox",
            "docker",
            "Failed to cleanup container {}: {}",
            crate::truncate_utf8(container_id, 12),
            cleanup_err
        );
    }
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
    pub host_os: String,
    #[serde(default)]
    pub backend: Option<DockerBackend>,
    #[serde(default)]
    pub wsl_installed: Option<bool>,
    #[serde(default)]
    pub wsl_distribution_installed: Option<bool>,
    #[serde(default)]
    pub wsl_docker_installed: Option<bool>,
}

pub async fn check_sandbox_available() -> DockerStatus {
    let (native_cli_installed, native_daemon_running) =
        tokio::join!(native_docker_cli_installed(), check_docker_available());
    // Do not wake a stopped WSL VM merely to enrich status when the preferred
    // native backend is already healthy. WSL probing is the Windows fallback.
    let wsl = if native_daemon_running {
        None
    } else {
        Some(wsl_docker_probe().await)
    };
    let backend = if native_daemon_running {
        Some(DockerBackend::Native)
    } else if wsl.as_ref().is_some_and(|probe| probe.daemon_running) {
        Some(DockerBackend::Wsl)
    } else if native_cli_installed {
        Some(DockerBackend::Native)
    } else if wsl.as_ref().is_some_and(|probe| probe.docker_installed) {
        Some(DockerBackend::Wsl)
    } else {
        None
    };
    let wsl_daemon_running = wsl.as_ref().is_some_and(|probe| probe.daemon_running);
    let wsl_docker_installed = wsl.as_ref().is_some_and(|probe| probe.docker_installed);

    DockerStatus {
        installed: native_cli_installed || native_daemon_running || wsl_docker_installed,
        running: native_daemon_running || wsl_daemon_running,
        host_os: host_os().to_string(),
        backend,
        wsl_installed: wsl.as_ref().map(|probe| probe.wsl_installed),
        wsl_distribution_installed: wsl.as_ref().map(|probe| probe.distribution_installed),
        wsl_docker_installed: wsl.as_ref().map(|probe| probe.docker_installed),
    }
}

pub async fn ensure_sandbox_available() -> Result<()> {
    let status = check_sandbox_available().await;
    if status.installed && status.running {
        return Ok(());
    }
    let reason = if !status.installed
        && status.host_os == "windows"
        && status.wsl_distribution_installed == Some(true)
        && status.wsl_docker_installed != Some(true)
    {
        "WSL is available, but Docker Engine is not installed in its default distribution. Install Docker Engine in WSL before using sandbox mode.".to_string()
    } else if !status.installed {
        format!(
            "Docker is not installed on this {} host. Configure Docker before using sandbox mode.",
            status.host_os
        )
    } else if status.backend == Some(DockerBackend::Wsl) {
        "Docker Engine is installed in WSL but its daemon is not running. Start Docker in WSL and retry.".to_string()
    } else {
        format!(
            "Docker is installed on this {} host but the daemon is not running. Start Docker and retry.",
            status.host_os
        )
    };
    Err(anyhow::anyhow!("SandboxUnavailable: {}", reason))
}

#[cfg(test)]
mod wsl_security_tests {
    use super::{
        normalize_local_docker_endpoint, validate_wsl_bind_mount, validate_wsl_docker_socket_mount,
    };

    #[test]
    fn wsl_mount_validation_blocks_linux_sensitive_paths() {
        for path in [
            "/",
            "/etc",
            "/etc/ssl",
            "/var/run/docker",
            "/var/run/docker/plugins",
            "/run/docker",
            "/run/docker.sock",
            "//etc",
            "/mnt/c/../Windows",
            "mnt/c/workspace",
        ] {
            assert!(
                validate_wsl_bind_mount(path).is_err(),
                "expected WSL mount path to be rejected: {path}"
            );
        }

        for path in ["/mnt/c/workspace", "/home/user/project", "/etc-project"] {
            assert!(
                validate_wsl_bind_mount(path).is_ok(),
                "expected WSL mount path to be accepted: {path}"
            );
        }
    }

    #[test]
    fn wsl_docker_endpoint_accepts_only_absolute_unix_sockets() {
        assert_eq!(
            normalize_local_docker_endpoint("unix:///var/run/docker.sock\n").as_deref(),
            Some("unix:///var/run/docker.sock")
        );
        assert_eq!(
            normalize_local_docker_endpoint("unix:///run/user/1000/docker.sock").as_deref(),
            Some("unix:///run/user/1000/docker.sock")
        );

        for endpoint in [
            "ssh://docker@example.com",
            "tcp://127.0.0.1:2375",
            "npipe:////./pipe/docker_engine",
            "unix://relative/docker.sock",
            "unix:////run/docker.sock",
            "unix:///run/../docker.sock",
            "unix:///run/docker.sock\nssh://example.com",
        ] {
            assert_eq!(
                normalize_local_docker_endpoint(endpoint),
                None,
                "expected Docker endpoint to be rejected: {endpoint}"
            );
        }
    }

    #[test]
    fn wsl_mount_validation_blocks_the_selected_docker_socket() {
        let socket_path = "/run/user/1000/docker.sock";
        for path in [
            "/run",
            "/run/user",
            "/run/user/1000",
            "/run/user/1000/docker.sock",
        ] {
            assert!(
                validate_wsl_docker_socket_mount(path, socket_path).is_err(),
                "expected Docker socket exposure to be rejected: {path}"
            );
        }
        assert!(validate_wsl_docker_socket_mount("/home/user/project", socket_path).is_ok());
        assert!(validate_wsl_docker_socket_mount("/run/user/1001/project", socket_path).is_ok());
        assert!(validate_wsl_docker_socket_mount("/run", "/run/docker.sock").is_err());
    }
}
