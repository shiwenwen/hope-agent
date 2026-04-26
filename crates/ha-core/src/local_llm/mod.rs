//! Local LLM helper — hardware detection, Ollama lifecycle, and model
//! installation glue used by the model-config "local LLM assistant" card.
//!
//! Outbound HTTPS to `ollama.com/install.sh` goes through `security::ssrf`
//! + `provider::proxy::apply_proxy_for_url` like every other public-internet
//! hop in the codebase. Loopback Ollama traffic (`127.0.0.1:11434`) is
//! recognized by `should_bypass_proxy` and bypasses the user's HTTP proxy
//! automatically — same convention as `provider/proxy.rs` and the Docker
//! integration.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::provider::{
    upsert_known_local_provider_model, ModelConfig, ProviderConfig, ThinkingStyle,
};
#[cfg(unix)]
use crate::security::ssrf::{check_url, SsrfPolicy};

pub mod types;
pub use types::*;

pub use crate::provider::LOCAL_OLLAMA_BASE_URL as OLLAMA_BASE_URL;
#[cfg(unix)]
const OLLAMA_INSTALL_URL: &str = "https://ollama.com/install.sh";
const PROVIDER_SOURCE: &str = "local-llm-wizard";
const OLLAMA_PROVIDER_NAME: &str = "Ollama (local)";
const RECOMMENDATION_BUDGET_PERCENT: u64 = 60;
const START_OLLAMA_TIMEOUT_SECS: u64 = 30;
const OLLAMA_BINARY_CHECK_TIMEOUT_SECS: u64 = 3;
#[cfg(unix)]
const INSTALL_PHASE_DOWNLOAD: &str = "download-installer";
#[cfg(unix)]
const INSTALL_PHASE_AUTHORIZE: &str = "authorize";
#[cfg(unix)]
const INSTALL_PHASE_INSTALL: &str = "install-ollama";
/// Hard ceiling on a single NDJSON line during `/api/pull`. Ollama's frames
/// stay well under 1 KiB; this bound only fires on a malicious or broken
/// peer that streams without newlines.
const MAX_PULL_LINE_BYTES: usize = 1 << 20;

// ── Hardware detection ────────────────────────────────────────────

/// OS, total RAM and dGPU don't change while the process is alive — caching
/// them lets us re-run `detect_hardware()` on every focus event for free.
struct StaticHardware {
    os: String,
    total_memory_mb: u64,
    gpu: Option<GpuInfo>,
}

fn static_hardware() -> &'static StaticHardware {
    static CACHE: OnceLock<StaticHardware> = OnceLock::new();
    CACHE.get_or_init(|| {
        use sysinfo::{MemoryRefreshKind, RefreshKind, System};
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        sys.refresh_memory();
        StaticHardware {
            os: std::env::consts::OS.to_string(),
            total_memory_mb: sys.total_memory() / (1024 * 1024),
            gpu: crate::platform::detect_dedicated_gpu().map(|g| GpuInfo {
                name: g.name,
                vram_mb: g.vram_mb,
            }),
        }
    })
}

/// Read system memory + GPU and pick the recommendation budget.
pub fn detect_hardware() -> HardwareInfo {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};

    let s = static_hardware();
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    sys.refresh_memory();
    let available_memory_mb = sys.available_memory() / (1024 * 1024);

    // macOS unified memory: don't double-count the integrated GPU as a
    // separate adapter even if `detect_dedicated_gpu()` were to fire.
    let (budget_source, base_mb) = if s.os == "macos" {
        (BudgetSource::UnifiedMemory, s.total_memory_mb)
    } else if let Some(GpuInfo {
        vram_mb: Some(vram),
        ..
    }) = s.gpu.as_ref().cloned()
    {
        (BudgetSource::DedicatedVram, vram)
    } else {
        (BudgetSource::SystemMemory, s.total_memory_mb)
    };

    let budget_mb = recommendation_budget_mb(base_mb);

    HardwareInfo {
        os: s.os.clone(),
        total_memory_mb: s.total_memory_mb,
        available_memory_mb,
        gpu: s.gpu.clone(),
        budget_source,
        budget_mb,
    }
}

