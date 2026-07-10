//! `audio` 形态：接线 `audio_generate` provider 栈，生成音频并内嵌进**自包含产物**
//! （data-uri `<audio>` 播放器，守「轻量自包含 HTML」红线——纯静态元素、零运行时、零网络，
//! 浏览器原生解码，比 motion 还轻）。
//!
//! failover 只在**支持该 AudioKind**（语音/音乐/音效）且 enabled + 有 key 的候选间轮换。

use anyhow::{anyhow, Result};
use base64::Engine;

use super::renderer::{html_escape, ArtifactParts};
use crate::tools::audio_generate::{
    effective_model, resolve_provider, AudioGenParams, AudioGenResult, AudioKind,
};

/// 从 prompt 推断音频子能力（含 `[music]` / `[sfx]` 前缀提示，默认语音旁白）。
pub fn infer_audio_kind(prompt: &str) -> AudioKind {
    let lower = prompt.trim().to_ascii_lowercase();
    if lower.starts_with("[music]") || lower.contains("背景音乐") || lower.contains("bgm") {
        AudioKind::Music
    } else if lower.starts_with("[sfx]") || lower.contains("音效") || lower.contains("sound effect")
    {
        AudioKind::Sfx
    } else {
        AudioKind::Speech
    }
}

/// 文本 prompt → 生成音频 → 返回内嵌 data-uri `<audio>` 播放器的 `ArtifactParts`。
/// `duration_seconds`（B8-2）：music / sfx 的可选目标时长。
pub async fn generate_audio_parts(
    prompt: &str,
    title: &str,
    duration_seconds: Option<f64>,
) -> Result<ArtifactParts> {
    let kind = infer_audio_kind(prompt);
    let (bytes, mime) = generate_audio_bytes(prompt, kind, duration_seconds).await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let esc_title = html_escape(title);
    let label = match kind {
        AudioKind::Speech => "语音旁白",
        AudioKind::Music => "音乐",
        AudioKind::Sfx => "音效",
    };
    // 居中卡片 + 原生 <audio controls>；纯静态、零网络（data-uri 内嵌）。
    let body_html = format!(
        "<main style=\"display:flex;flex-direction:column;align-items:center;justify-content:center;\
min-height:60vh;gap:18px;padding:48px;text-align:center;\
font-family:var(--ds-font-sans,system-ui,-apple-system,sans-serif)\">\
<div style=\"font-size:20px;font-weight:600;color:var(--ds-color-fg,#111827)\">{esc_title}</div>\
<div style=\"font-size:13px;color:var(--ds-color-muted,#6b7280)\">{label}</div>\
<audio controls src=\"data:{mime};base64,{b64}\" style=\"width:min(520px,90vw)\"></audio>\
</main>"
    );
    Ok(ArtifactParts {
        body_html,
        css: String::new(),
        js: String::new(),
    })
}

/// 生成音频字节，按配置顺序在**支持该 kind** 的多个 provider 间 failover。
/// `duration_seconds`（B8-2）：music / sfx 的目标时长，None = provider 默认（各自钳合法区间）。
async fn generate_audio_bytes(
    prompt: &str,
    kind: AudioKind,
    duration_seconds: Option<f64>,
) -> Result<(Vec<u8>, String)> {
    if prompt.trim().is_empty() {
        anyhow::bail!("audio prompt is empty");
    }
    let app_cfg = crate::config::cached_config();
    let cfg = &app_cfg.audio_generate;
    // strip 前缀提示（[music]/[sfx]，**大小写不敏感**，对齐 infer_audio_kind 的小写匹配——否则
    // `[MUSIC]` 剥不掉、字面标签随文本进 provider 劣化生成），把干净文本喂给 provider。
    let trimmed = prompt.trim();
    let low = trimmed.to_ascii_lowercase();
    let clean = if low.starts_with("[music]") {
        trimmed[7..].trim()
    } else if low.starts_with("[sfx]") {
        trimmed[5..].trim()
    } else {
        trimmed
    };

    let candidates: Vec<_> = cfg
        .providers
        .iter()
        .filter(|p| {
            p.enabled
                && p.api_key.as_deref().is_some_and(|k| !k.is_empty())
                && resolve_provider(&p.id).is_some_and(|impl_| impl_.supports(kind))
        })
        .collect();
    if candidates.is_empty() {
        anyhow::bail!(
            "no audio provider configured for {} (Settings → Tools → Audio)",
            kind.as_str()
        );
    }

    let mut last_err: Option<anyhow::Error> = None;
    for entry in candidates {
        let Some(provider) = resolve_provider(&entry.id) else {
            continue;
        };
        let Some(api_key) = entry.api_key.as_deref() else {
            continue;
        };
        let model = effective_model(entry, kind);
        let params = AudioGenParams {
            api_key,
            base_url: entry.base_url.as_deref(),
            model: &model,
            prompt: clean,
            kind,
            timeout_secs: cfg.timeout_seconds,
            duration_seconds,
            entry,
        };
        let started = std::time::Instant::now();
        match provider.generate(params).await {
            Ok(AudioGenResult { data, mime }) => {
                // Audio providers return no token usage — record call + duration
                // only (KIND_AUDIO_GENERATION), matching the image path.
                record_audio_usage(
                    &entry.id,
                    provider.display_name(),
                    &model,
                    kind,
                    started.elapsed().as_millis() as u64,
                    true,
                    None,
                );
                crate::app_info!(
                    "design",
                    "audio",
                    "generated {} audio {} bytes via provider={}",
                    kind.as_str(),
                    data.len(),
                    entry.id
                );
                return Ok((data, mime));
            }
            Err(e) => {
                record_audio_usage(
                    &entry.id,
                    provider.display_name(),
                    &model,
                    kind,
                    started.elapsed().as_millis() as u64,
                    false,
                    Some(e.to_string()),
                );
                crate::app_warn!(
                    "design",
                    "audio",
                    "audio provider '{}' failed for {}, trying next: {e}",
                    entry.id,
                    kind.as_str()
                );
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("all audio providers failed")))
}

/// Record one design audio-generation attempt (`KIND_AUDIO_GENERATION`, owner
/// plane so no session/agent id). Audio carries no token usage — call count +
/// duration only, per the usage-ledger contract.
fn record_audio_usage(
    provider_id: &str,
    provider_name: &str,
    model: &str,
    kind: AudioKind,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
) {
    let mut event =
        crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_AUDIO_GENERATION);
    event.operation = Some("design.audio".to_string());
    event.source = Some("design.audio".to_string());
    event.provider_id = Some(provider_id.to_string());
    // Human display name (uniform with the image / tool paths) so Dashboard
    // "by model" grouping shows a clean provider label.
    event.provider_name = Some(provider_name.to_string());
    event.model_id = Some(model.to_string());
    event.duration_ms = Some(duration_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(serde_json::json!({ "audio_kind": kind.as_str() }));
    crate::model_usage::record_model_usage_best_effort(event);
}
