//! 沙箱配置面与 kernel trampoline。Docker/WSL 执行机器在 `ha-vcs`。
//!
//! kernel 持有：配置 wire 类型与持久化（`sandbox.json`）、状态 wire 类型
//! （`DockerStatus`/`DockerBackend`）、以及三个 hook trampoline
//! （`check_sandbox_available` / `ensure_sandbox_available` /
//! `exec_in_sandbox_mode`）。调用方（tools::exec / cron / settings_reset /
//! system_prompt / 壳层）签名与路径不变；未接线语义见
//! [`crate::vcs_hooks`]（ensure/exec fail-closed，check 返不可用状态）。

use anyhow::Result;
use serde::{Deserialize, Serialize};

const DEFAULT_SANDBOX_IMAGE: &str = "debian:bookworm-slim";

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

// ── 壳层薄封装 ────────────────────────────────────────────────────

pub async fn get_sandbox_config() -> Result<SandboxConfig, String> {
    load_sandbox_config().map_err(|e| e.to_string())
}

pub async fn set_sandbox_config(config: SandboxConfig) -> Result<(), String> {
    save_sandbox_config(&config).map_err(|e| e.to_string())
}

// ── 状态 wire 类型 ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerBackend {
    Native,
    Wsl,
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

// ── ha-vcs hook trampolines ───────────────────────────────────────

/// 探测沙箱后端状态（实现在 ha-vcs）。未接线返「未安装 / 未运行」的状态
/// 对象——只读展示面，不 gate 执行。
pub async fn check_sandbox_available() -> DockerStatus {
    match crate::vcs_hooks::vcs_hooks() {
        Some(hooks) => (hooks.sandbox_check)().await,
        None => DockerStatus {
            installed: false,
            running: false,
            host_os: host_os().to_string(),
            backend: None,
            wsl_installed: None,
            wsl_distribution_installed: None,
            wsl_docker_installed: None,
        },
    }
}

/// 沙箱可用性预检（实现在 ha-vcs）。未接线即 `Err`——与「Docker 不可用」
/// 同一 fail-closed 语义，调用方（exec 工具 / cron 执行器）绝不回落宿主机。
pub async fn ensure_sandbox_available() -> Result<()> {
    match crate::vcs_hooks::vcs_hooks() {
        Some(hooks) => (hooks.sandbox_ensure)().await,
        None => Err(anyhow::anyhow!(
            "SandboxUnavailable: sandbox backend is not wired in this process (ha_vcs::wire() missing)"
        )),
    }
}

/// 在选定沙箱模式内执行命令（实现在 ha-vcs）。未接线即 `Err`（fail-closed，
/// 绝不回落宿主机执行）。
pub async fn exec_in_sandbox_mode(
    command: &str,
    cwd: &str,
    env: Option<&serde_json::Map<String, serde_json::Value>>,
    config: &SandboxConfig,
    timeout_secs: u64,
    cancellation_token: Option<tokio_util::sync::CancellationToken>,
    mode: crate::permission::SandboxMode,
) -> Result<SandboxResult> {
    let Some(hooks) = crate::vcs_hooks::vcs_hooks() else {
        return Err(anyhow::anyhow!(
            "SandboxUnavailable: sandbox backend is not wired in this process (ha_vcs::wire() missing)"
        ));
    };
    (hooks.sandbox_exec_mode)(crate::vcs_hooks::SandboxExecRequest {
        command: command.to_string(),
        cwd: cwd.to_string(),
        env: env.cloned(),
        config: config.clone(),
        timeout_secs,
        cancellation_token,
        mode,
    })
    .await
}