fn recommendation_budget_mb(base_mb: u64) -> u64 {
    // Use 60% of the chosen axis, minus a 1 GiB buffer for Ollama runtime and
    // KV-cache fluctuation. This keeps recommendations useful without letting
    // a local model crowd out the rest of the desktop.
    base_mb
        .saturating_mul(RECOMMENDATION_BUDGET_PERCENT)
        .saturating_div(100)
        .saturating_sub(1024)
}

/// Walk the catalog (descending size) and return the first model that fits
/// in the hardware budget. Smaller alternatives are returned for UI override.
pub fn recommend_model(hardware: &HardwareInfo) -> ModelRecommendation {
    let alternatives: Vec<ModelCandidate> = model_catalog()
        .into_iter()
        .filter(|c| c.size_mb <= hardware.budget_mb)
        .collect();
    let recommended = alternatives.first().cloned();

    let reason = match (recommended.as_ref(), hardware.budget_source) {
        (None, _) => RecommendationReason::Insufficient,
        (_, BudgetSource::UnifiedMemory) => RecommendationReason::UnifiedMemory,
        (_, BudgetSource::DedicatedVram) => RecommendationReason::Dgpu,
        (_, BudgetSource::SystemMemory) => RecommendationReason::RamFallback,
    };

    ModelRecommendation {
        hardware: hardware.clone(),
        recommended,
        alternatives,
        reason,
    }
}

// ── Ollama detection ──────────────────────────────────────────────

/// Probe Ollama: is the binary present, and is the daemon answering?
pub async fn detect_ollama() -> OllamaStatus {
    let running = ping_ollama().await;
    let installed = if running {
        true
    } else {
        usable_ollama_binary().await.is_some()
    };
    let phase = match (installed, running) {
        (_, true) => OllamaPhase::Running,
        (true, false) => OllamaPhase::Installed,
        (false, false) => OllamaPhase::NotInstalled,
    };

    OllamaStatus {
        phase,
        base_url: OLLAMA_BASE_URL.to_string(),
        install_script_supported: cfg!(unix),
    }
}

async fn locate_ollama_binaries() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(from_path) = tokio::task::spawn_blocking(|| which::which("ollama"))
        .await
        .ok()
        .and_then(|res| res.ok())
    {
        candidates.push(from_path);
    }

    #[cfg(target_os = "macos")]
    {
        let app_binary = PathBuf::from("/Applications/Ollama.app/Contents/Resources/ollama");
        if app_binary.is_file() && !candidates.iter().any(|path| path == &app_binary) {
            candidates.push(app_binary);
        }
    }

    candidates
}

async fn usable_ollama_binary() -> Option<PathBuf> {
    for path in locate_ollama_binaries().await {
        if ollama_binary_responds(&path).await {
            return Some(path);
        }
        app_warn!(
            "local_llm",
            "detect_ollama",
            "found ollama binary at {} but `ollama --version` failed",
            path.display()
        );
    }
    None
}

async fn ollama_binary_responds(path: &Path) -> bool {
    use std::process::Stdio;

    let status = tokio::process::Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    matches!(
        tokio::time::timeout(Duration::from_secs(OLLAMA_BINARY_CHECK_TIMEOUT_SECS), status).await,
        Ok(Ok(status)) if status.success()
    )
}

/// Cached 1-second-timeout, no-proxy reqwest client used only for the
/// `/api/tags` liveness probe. Built once per process.
fn ping_client() -> &'static reqwest::Client {
    static CACHE: OnceLock<reqwest::Client> = OnceLock::new();
    CACHE.get_or_init(|| {
        crate::provider::apply_proxy_for_url(
            reqwest::Client::builder().timeout(Duration::from_secs(1)),
            OLLAMA_BASE_URL,
        )
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
    })
}

