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
use std::sync::OnceLock;
use std::time::Duration;

use crate::provider::{ApiType, ModelConfig, ProviderConfig, ThinkingStyle};
#[cfg(unix)]
use crate::security::ssrf::{check_url, SsrfPolicy};

pub mod types;
pub use types::*;

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
#[cfg(unix)]
const OLLAMA_INSTALL_URL: &str = "https://ollama.com/install.sh";
const PROVIDER_SOURCE: &str = "local-llm-wizard";
const OLLAMA_PROVIDER_NAME: &str = "Ollama (local)";
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

    // Half the chosen axis, minus a 1 GiB buffer for runtime overhead.
    let budget_mb = base_mb.saturating_div(2).saturating_sub(1024);

    HardwareInfo {
        os: s.os.clone(),
        total_memory_mb: s.total_memory_mb,
        available_memory_mb,
        gpu: s.gpu.clone(),
        budget_source,
        budget_mb,
    }
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
    let installed = tokio::task::spawn_blocking(|| which::which("ollama").is_ok())
        .await
        .unwrap_or(false);

    let running = ping_ollama().await;
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

// ── Ollama lifecycle ──────────────────────────────────────────────

/// Spawn `ollama serve` detached and wait up to 10 s for the HTTP API to
/// answer. Idempotent — if the daemon already responds, returns Ok early.
pub async fn start_ollama() -> Result<()> {
    if ping_ollama().await {
        app_info!("local_llm", "start_ollama", "already running");
        return Ok(());
    }
    let installed = tokio::task::spawn_blocking(|| which::which("ollama").is_ok())
        .await
        .unwrap_or(false);
    if !installed {
        return Err(anyhow!("Ollama is not installed"));
    }

    spawn_ollama_serve()?;

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if ping_ollama().await {
            app_info!("local_llm", "start_ollama", "ready");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(anyhow!(
        "Ollama did not respond on {OLLAMA_BASE_URL}/api/tags within 10s"
    ))
}

#[cfg(unix)]
fn spawn_ollama_serve() -> Result<()> {
    use std::process::{Command, Stdio};
    Command::new("ollama")
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn `ollama serve`")?;
    Ok(())
}

#[cfg(windows)]
fn spawn_ollama_serve() -> Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("ollama")
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .context("spawn `ollama serve`")?;
    Ok(())
}

/// Run the upstream Ollama install script and stream stdout/stderr to
/// `on_progress`. Unix only — Windows users are routed to the download page
/// in the UI, see [`OllamaStatus::install_script_supported`].
#[cfg(unix)]
pub async fn install_ollama_via_script<F>(on_progress: F) -> Result<()>
where
    F: Fn(&InstallScriptProgress) + Send + Sync + 'static,
{
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    let emit = std::sync::Arc::new(on_progress);

    // Public HTTPS — must pass through SSRF + global proxy like every
    // other outbound hop. Loopback bypass doesn't apply (this is ollama.com).
    let trusted = crate::config::cached_config().ssrf.trusted_hosts.clone();
    check_url(OLLAMA_INSTALL_URL, SsrfPolicy::Default, &trusted)
        .await
        .with_context(|| format!("SSRF blocked {OLLAMA_INSTALL_URL}"))?;

    emit(&InstallScriptProgress {
        kind: InstallScriptKind::Step,
        message: "download".into(),
    });

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

    emit(&InstallScriptProgress {
        kind: InstallScriptKind::Step,
        message: "running script".into(),
    });
    app_info!(
        "local_llm",
        "install_ollama",
        "downloaded install.sh ({} bytes)",
        script.len()
    );

    let mut child = Command::new("sh")
        .arg("-s")
        .arg("--")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn sh -s")?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(script.as_bytes()).await.ok();
        // Drop closes stdin so `sh` knows the script is complete.
    }

    let stdout = child.stdout.take().context("capture stdout")?;
    let stderr = child.stderr.take().context("capture stderr")?;

    let emit_out = emit.clone();
    let stdout_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            emit_out(&InstallScriptProgress {
                kind: InstallScriptKind::Log,
                message: line,
            });
        }
    });
    let emit_err = emit.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            emit_err(&InstallScriptProgress {
                kind: InstallScriptKind::Log,
                message: line,
            });
        }
    });

    let status = child.wait().await.context("wait for install script")?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        emit(&InstallScriptProgress {
            kind: InstallScriptKind::Error,
            message: format!("install script exited with code {code}"),
        });
        app_warn!(
            "local_llm",
            "install_ollama",
            "install.sh exited with code {}",
            code
        );
        return Err(anyhow!("install script exited with code {code}"));
    }

    emit(&InstallScriptProgress {
        kind: InstallScriptKind::Step,
        message: "done".into(),
    });
    app_info!("local_llm", "install_ollama", "install.sh succeeded");
    Ok(())
}

