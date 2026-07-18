//! `audio` 形态：接线统一媒体生成栈（`crate::media_gen`），生成音频并内嵌进
//! **自包含产物**（data-uri `<audio>` 播放器，守「轻量自包含 HTML」红线——纯静态
//! 元素、零运行时、零网络，浏览器原生解码，比 motion 还轻）。
//!
//! 链解析 / kind 过滤 / failover / 记账全在 `media_gen::execute_audio`；kind 可
//! 显式指定（UI 分段选择 / design 工具参数），缺省才回退 prompt 前缀推断。

use anyhow::Result;
use base64::Engine;

use super::renderer::{html_escape, ArtifactParts};
use crate::media_gen::{execute_audio, AudioKind, AudioRequest, UsageMeta};

/// 从 prompt 推断音频子能力（含 `[music]` / `[sfx]` 前缀提示，默认语音旁白）。
/// 显式 kind（`AudioGenPartsOptions.kind`）优先，本函数只是兼容回退。
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

/// 音频生成可选项：显式 kind > prompt 前缀推断；voice 是调用级最高覆盖
/// （> 模型默认 > provider 默认 > adapter 内置）；duration 只对 music / sfx 生效。
#[derive(Default)]
pub struct AudioGenPartsOptions {
    pub kind: Option<AudioKind>,
    pub voice: Option<String>,
    pub duration_seconds: Option<f64>,
}

/// 文本 prompt → 生成音频 → 返回内嵌 data-uri `<audio>` 播放器的 `ArtifactParts`。
pub async fn generate_audio_parts(
    prompt: &str,
    title: &str,
    opts: &AudioGenPartsOptions,
) -> Result<ArtifactParts> {
    let kind = opts.kind.unwrap_or_else(|| infer_audio_kind(prompt));
    let (bytes, mime) = generate_audio_bytes(prompt, kind, opts).await?;
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

/// 生成音频字节。前缀提示剥离后喂干净文本给 provider；其余交给统一执行器。
async fn generate_audio_bytes(
    prompt: &str,
    kind: AudioKind,
    opts: &AudioGenPartsOptions,
) -> Result<(Vec<u8>, String)> {
    if prompt.trim().is_empty() {
        anyhow::bail!("audio prompt is empty");
    }
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

    let app_cfg = crate::config::cached_config();
    let outcome = execute_audio(
        &app_cfg.media_gen,
        AudioRequest {
            prompt: clean,
            kind: Some(kind),
            voice: opts.voice.as_deref(),
            duration_seconds: opts.duration_seconds,
            explicit_model: None,
        },
        UsageMeta {
            operation: "design.audio",
            source: "design.audio",
            session_id: None,
            agent_id: None,
        },
    )
    .await?;

    crate::app_info!(
        "design",
        "audio",
        "generated {} audio {} bytes via {}/{}",
        kind.as_str(),
        outcome.result.data.len(),
        outcome.provider_name,
        outcome.model_id
    );
    Ok((outcome.result.data, outcome.result.mime))
}