async fn ping_ollama() -> bool {
    ping_client()
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
struct TagModel {
    name: Option<String>,
    model: Option<String>,
}

/// Return the model names currently present in the local Ollama store.
/// An unreachable daemon is treated as an empty list so callers can still
/// render their catalog while surfacing daemon status separately.
pub async fn list_ollama_model_names() -> Result<Vec<String>> {
    let resp = ping_client()
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .await
        .context("GET /api/tags")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Ollama /api/tags returned {status}: {body}"));
    }
    let tags = resp
        .json::<TagsResponse>()
        .await
        .context("parse /api/tags")?;
    Ok(tags
        .models
        .into_iter()
        .filter_map(|m| m.model.or(m.name))
        .collect())
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    version: String,
}

/// Return the local Ollama daemon version, when it is reachable.
pub async fn detect_ollama_version() -> Result<Option<String>> {
    let resp = ping_client()
        .get(format!("{OLLAMA_BASE_URL}/api/version"))
        .send()
        .await
        .context("GET /api/version")?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    Ok(Some(
        resp.json::<VersionResponse>()
            .await
            .context("parse /api/version")?
            .version,
    ))
}

// ── Ollama lifecycle ──────────────────────────────────────────────

/// Spawn Ollama detached and wait for the HTTP API to answer. Idempotent — if
/// the daemon already responds, returns Ok early.
pub async fn start_ollama() -> Result<()> {
    if ping_ollama().await {
        app_info!("local_llm", "start_ollama", "already running");
        return Ok(());
    }
    let binary = usable_ollama_binary()
        .await
        .ok_or_else(|| anyhow!("Ollama is not installed or the installed binary is not usable"))?;

    spawn_ollama_serve(&binary)?;

    let deadline = std::time::Instant::now() + Duration::from_secs(START_OLLAMA_TIMEOUT_SECS);
    while std::time::Instant::now() < deadline {
        if ping_ollama().await {
            app_info!("local_llm", "start_ollama", "ready");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(anyhow!(
        "Ollama did not respond on {OLLAMA_BASE_URL}/api/tags within {START_OLLAMA_TIMEOUT_SECS}s"
    ))
}

#[cfg(target_os = "macos")]
fn spawn_ollama_serve(binary: &Path) -> Result<()> {
    use std::process::{Command, Stdio};

    if Path::new("/Applications/Ollama.app").is_dir() {
        Command::new("open")
            .arg("-a")
            .arg("Ollama")
            .arg("--args")
            .arg("hidden")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("open Ollama.app")?;
        return Ok(());
    }

    Command::new(binary)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn `ollama serve`")?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn spawn_ollama_serve(binary: &Path) -> Result<()> {
    use std::process::{Command, Stdio};
    Command::new(binary)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn `ollama serve`")?;
    Ok(())
}

#[cfg(windows)]
fn spawn_ollama_serve(binary: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(binary)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .context("spawn `ollama serve`")?;
    Ok(())
}

/// Run the upstream Ollama install script with OS-level administrator
/// authorization and stream progress to `on_progress`. Unix only — Windows
/// users are routed to the download page in the UI, see
/// [`OllamaStatus::install_script_supported`].
#[cfg(unix)]
pub async fn install_ollama_via_script_cancellable<F>(
    on_progress: F,
    cancel_token: CancellationToken,
) -> Result<()>
where
    F: Fn(&InstallScriptProgress) + Send + Sync + 'static,
{
    let emit: InstallProgressEmitter = Arc::new(on_progress);
    let script = download_ollama_install_script(&emit).await?;
    if cancel_token.is_cancelled() {
        return Err(anyhow!("Ollama install was cancelled"));
    }
    run_ollama_install_script_authorized(&script, &emit, cancel_token).await?;

    emit_install_progress(&emit, InstallScriptKind::Step, "done");
    app_info!("local_llm", "install_ollama", "install.sh succeeded");
    Ok(())
}

#[cfg(unix)]
type InstallProgressEmitter = Arc<dyn Fn(&InstallScriptProgress) + Send + Sync + 'static>;

#[cfg(unix)]
const INSTALL_STARTED_MARKER: &str = "__HOPE_AGENT_OLLAMA_INSTALL_STARTED__";

#[cfg(unix)]
fn emit_install_progress(
    emit: &InstallProgressEmitter,
    kind: InstallScriptKind,
    message: impl Into<String>,
) {
    emit(&InstallScriptProgress {
        kind,
        message: message.into(),
    });
}

#[cfg(unix)]
async fn download_ollama_install_script(emit: &InstallProgressEmitter) -> Result<String> {
    // Public HTTPS — must pass through SSRF + global proxy like every
    // other outbound hop. Loopback bypass doesn't apply (this is ollama.com).
    let trusted = crate::config::cached_config().ssrf.trusted_hosts.clone();
    check_url(OLLAMA_INSTALL_URL, SsrfPolicy::Default, &trusted)
        .await
        .with_context(|| format!("SSRF blocked {OLLAMA_INSTALL_URL}"))?;

    emit_install_progress(emit, InstallScriptKind::Step, INSTALL_PHASE_DOWNLOAD);

    let client = crate::provider::apply_proxy_for_url(
        reqwest::Client::builder().timeout(Duration::from_secs(60)),
        OLLAMA_INSTALL_URL,
    )
    .build()
    .context("build install.sh client")?;
    let script = client
        .get(OLLAMA_INSTALL_URL)
        .send()
        .await
        .context("download install.sh")?
        .error_for_status()?
        .text()
        .await
        .context("read install.sh body")?;

    app_info!(
        "local_llm",
        "install_ollama",
        "downloaded install.sh ({} bytes)",
        script.len()
    );

    Ok(script)
}

#[cfg(unix)]
async fn run_ollama_install_script_authorized(
    script: &str,
    emit: &InstallProgressEmitter,
    cancel_token: CancellationToken,
) -> Result<()> {
    let temp = InstallerTempDir::new(script).context("prepare ollama install script")?;
    let command = build_logged_install_command(&temp.script_path, &temp.log_path);

    emit_install_progress(emit, InstallScriptKind::Step, INSTALL_PHASE_AUTHORIZE);

    let child = match spawn_authorized_install_command(&command) {
        Ok(child) => child,
        Err(err) => {
            let message = err.to_string();
            emit_install_progress(emit, InstallScriptKind::Error, message.clone());
            return Err(err);
        }
    };

    let output = wait_with_log_tail(child, &temp.log_path, emit, cancel_token).await?;

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let message = install_failure_message(&output);
        emit_install_progress(emit, InstallScriptKind::Error, message.clone());
        app_warn!(
            "local_llm",
            "install_ollama",
            "install.sh exited with code {}",
            code
        );
        return Err(anyhow!(message));
    }

    Ok(())
}

#[cfg(unix)]
struct InstallerTempDir {
    dir: PathBuf,
    script_path: PathBuf,
    log_path: PathBuf,
}

#[cfg(unix)]
impl InstallerTempDir {
    fn new(script: &str) -> Result<Self> {
        use std::fs::OpenOptions;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "hope-agent-ollama-install-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&dir).context("create installer temp dir")?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .context("secure installer temp dir")?;

        let script_path = dir.join("install.sh");
        let log_path = dir.join("install.log");

        std::fs::write(&script_path, script).context("write install script")?;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o600))
            .context("secure install script")?;

        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .context("create install log")?;
        log.set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("secure install log")?;

        Ok(Self {
            dir,
            script_path,
            log_path,
        })
    }
}