#[cfg(windows)]
pub async fn install_ollama_via_script<F>(_on_progress: F) -> Result<()>
where
    F: Fn(&InstallScriptProgress) + Send + Sync + 'static,
{
    Err(anyhow!(
        "Bundled installer is not supported on Windows. Please download Ollama from https://ollama.com/download"
    ))
}

// ── Model pull ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PullLine {
    status: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}

/// Stream `POST /api/pull` and emit per-frame progress. Returns when Ollama
/// closes the connection — successful pulls end with `status="success"`.
pub async fn pull_model<F>(model_id: &str, on_progress: F) -> Result<()>
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
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read pull chunk")?;
        buf.extend_from_slice(&chunk);
        if buf.len() > MAX_PULL_LINE_BYTES {
            return Err(anyhow!(
                "Ollama pull stream exceeded {MAX_PULL_LINE_BYTES} bytes without a newline"
            ));
        }

        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line = buf.drain(..=pos).collect::<Vec<u8>>();
            let line_text = String::from_utf8_lossy(&line[..line.len().saturating_sub(1)]);
            if line_text.trim().is_empty() {
                continue;
            }
            let parsed: PullLine = match serde_json::from_str(&line_text) {
                Ok(p) => p,
                Err(e) => {
                    app_warn!(
                        "local_llm",
                        "pull_model",
                        "skip non-JSON line ({}): {}",
                        e,
                        line_text
                    );
                    continue;
                }
            };
            if let Some(err) = parsed.error {
                return Err(anyhow!("Ollama pull error: {err}"));
            }
            let phase = parsed.status.unwrap_or_else(|| "unknown".into());
            latest_phase = phase.clone();
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
        }
    }

    if !latest_phase.eq_ignore_ascii_case("success") {
        app_warn!(
            "local_llm",
            "pull_model",
            "pull stream ended without success status (last={})",
            latest_phase
        );
    }
    Ok(())
}

// ── Provider registration ─────────────────────────────────────────

/// Add the local Ollama provider (or upsert the requested model into the
/// existing one) and set it as the active model in a single atomic
/// `mutate_config` write so the `config:changed` consumers never observe
/// the half-done state.
pub fn ensure_ollama_provider_with_model(model: &ModelCandidate) -> Result<(String, String)> {
    use crate::config::mutate_config;
    use crate::provider::ActiveModel;

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

    let model_id = model.id.clone();
    let provider_id = mutate_config(("providers.add+activate", PROVIDER_SOURCE), |store| {
        // Upsert the local-Ollama provider keyed by base_url so we never
        // create duplicate rows when the user clicks Install twice.
        let existing_idx = store
            .providers
            .iter()
            .position(|p| p.api_type == ApiType::OpenaiChat && is_local_ollama_url(&p.base_url));
        let pid = if let Some(idx) = existing_idx {
            let p = &mut store.providers[idx];
            if !p.models.iter().any(|m| m.id == model_id) {
                p.models.push(model_cfg.clone());
            }
            p.enabled = true;
            p.allow_private_network = true;
            p.id.clone()
        } else {
            let mut new_provider = ProviderConfig::new(
                OLLAMA_PROVIDER_NAME.into(),
                ApiType::OpenaiChat,
                OLLAMA_BASE_URL.into(),
                String::new(),
            );
            new_provider.models.push(model_cfg.clone());
            new_provider.allow_private_network = true;
            new_provider.thinking_style = ThinkingStyle::Qwen;
            let id = new_provider.id.clone();
            store.providers.push(new_provider);
            id
        };
        store.active_model = Some(ActiveModel {
            provider_id: pid.clone(),
            model_id: model_id.clone(),
        });
        Ok(pid)
    })?;

    app_info!(
        "local_llm",
        "register_provider",
        "Ollama provider {} active with model {}",
        provider_id,
        model.id
    );
    Ok((provider_id, model.id.clone()))
}

fn is_local_ollama_url(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    lowered.contains("127.0.0.1:11434")
        || lowered.contains("localhost:11434")
        || lowered.contains("ollama.local")
}

// ── End-to-end orchestration ──────────────────────────────────────

/// Pull the requested model, register the local-Ollama provider, and mark
/// it active. Progress frames are emitted for both the pull phase and the
/// post-pull bookkeeping phases.
pub async fn pull_and_activate<F>(model: ModelCandidate, on_progress: F) -> Result<(String, String)>
where
    F: Fn(&PullProgress) + Send + Sync + 'static,
{
    let on_progress = std::sync::Arc::new(on_progress);
    let cb = on_progress.clone();
    pull_model(&model.id, move |p| cb(p)).await?;

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
        // 32 GiB Mac → budget = 32 GiB / 2 - 1 GiB ≈ 15360 MiB.
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
    fn local_ollama_url_match() {
        assert!(is_local_ollama_url("http://127.0.0.1:11434"));
        assert!(is_local_ollama_url("http://localhost:11434/v1"));
        assert!(is_local_ollama_url("http://ollama.local:11434"));
        assert!(!is_local_ollama_url("https://api.openai.com"));
    }
}
