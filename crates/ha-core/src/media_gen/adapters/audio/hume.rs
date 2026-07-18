//! Hume AI Octave TTS — speech via `POST /v0/tts/file`（裸音频字节；
//! 同族的 `/v0/tts` 返回 JSON + base64，这里不用）。Hume 无音乐 / 音效端点。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;
use serde_json::{json, Value};

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.hume.ai";
/// 单条 utterance 的官方 text 上限（字符数，非字节）。
const MAX_UTTERANCE_CHARS: usize = 5000;
const SPEED_MIN: f64 = 0.5;
const SPEED_MAX: f64 = 2.0;

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

/// Octave 的模型代际走 `version` 枚举（字面值只有 `"1"` / `"2"`），请求体里
/// **没有 `model` 字段**——把 `octave-2` 这类 id 当 model 发出去必被 schema 拒。
/// 认不出代际时返回 `None`，让服务端按账号默认路由。
/// Matches the catalog ids exactly rather than sniffing for digits: a
/// snapshot-suffixed id like `octave-1-20260210` contains a '2' and would
/// otherwise be routed to Octave 2, whose voices are incompatible.
fn resolve_version(model: &str) -> Option<&'static str> {
    match model.trim().to_ascii_lowercase().as_str() {
        "octave-2" | "2" => Some("2"),
        "octave-1" | "1" => Some("1"),
        // Unknown id: let the server route it rather than guessing wrong.
        _ => None,
    }
}

/// `format` 是对象 `{"type": ...}`，取值只有 mp3 / pcm / wav。
/// 返回 (format type, 响应 mime)。
fn resolve_format(extra: &HashMap<String, String>) -> (&'static str, &'static str) {
    match extra
        .get("format")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => ("wav", "audio/wav"),
        // `pcm` is deliberately not offered: Hume returns headerless samples
        // and the artifact writer keys the file extension off the mime, so
        // the result would be saved as .mp3 and refuse to play.
        _ => ("mp3", "audio/mpeg"),
    }
}

fn resolve_speed(extra: &HashMap<String, String>) -> Option<f64> {
    extra
        .get("speed")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(SPEED_MIN, SPEED_MAX))
}

/// 按字符边界截断（`truncate_utf8` 按字节，会把中文的 5000 字符额度砍到约 1/3）。
fn clamp_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

fn build_body(
    text: &str,
    voice: Option<&str>,
    version: Option<&str>,
    format_type: &str,
    speed: Option<f64>,
) -> Value {
    // text 只能待在 utterances 里，没有顶层 text；voice / speed 同样是 per-utterance。
    let mut utterance = json!({ "text": clamp_chars(text, MAX_UTTERANCE_CHARS) });
    if let Some(v) = voice.filter(|s| !s.is_empty()) {
        // Hume 的 voice 既接受 name 也接受 id；我们解析出的是用户可读音色名，走 name。
        utterance["voice"] = json!({ "name": v, "provider": "HUME_AI" });
    }
    if let Some(s) = speed {
        utterance["speed"] = json!(s);
    }

    let mut body = json!({
        "utterances": [utterance],
        "format": { "type": format_type },
        "num_generations": 1,
    });
    if let Some(v) = version {
        body["version"] = json!(v);
    }
    if voice.is_none_or(|s| s.is_empty()) {
        // instant_mode 服务端默认 true，而该模式下 voice 是必填——没音色时不关掉它
        // 就是一次必然 400，所以显式降级为非 instant 让服务端自选音色。
        body["instant_mode"] = json!(false);
    }
    body
}

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    if params.kind != AudioKind::Speech {
        bail!(
            "Hume Octave only supports speech (requested {})",
            params.kind.as_str()
        );
    }

    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/v0/tts/file", base);

    let (format_type, mime) = resolve_format(params.extra);
    let body = build_body(
        params.prompt,
        params.voice,
        resolve_version(params.model),
        format_type,
        resolve_speed(params.extra),
    );

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;
    let resp = client
        .post(&url)
        // Hume 用自定义 header 鉴权，不是 Authorization: Bearer。
        .header("X-Hume-Api-Key", params.api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        bail!(
            "Hume TTS failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("Hume TTS returned empty audio");
    }
    crate::app_info!("tool", "audio", "Hume TTS produced {} bytes", data.len());
    Ok(AudioGenResult {
        data,
        mime: mime.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_maps_from_model_id_not_sent_as_model() {
        assert_eq!(resolve_version("octave-2"), Some("2"));
        assert_eq!(resolve_version("octave-1"), Some("1"));
        assert_eq!(resolve_version("octave"), None);

        let body = build_body("hi", None, resolve_version("octave-2"), "mp3", None);
        assert_eq!(body["version"], json!("2"));
        assert!(body.get("model").is_none());
    }

    #[test]
    fn body_nests_text_and_voice_in_utterances() {
        let body = build_body("hello", Some("Ava"), Some("1"), "wav", Some(1.25));
        let u = &body["utterances"][0];
        assert_eq!(u["text"], json!("hello"));
        assert_eq!(u["voice"], json!({ "name": "Ava", "provider": "HUME_AI" }));
        assert_eq!(u["speed"], json!(1.25));
        // format 是对象而非字符串；顶层无 text。
        assert_eq!(body["format"], json!({ "type": "wav" }));
        assert!(body.get("text").is_none());
        // 有音色时不触碰 instant_mode，保留服务端默认。
        assert!(body.get("instant_mode").is_none());
    }

    #[test]
    fn missing_voice_disables_instant_mode() {
        let body = build_body("hello", None, None, "mp3", None);
        assert_eq!(body["instant_mode"], json!(false));
        assert!(body["utterances"][0].get("voice").is_none());
        assert!(body.get("version").is_none());
    }

    #[test]
    fn format_and_speed_come_from_extra() {
        let mut extra = HashMap::new();
        assert_eq!(resolve_format(&extra), ("mp3", "audio/mpeg"));
        assert_eq!(resolve_speed(&extra), None);

        extra.insert("format".into(), "WAV".into());
        extra.insert("speed".into(), "9".into());
        assert_eq!(resolve_format(&extra), ("wav", "audio/wav"));
        assert_eq!(resolve_speed(&extra), Some(SPEED_MAX));

        // pcm is intentionally unsupported: headerless samples would be
        // saved with an .mp3 extension and refuse to play.
        extra.insert("format".into(), "pcm".into());
        assert_eq!(resolve_format(&extra), ("mp3", "audio/mpeg"));

        extra.insert("speed".into(), "nonsense".into());
        assert_eq!(resolve_speed(&extra), None);
    }

    #[test]
    fn text_truncates_on_char_boundary() {
        let long: String = "字".repeat(MAX_UTTERANCE_CHARS + 10);
        let clamped = clamp_chars(&long, MAX_UTTERANCE_CHARS);
        assert_eq!(clamped.chars().count(), MAX_UTTERANCE_CHARS);
        assert_eq!(clamp_chars("short", MAX_UTTERANCE_CHARS), "short");
    }
}