#[cfg(unix)]
impl Drop for InstallerTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
fn build_logged_install_command(script_path: &Path, log_path: &Path) -> String {
    format!(
        "{{ printf '%s\\n' {}; OLLAMA_NO_START=1 /bin/sh {}; }} >> {} 2>&1",
        shell_quote(INSTALL_STARTED_MARKER),
        shell_quote_path(script_path),
        shell_quote_path(log_path)
    )
}

#[cfg(unix)]
fn shell_quote_path(path: &Path) -> String {
    shell_quote(path.to_string_lossy().as_ref())
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn spawn_authorized_install_command(command: &str) -> Result<tokio::process::Child> {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut cmd = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("osascript");
        cmd.arg("-e").arg(format!(
            "do shell script \"{}\" with administrator privileges",
            applescript_string(command)
        ));
        cmd
    } else if current_user_is_root() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd
    } else if which::which("pkexec").is_ok() {
        let mut cmd = Command::new("pkexec");
        cmd.arg("/bin/sh").arg("-c").arg(command);
        cmd
    } else if sudo_askpass_available() && which::which("sudo").is_ok() {
        let mut cmd = Command::new("sudo");
        if let Some(askpass) =
            std::env::var_os("SUDO_ASKPASS").or_else(|| std::env::var_os("SSH_ASKPASS"))
        {
            cmd.env("SUDO_ASKPASS", askpass);
        }
        cmd.arg("-A").arg("/bin/sh").arg("-c").arg(command);
        cmd
    } else {
        return Err(anyhow!(
            "Graphical administrator authorization is unavailable. Install polkit/pkexec or configure SUDO_ASKPASS, then try again."
        ));
    };

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn authorized ollama installer")
}

