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
use std::error::Error as StdError;
use std::io::ErrorKind;
use std::path::Path;

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
    /// Docker 连接失败的具体分类（见 [`DockerConnectionErrorKind`]）。
    /// 只在 daemon 不可达时有值；已 running 时恒 `None`。
    #[serde(default)]
    pub connection_error: Option<DockerConnectionErrorKind>,
    /// 本进程是否运行在容器化部署（`HA_DEPLOYMENT=docker`）里。
    /// 容器部署只支持 `isolated` 沙箱模式（`bind` 模式需宿主机路径访问）。
    #[serde(default)]
    pub containerized: bool,
    /// 只允许 `isolated` 沙箱模式（当前 = `containerized`）。为独立字段是
    /// 因为将来可能其它路径也触发这个限制，且它是 GUI/审批面消费的读侧
    /// 语义，不宜让消费者去推 `containerized`。
    #[serde(default)]
    pub isolated_mode_only: bool,
}

/// Docker 连接失败分类，`DockerStatus.connection_error` 的取值。
///
/// 消费者：`ensure_sandbox_available` 用它把 Docker 守护 dead 的技术错误翻译成
/// 人类可读的原因（socket 缺失 → 提示挂载；permission_denied → 提示 GID；等等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerConnectionErrorKind {
    /// Docker socket 文件不存在（`ENOENT`）。容器部署下通常是没把宿主机
    /// Docker socket 挂进 Hope Agent 容器。
    SocketMissing,
    /// socket 存在但拒绝连接（`ECONNREFUSED`）或 daemon 已停。
    DaemonUnreachable,
    /// 权限被拒（`EACCES` / `EPERM`）——容器附加组没匹配 socket GID。
    PermissionDenied,
    /// 客户端侧其它错误（如握手失败）。
    ClientError,
}

// ── ha-vcs hook trampolines ───────────────────────────────────────

