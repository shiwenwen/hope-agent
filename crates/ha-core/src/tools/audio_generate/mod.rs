//! `audio_generate` chat tool — thin front-end over the unified media-gen
//! stack (`crate::media_gen`): parses tool args (kind / voice / duration /
//! model), delegates candidate resolution / failover / accounting to
//! `media_gen::execute_audio`, saves the audio to the session's attachments
//! and returns a `__MEDIA_ITEMS__` payload (kind `file` + accurate audio
//! mime — the existing FileCard → FilePreviewPane `<audio controls>` path
//! plays it).
//!
//! Billed side effect: must stay OUT of `async_jobs::retry::is_retry_eligible`.

use anyhow::Result;
use chrono::Local;
use serde_json::Value;

use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};
use crate::media_gen::{
    execute_audio, AudioKind, AudioRequest, MediaGenConfig, MediaModality, UsageMeta,
};
use crate::tools::execution::ToolExecContext;

/// Explicit kind arg > `[music]` / `[sfx]` prompt prefix > speech.
fn resolve_kind(args: &Value, prompt: &str) -> AudioKind {
    if let Some(kind) = args
        .get("kind")
        .and_then(|v| v.as_str())
        .and_then(AudioKind::parse)
    {
        return kind;
    }
    let lower = prompt.trim().to_ascii_lowercase();
    if lower.starts_with("[music]") {
        AudioKind::Music
    } else if lower.starts_with("[sfx]") {
        AudioKind::Sfx
    } else {
        AudioKind::Speech
    }
}

/// Strip a leading `[music]` / `[sfx]` hint (case-insensitive) so the
/// literal tag doesn't degrade generation.
fn strip_kind_prefix(prompt: &str) -> &str {
    let trimmed = prompt.trim();
    let low = trimmed.to_ascii_lowercase();
    if low.starts_with("[music]") {
        trimmed[7..].trim()
    } else if low.starts_with("[sfx]") {
        trimmed[5..].trim()
    } else {
        trimmed
    }
}

fn ext_for_mime(mime: &str) -> &'static str {
    if mime.contains("wav") {
        "wav"
    } else if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("aac") {
        "aac"
    } else {
        "mp3"
    }
}

pub(crate) async fn tool_audio_generate(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let config = crate::config::cached_config().media_gen.clone();

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("generate");

    if action == "list" {
        return build_list_result(&config);
    }
    if action != "generate" {
        anyhow::bail!("Invalid action '{}'. Must be 'generate' or 'list'.", action);
    }

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;
    let kind = resolve_kind(args, prompt);
    let clean_prompt = strip_kind_prefix(prompt);
    let voice = args
        .get("voice")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let duration_seconds = args
        .get("durationSeconds")
        .and_then(|v| v.as_f64())
        .filter(|d| d.is_finite() && *d > 0.0);
    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "auto");

    let outcome = execute_audio(
        &config,
        AudioRequest {
            prompt: clean_prompt,
            kind: Some(kind),
            voice,
            duration_seconds,
            explicit_model: model_override,
        },
        UsageMeta {
            operation: "tool.audio_generate",
            source: "tool",
            session_id: ctx.session_id.clone(),
            agent_id: ctx.agent_id.clone(),
        },
    )
    .await?;

    let audio = outcome.result;
    let ext = ext_for_mime(&audio.mime);
    let display_filename = format!(
        "{}_{}.{}",
        Local::now().format("%Y%m%d_%H%M%S"),
        kind.as_str(),
        ext
    );
    let session_id = ctx.session_id.as_deref();
    let saved_path =
        attachments::save_attachment_bytes(session_id, &display_filename, &audio.data)?;
    // `MediaKind::File` + accurate audio mime rides the existing media_items
    // pipeline (FileCard inline, FilePreviewPane playback) — no new kind.
    let item = MediaItem::from_saved_path(
        session_id,
        &saved_path,
        &display_filename,
        audio.mime.clone(),
        audio.data.len() as u64,
        MediaKind::File,
        None,
    );

    let mut text_parts = vec![format!(
        "Generated {} audio with {}/{}.",
        kind.as_str(),
        outcome.provider_name,
        outcome.model_id
    )];
    if let Some(v) = voice {
        text_parts.push(format!("Voice: {}", v));
    }
    if let Some(d) = duration_seconds {
        text_parts.push(format!("Requested duration: {}s", d));
    }
    if !outcome.failover_log.is_empty() {
        text_parts.push(format!("[Failover] {}", outcome.failover_log.join(" → ")));
    }
    text_parts.push(format!("Saved to: {}", saved_path));

    let items_json = serde_json::to_string(&vec![item]).unwrap_or_else(|_| "[]".to_string());
    app_info!(
        "tool",
        "audio_generate",
        "audio generation complete: kind={}, {} bytes, provider={}/{}",
        kind.as_str(),
        audio.data.len(),
        outcome.provider_name,
        outcome.model_id
    );
    Ok(format!(
        "{}{}\n{}",
        MEDIA_ITEMS_PREFIX,
        items_json,
        text_parts.join("\n")
    ))
}

/// `action=list`: providers × audio models with kinds / duration / voice.
fn build_list_result(config: &MediaGenConfig) -> Result<String> {
    let mut lines = Vec::new();
    lines.push("Available Audio Generation Providers:".to_string());
    lines.push(String::new());

    let usable: Vec<_> = config
        .providers
        .iter()
        .filter(|p| p.is_usable() && p.models.iter().any(|m| m.modality == MediaModality::Audio))
        .collect();
    if usable.is_empty() {
        lines.push(
            "No providers configured. Add one in Settings → Model Providers → Generation Models."
                .to_string(),
        );
        return Ok(lines.join("\n"));
    }

    for (i, provider) in usable.iter().enumerate() {
        lines.push(format!("{}. {} [Priority {}]", i + 1, provider.name, i + 1));
        if let Some(voice) = provider.default_voice.as_deref().filter(|v| !v.is_empty()) {
            lines.push(format!("   Default voice: {}", voice));
        }
        for model in provider
            .models
            .iter()
            .filter(|m| m.modality == MediaModality::Audio)
        {
            let Some(caps) = &model.audio else {
                lines.push(format!("   - {} (capabilities unknown)", model.id));
                continue;
            };
            let kinds = if caps.kinds.is_empty() {
                "any".to_string()
            } else {
                caps.kinds
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            };
            let mut feats = vec![format!("kinds: {}", kinds)];
            if caps.supports_duration {
                let range = match (caps.min_duration_secs, caps.max_duration_secs) {
                    (Some(min), Some(max)) => format!("duration {min}-{max}s"),
                    _ => "duration".to_string(),
                };
                feats.push(range);
            }
            if caps.needs_voice {
                feats.push("voice".to_string());
            }
            lines.push(format!("   - {}: {}", model.id, feats.join(", ")));
        }
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}