#[cfg(unix)]
fn current_user_is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(unix)]
fn sudo_askpass_available() -> bool {
    std::env::var_os("SUDO_ASKPASS").is_some() || std::env::var_os("SSH_ASKPASS").is_some()
}

#[cfg(unix)]
fn applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(unix)]
async fn wait_with_log_tail(
    mut child: tokio::process::Child,
    log_path: &Path,
    emit: &InstallProgressEmitter,
    cancel_token: CancellationToken,
) -> Result<std::process::Output> {
    let mut offset = 0_u64;

    loop {
        tokio::select! {
            output = child.wait() => {
                let status = output.context("wait for authorized install script")?;
                let _ = emit_new_install_log_lines(log_path, &mut offset, emit).await;
                return Ok(std::process::Output {
                    status,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }
            _ = cancel_token.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = emit_new_install_log_lines(log_path, &mut offset, emit).await;
                return Err(anyhow!("Ollama install was cancelled"));
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                let _ = emit_new_install_log_lines(log_path, &mut offset, emit).await;
            }
        }
    }
}

#[cfg(not(unix))]
pub async fn install_ollama_via_script_cancellable<F>(
    _on_progress: F,
    _cancel_token: CancellationToken,
) -> Result<()>
where
    F: Fn(&InstallScriptProgress) + Send + Sync + 'static,
{
    Err(anyhow!(
        "Bundled installer is not supported on Windows. Please download Ollama from https://ollama.com/download"
    ))
}

#[cfg(unix)]
async fn emit_new_install_log_lines(
    log_path: &Path,
    offset: &mut u64,
    emit: &InstallProgressEmitter,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(log_path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).context("open install log"),
    };

    let len = file.metadata().await.context("stat install log")?.len();
    if len < *offset {
        *offset = 0;
    }

    file.seek(std::io::SeekFrom::Start(*offset))
        .await
        .context("seek install log")?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .await
        .context("read install log")?;
    *offset += bytes.len() as u64;

    let text = String::from_utf8_lossy(&bytes);
    for line in text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
    {
        if line == INSTALL_STARTED_MARKER {
            emit_install_progress(emit, InstallScriptKind::Step, INSTALL_PHASE_INSTALL);
        } else {
            emit_install_progress(emit, InstallScriptKind::Log, line);
        }
    }

    Ok(())
}