/// 探测沙箱后端状态（实现在 ha-vcs）。未接线返「未安装 / 未运行」的状态
/// 对象——只读展示面，不 gate 执行。
pub async fn check_sandbox_available() -> DockerStatus {
    match crate::vcs_hooks::vcs_hooks() {
        Some(hooks) => (hooks.sandbox_check)().await,
        None => {
            // 少了 `ha_vcs::wire()` → 前端只看到 installed=false / running=false
            // 与 containerized=deployment_is_docker() 的合成状态，UI 会渲染
            // 「container isolated only」提示指错方向。这里 emit 一次 warn
            // （用 OnceLock 去重，避免每次前端 poll 都刷屏）——ops grep
            // "sandbox not wired" 就能立刻找到根因，比追 UI 提示快得多。
            static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
            WARNED.get_or_init(|| {
                app_warn!(
                    "sandbox",
                    "wire",
                    "check_sandbox_available fallback triggered — ha_vcs::wire() has not been called in this process; UI status is a placeholder"
                );
            });
            DockerStatus {
                installed: false,
                running: false,
                host_os: host_os().to_string(),
                backend: None,
                wsl_installed: None,
                wsl_distribution_installed: None,
                wsl_docker_installed: None,
                connection_error: None,
                containerized: deployment_is_docker(),
                isolated_mode_only: deployment_is_docker(),
            }
        }
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

// ── Deployment / mode 判定（纯谓词） ─────────────────────────────

/// 本进程是否运行在容器化部署里（`HA_DEPLOYMENT=docker`，见
/// `docs/deployment/docker.md`）。纯环境变量读取，无 IO。
pub fn deployment_is_docker() -> bool {
    std::env::var("HA_DEPLOYMENT")
        .map(|value| value.eq_ignore_ascii_case("docker"))
        .unwrap_or(false)
}

/// 容器化部署下**只**允许 `isolated` 沙箱模式——`bind` 模式需要宿主机路径
/// 权限（容器里的 `/host` 不是真实的宿主文件系统），`disabled` / `network`
/// 等其它模式在容器 sandbox 语义下也无意义。
///
/// 纯谓词：唯一入参是模式枚举。
pub fn container_sandbox_mode_supported(mode: crate::permission::SandboxMode) -> bool {
    matches!(mode, crate::permission::SandboxMode::Isolated)
}

// ── 顶层入口 ──────────────────────────────────────────────────────

/// `ensure_sandbox_available` 之上再加一层「mode 是否被当前部署支持」的判定。
/// 容器化部署且传了非 `isolated` 时 `bail!`——`ensure_sandbox_available` 的
/// fail-closed 语义仍在，此处只是把「container + bind」的组合早失败。
///
/// **纯 trampoline**：先调 `ensure_sandbox_available()`（走 hook），再做纯谓词
/// 判断，不新增机器依赖。
pub async fn ensure_sandbox_available_for_mode(mode: crate::permission::SandboxMode) -> Result<()> {
    ensure_sandbox_available().await?;
    if deployment_is_docker() && !container_sandbox_mode_supported(mode) {
        anyhow::bail!(
            "SandboxUnavailable: container deployments currently support only isolated sandbox mode; requested {}",
            mode.as_str()
        );
    }
    Ok(())
}

pub fn docker_connection_error_from_io_kind(kind: ErrorKind) -> DockerConnectionErrorKind {
    match kind {
        ErrorKind::NotFound => DockerConnectionErrorKind::SocketMissing,
        ErrorKind::PermissionDenied => DockerConnectionErrorKind::PermissionDenied,
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::AddrNotAvailable
        | ErrorKind::BrokenPipe
        | ErrorKind::TimedOut => DockerConnectionErrorKind::DaemonUnreachable,
        _ => DockerConnectionErrorKind::ClientError,
    }
}

pub fn classify_docker_connection_error(
    error: &(dyn StdError + 'static),
) -> DockerConnectionErrorKind {
    let mut current = Some(error);
    while let Some(source) = current {
        if let Some(io_error) = source.downcast_ref::<std::io::Error>() {
            return docker_connection_error_from_io_kind(io_error.kind());
        }
        current = source.source();
    }

    // Some HTTP connector errors do not expose their nested io::Error through
    // `source()`. Keep the fallback deliberately narrow and return only a
    // stable category; the raw error may contain a credential-bearing
    // DOCKER_HOST and must never be returned to the UI or logs.
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("permission denied") || message.contains("os error 13") {
        DockerConnectionErrorKind::PermissionDenied
    } else if message.contains("no such file") || message.contains("os error 2") {
        DockerConnectionErrorKind::SocketMissing
    } else if message.contains("connection refused")
        || message.contains("connection reset")
        || message.contains("timed out")
    {
        DockerConnectionErrorKind::DaemonUnreachable
    } else {
        DockerConnectionErrorKind::ClientError
    }
}

pub fn validate_container_isolated_source(source: &Path) -> Result<()> {
    let data_root = crate::paths::root_dir()?.canonicalize().map_err(|e| {
        anyhow::anyhow!(
            "Cannot resolve Hope Agent data directory before preparing isolated sandbox: {e}"
        )
    })?;
    validate_container_isolated_source_against(source, &data_root)
}

pub fn validate_container_isolated_source_against(source: &Path, data_root: &Path) -> Result<()> {
    if source == data_root || data_root.starts_with(source) {
        anyhow::bail!(
            "SandboxUnavailable: refusing to copy the Hope Agent data root or one of its parent directories into an isolated container because it contains credentials and databases; select a project or explicit workspace directory"
        );
    }
    let credentials_path = data_root.join("credentials");
    let credentials = credentials_path.canonicalize().unwrap_or(credentials_path);
    if source.starts_with(&credentials) {
        anyhow::bail!(
            "Sandbox security: the Hope Agent credentials directory cannot be used as an isolated workspace"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::SandboxMode;
    use std::fs;
    use std::io::ErrorKind;

    #[test]
    fn docker_connection_io_errors_have_stable_categories() {
        assert_eq!(
            docker_connection_error_from_io_kind(ErrorKind::NotFound),
            DockerConnectionErrorKind::SocketMissing
        );
        assert_eq!(
            docker_connection_error_from_io_kind(ErrorKind::PermissionDenied),
            DockerConnectionErrorKind::PermissionDenied
        );
        assert_eq!(
            docker_connection_error_from_io_kind(ErrorKind::ConnectionRefused),
            DockerConnectionErrorKind::DaemonUnreachable
        );
        assert_eq!(
            docker_connection_error_from_io_kind(ErrorKind::InvalidInput),
            DockerConnectionErrorKind::ClientError
        );
    }

    #[test]
    fn container_deployment_supports_only_isolated_mode() {
        for mode in [
            SandboxMode::Off,
            SandboxMode::Standard,
            SandboxMode::Workspace,
            SandboxMode::Trusted,
        ] {
            assert!(!container_sandbox_mode_supported(mode));
        }
        assert!(container_sandbox_mode_supported(SandboxMode::Isolated));
    }

    #[test]
    fn container_archive_source_rejects_data_root_and_credentials() {
        let base = tempfile::tempdir().expect("base tempdir");
        let data_root_path = base.path().join("state");
        fs::create_dir_all(&data_root_path).expect("data root directory");
        let data_root = data_root_path.canonicalize().expect("canonical data root");
        let data_root_parent = base
            .path()
            .canonicalize()
            .expect("canonical data root parent");
        let credentials = data_root.join("credentials");
        let workspace = data_root.join("projects/demo/workspace");
        fs::create_dir_all(&credentials).expect("credentials directory");
        fs::create_dir_all(&workspace).expect("workspace directory");

        assert!(validate_container_isolated_source_against(&data_root, &data_root).is_err());
        assert!(validate_container_isolated_source_against(&data_root_parent, &data_root).is_err());
        assert!(validate_container_isolated_source_against(&credentials, &data_root).is_err());
        assert!(validate_container_isolated_source_against(&workspace, &data_root).is_ok());
    }

    #[test]
    fn docker_status_serializes_structured_diagnostics() {
        let status = DockerStatus {
            installed: true,
            running: false,
            host_os: "linux".to_string(),
            backend: Some(DockerBackend::Native),
            wsl_installed: None,
            wsl_distribution_installed: None,
            wsl_docker_installed: None,
            connection_error: Some(DockerConnectionErrorKind::PermissionDenied),
            containerized: true,
            isolated_mode_only: true,
        };
        let value = serde_json::to_value(status).expect("serialize Docker status");
        assert_eq!(value["connectionError"], "permission_denied");
        assert_eq!(value["containerized"], true);
        assert_eq!(value["isolatedModeOnly"], true);
    }
}