#[cfg(unix)]
fn install_failure_message(output: &std::process::Output) -> String {
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = format!("{stderr}\n{stdout}");
    let detail = details.trim();

    if authorization_was_denied(detail) {
        return "System authorization was canceled or denied.".into();
    }

    if detail.is_empty() {
        format!("install script exited with code {code}")
    } else {
        format!("install script exited with code {code}: {detail}")
    }
}

#[cfg(unix)]
fn authorization_was_denied(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("user canceled")
        || lower.contains("user cancelled")
        || lower.contains("not authorized")
        || lower.contains("authentication failed")
        || lower.contains("authorization failed")
        || lower.contains("dismissed")
}

// ── Model pull ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PullLine {
    status: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}

fn handle_pull_line(
    model_id: &str,
    line_text: &str,
    latest_phase: &mut String,
    emit: &(dyn Fn(&PullProgress) + Send + Sync),
    strict_json: bool,
) -> Result<()> {
    let trimmed = line_text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let parsed: PullLine = match serde_json::from_str(trimmed) {
        Ok(p) => p,
        Err(e) if strict_json => {
            return Err(anyhow!(
                "Ollama pull stream ended with invalid JSON frame: {e}"
            ));
        }
        Err(e) => {
            app_warn!(
                "local_llm",
                "pull_model",
                "skip non-JSON line ({}): {}",
                e,
                trimmed
            );
            return Ok(());
        }
    };
    if let Some(err) = parsed.error {
        return Err(anyhow!("Ollama pull error: {err}"));
    }

    let phase = parsed.status.unwrap_or_else(|| "unknown".into());
    latest_phase.clear();
    latest_phase.push_str(&phase);
    let percent = match (parsed.completed, parsed.total) {
        (Some(c), Some(t)) if t > 0 => {
            Some(((c as f64 / t as f64) * 100.0).clamp(0.0, 100.0) as u8)
        }
        _ => None,
    };
    emit(&PullProgress {
        model_id: model_id.into(),
        phase,
        percent,
    });
    Ok(())
}

fn drain_pull_lines(
    model_id: &str,
    buf: &mut Vec<u8>,
    latest_phase: &mut String,
    emit: &(dyn Fn(&PullProgress) + Send + Sync),
) -> Result<()> {
    while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
        let line = buf.drain(..=pos).collect::<Vec<u8>>();
        let line_text = std::str::from_utf8(&line[..line.len().saturating_sub(1)])
            .context("decode Ollama pull line")?;
        handle_pull_line(model_id, line_text, latest_phase, emit, false)?;
    }
    Ok(())
}

fn finish_pull_stream(
    model_id: &str,
    buf: &mut Vec<u8>,
    latest_phase: &mut String,
    emit: &(dyn Fn(&PullProgress) + Send + Sync),
) -> Result<()> {
    if !buf.is_empty() {
        let line_text = std::str::from_utf8(buf).context("decode trailing Ollama pull line")?;
        handle_pull_line(model_id, line_text, latest_phase, emit, true)
            .context("parse trailing Ollama pull frame")?;
        buf.clear();
    }

    if !latest_phase.eq_ignore_ascii_case("success") {
        let last = if latest_phase.is_empty() {
            "<none>"
        } else {
            latest_phase.as_str()
        };
        app_warn!(
            "local_llm",
            "pull_model",
            "pull stream ended without success status (last={})",
            last
        );
        return Err(anyhow!(
            "Ollama pull stream ended before success status (last={last})"
        ));
    }

    Ok(())
}

/// Stream `POST /api/pull` and emit per-frame progress. Returns when Ollama
/// closes the connection — successful pulls end with `status="success"`.
pub async fn pull_model<F>(model_id: &str, on_progress: F) -> Result<()>
where
    F: Fn(&PullProgress) + Send + Sync + 'static,
{
    pull_model_cancellable(model_id, on_progress, CancellationToken::new()).await
}

pub async fn pull_model_cancellable<F>(
    model_id: &str,
    on_progress: F,
    cancel_token: CancellationToken,
) -> Result<()>
where
    F: Fn(&PullProgress) + Send + Sync + 'static,
{
    use futures_util::StreamExt;

    if !ping_ollama().await {
        return Err(anyhow!(
            "Ollama daemon is not running on {OLLAMA_BASE_URL}. Click Start Ollama first."
        ));
    }

    let emit = std::sync::Arc::new(on_progress);
    emit(&PullProgress {
        model_id: model_id.into(),
        phase: "starting".into(),
        percent: None,
    });

    // Pulls run for many minutes — no outer timeout. The peer closes the
    // stream at end-of-pull; reqwest's TCP keepalive notices a dead peer.
    let client = crate::provider::apply_proxy_for_url(reqwest::Client::builder(), OLLAMA_BASE_URL)
        .build()
        .context("build pull client")?;

    let resp = client
        .post(format!("{OLLAMA_BASE_URL}/api/pull"))
        .json(&serde_json::json!({"model": model_id, "stream": true}))
        .send()
        .await
        .context("POST /api/pull")?;
    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Ollama /api/pull returned {code}: {body}"));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut latest_phase = String::new();
    loop {
        let next_chunk = tokio::select! {
            _ = cancel_token.cancelled() => {
                return Err(anyhow!("Ollama pull was cancelled"));
            }
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next_chunk else { break };
        let chunk = chunk.context("read pull chunk")?;
        buf.extend_from_slice(&chunk);
        if buf.len() > MAX_PULL_LINE_BYTES {
            return Err(anyhow!(
                "Ollama pull stream exceeded {MAX_PULL_LINE_BYTES} bytes without a newline"
            ));
        }

        drain_pull_lines(model_id, &mut buf, &mut latest_phase, emit.as_ref())?;
    }

    finish_pull_stream(model_id, &mut buf, &mut latest_phase, emit.as_ref())
}

// ── Provider registration ─────────────────────────────────────────

/// Add the local Ollama provider (or upsert the requested model into the
/// existing one) and set it as the active model in a single atomic
/// `mutate_config` write so the `config:changed` consumers never observe
/// the half-done state.
pub fn ensure_ollama_provider_with_model(model: &ModelCandidate) -> Result<(String, String)> {
    let model_cfg = ModelConfig {
        id: model.id.clone(),
        name: model.display_name.clone(),
        input_types: vec!["text".into()],
        context_window: model.context_window,
        max_tokens: 8192,
        reasoning: model.reasoning,
        thinking_style: None,
        cost_input: 0.0,
        cost_output: 0.0,
    };

    let mut provider = ProviderConfig::new(
        OLLAMA_PROVIDER_NAME.into(),
        crate::provider::ApiType::OpenaiChat,
        OLLAMA_BASE_URL.into(),
        String::new(),
    );
    provider.thinking_style = ThinkingStyle::Qwen;

    let (provider_id, model_id) =
        upsert_known_local_provider_model("ollama", provider, model_cfg, true, PROVIDER_SOURCE)?;

    app_info!(
        "local_llm",
        "register_provider",
        "Ollama provider {} active with model {}",
        provider_id,
        model_id
    );
    Ok((provider_id, model_id))
}

// ── End-to-end orchestration ──────────────────────────────────────

/// Pull the requested model, register the local-Ollama provider, and mark
/// it active. Progress frames are emitted for both the pull phase and the
/// post-pull bookkeeping phases.
pub async fn pull_and_activate_cancellable<F>(
    model: ModelCandidate,
    on_progress: F,
    cancel_token: CancellationToken,
) -> Result<(String, String)>
where
    F: Fn(&PullProgress) + Send + Sync + 'static,
{
    let on_progress = std::sync::Arc::new(on_progress);
    let cb = on_progress.clone();
    pull_model_cancellable(&model.id, move |p| cb(p), cancel_token).await?;

    let model_id = model.id.clone();
    on_progress(&PullProgress {
        model_id: model_id.clone(),
        phase: "register-provider".into(),
        percent: Some(99),
    });
    let result = ensure_ollama_provider_with_model(&model)?;

    on_progress(&PullProgress {
        model_id,
        phase: "done".into(),
        percent: Some(100),
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget_hw(budget_mb: u64, src: BudgetSource) -> HardwareInfo {
        HardwareInfo {
            os: "test".into(),
            total_memory_mb: 16 * 1024,
            available_memory_mb: 12 * 1024,
            gpu: None,
            budget_source: src,
            budget_mb,
        }
    }

    #[test]
    fn recommends_largest_fitting_model() {
        // 32 GiB Mac → budget is capped below qwen3.6:27b in this fixture.
        // gemma4:e4b (9830 MiB) is the largest entry that fits; the next
        // step up (qwen3.6:27b @ 17408) overshoots.
        let rec = recommend_model(&budget_hw(15 * 1024, BudgetSource::UnifiedMemory));
        let r = rec.recommended.expect("should recommend");
        assert_eq!(r.id, "gemma4:e4b");
        assert_eq!(rec.reason, RecommendationReason::UnifiedMemory);
        assert!(rec
            .alternatives
            .first()
            .map(|c| c.id == r.id)
            .unwrap_or(false));
    }

    #[test]
    fn returns_none_when_budget_too_small() {
        // 16 GiB Mac → budget ≈ 7 GiB ≈ 7168 MiB; smaller than the smallest
        // catalog entry (gemma4:e2b @ 7373 MiB), so we bow out gracefully.
        let rec = recommend_model(&budget_hw(7 * 1024, BudgetSource::SystemMemory));
        assert!(rec.recommended.is_none());
        assert_eq!(rec.reason, RecommendationReason::Insufficient);
    }

    #[test]
    fn recommendation_budget_uses_sixty_percent_with_buffer() {
        assert_eq!(recommendation_budget_mb(32 * 1024), 18_636);
    }

    fn collect_pull_frames(chunks: &[&[u8]]) -> Result<Vec<PullProgress>> {
        use std::sync::Mutex;

        let frames = Mutex::new(Vec::<PullProgress>::new());
        let emit = |p: &PullProgress| frames.lock().unwrap().push(p.clone());
        let mut buf = Vec::<u8>::new();
        let mut latest_phase = String::new();

        for chunk in chunks {
            buf.extend_from_slice(chunk);
            drain_pull_lines("test-model", &mut buf, &mut latest_phase, &emit)?;
        }
        finish_pull_stream("test-model", &mut buf, &mut latest_phase, &emit)?;

        Ok(frames.into_inner().unwrap())
    }

    #[test]
    fn pull_stream_accepts_final_success_with_newline() {
        let frames = collect_pull_frames(&[
            br#"{"status":"pulling manifest"}"#,
            b"\n",
            br#"{"status":"success"}"#,
            b"\n",
        ])
        .expect("success frame should finish pull");

        assert_eq!(frames.last().map(|p| p.phase.as_str()), Some("success"));
    }

    #[test]
    fn pull_stream_accepts_final_success_without_newline() {
        let frames = collect_pull_frames(&[
            br#"{"status":"downloading","completed":50,"total":100}"#,
            b"\n",
            br#"{"status":"success"}"#,
        ])
        .expect("trailing success frame should finish pull");

        assert_eq!(frames.last().map(|p| p.phase.as_str()), Some("success"));
    }

    #[test]
    fn pull_stream_errors_on_early_eof_without_success() {
        let err = collect_pull_frames(&[
            br#"{"status":"downloading","completed":50,"total":100}"#,
            b"\n",
        ])
        .expect_err("early EOF should not activate a model");

        assert!(err.to_string().contains("before success status"));
    }

    #[test]
    fn pull_stream_errors_on_truncated_final_frame() {
        let err = collect_pull_frames(&[br#"{"status":"success""#])
            .expect_err("truncated trailing JSON should fail");

        assert!(err.to_string().contains("parse trailing Ollama pull frame"));
    }
}
