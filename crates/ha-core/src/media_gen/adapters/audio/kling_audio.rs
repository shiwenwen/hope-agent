//! Kuaishou Kling audio: TTS (`/v1/audio/tts`) and sound effects
//! (`/v1/audio/text-to-audio`). Both are async submit + poll jobs sharing the
//! same envelope as the Kling image wire: `{code, message, data:{task_id,
//! task_status, task_result}}`, with `code != 0` meaning a business error even
//! on HTTP 200.
//!
//! Auth: static `Authorization: Bearer <API_KEY>` only. Kling also documents
//! an AK/SK → JWT (HS256) scheme, but the official note scopes it to the
//! *legacy* API design and the sibling image adapter exports no shared signer,
//! so duplicating a JWT implementation here would be an unverifiable second
//! copy. Users configure the console-issued API key, which the vendor states
//! applies to all models.
//!
//! Kling has **no music generation** endpoint (only TTS and 3–10s SFX), so
//! `AudioKind::Music` is rejected up front rather than mapped onto SFX.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::media_gen::adapters::fetch::fetch_asset;
use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::types::AudioKind;

/// International endpoint. Kling runs region-split domains (`api-singapore`
/// vs `api-beijing`) with no universal host, so a mainland account must set
/// `base_url` explicitly — this is only the fallback when config leaves it
/// empty.
const DEFAULT_BASE_URL: &str = "https://api-singapore.klingai.com";

const TTS_PATH: &str = "/v1/audio/tts";
const SFX_PATH: &str = "/v1/audio/text-to-audio";

const TTS_MAX_CHARS: usize = 1000;
const SFX_MAX_CHARS: usize = 200;

const SFX_MIN_DURATION: f64 = 3.0;
const SFX_MAX_DURATION: f64 = 10.0;
const SFX_DEFAULT_DURATION: f64 = 5.0;

const VOICE_SPEED_MIN: f64 = 0.8;
const VOICE_SPEED_MAX: f64 = 2.0;

const POLL_INITIAL_MS: u64 = 1000;
const POLL_STEP_MS: u64 = 1000;
const POLL_MAX_MS: u64 = 3000;

const FALLBACK_MIME: &str = "audio/mpeg";

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

// ── Wire types ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Envelope {
    code: Option<i64>,
    message: Option<String>,
    data: Option<TaskData>,
}

#[derive(Deserialize)]
struct TaskData {
    task_id: Option<String>,
    task_status: Option<String>,
    /// Carries the reason on failure — without it every failed task logs
    /// identically regardless of cause.
    task_status_msg: Option<String>,
    task_result: Option<Value>,
}

// ── Pure helpers ──────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TaskState {
    Pending,
    Succeeded,
    Failed,
    Unknown,
}

fn classify_status(status: &str) -> TaskState {
    match status.trim().to_ascii_lowercase().as_str() {
        "submitted" | "processing" | "pending" | "running" | "queued" => TaskState::Pending,
        "succeed" | "succeeded" | "success" => TaskState::Succeeded,
        "failed" | "failure" | "error" => TaskState::Failed,
        _ => TaskState::Unknown,
    }
}

/// SFX duration is a *required* field with a documented 3.0–10.0s range at one
/// decimal place; anything outside is a server-side 400.
fn clamp_sfx_duration(requested: Option<f64>) -> f64 {
    let raw = requested
        .filter(|d| d.is_finite())
        .unwrap_or(SFX_DEFAULT_DURATION);
    let clamped = raw.clamp(SFX_MIN_DURATION, SFX_MAX_DURATION);
    (clamped * 10.0).round() / 10.0
}

/// `voice_language` accepts only `zh` / `en`. An unset value defaults to
/// `zh`; an explicitly *wrong* value is rejected rather than coerced, because
/// silently returning Chinese audio for `voice_language=ja` is
/// indistinguishable from the request having been honoured.
fn resolve_voice_language(extra: &HashMap<String, String>) -> Result<&'static str> {
    match extra
        .get("voice_language")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("zh") => Ok("zh"),
        Some("en") => Ok("en"),
        Some(other) => anyhow::bail!("Kling TTS supports voice_language zh or en, got {other:?}"),
    }
}

/// Only forwarded when the user actually set it — omitting lets Kling apply
/// its own default rather than us guessing one.
fn resolve_voice_speed(extra: &HashMap<String, String>) -> Option<f64> {
    let raw: f64 = extra.get("voice_speed")?.trim().parse().ok()?;
    if !raw.is_finite() {
        return None;
    }
    Some(raw.clamp(VOICE_SPEED_MIN, VOICE_SPEED_MAX))
}

fn build_tts_body(
    text: &str,
    voice_id: &str,
    voice_language: &str,
    voice_speed: Option<f64>,
) -> Value {
    // No `model` / `model_name`: Kling publishes no audio model ids and the
    // documented TTS body has no such field.
    let mut body = serde_json::json!({
        "text": text,
        "voice_id": voice_id,
        "voice_language": voice_language,
    });
    if let Some(speed) = voice_speed {
        body["voice_speed"] = serde_json::json!(speed);
    }
    body
}

fn build_sfx_body(prompt: &str, duration: f64) -> Value {
    serde_json::json!({
        "prompt": prompt,
        "duration": duration,
    })
}

/// Kling documents `task_result` per-modality (images are
/// `task_result.images[].url`) but publishes no audio-side field name, so we
/// walk the object for the first http(s) string under a URL-ish key instead of
/// inventing a schema.
fn extract_audio_url(result: &Value) -> Option<String> {
    const URL_KEYS: &[&str] = &["url", "audio_url", "audio_file", "audio", "file_url"];

    match result {
        Value::Object(map) => {
            for key in URL_KEYS {
                if let Some(Value::String(s)) = map.get(*key) {
                    if is_http_url(s) {
                        return Some(s.clone());
                    }
                }
            }
            map.values().find_map(extract_audio_url)
        }
        Value::Array(items) => items.iter().find_map(extract_audio_url),
        _ => None,
    }
}

fn is_http_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://")
}

fn char_count_over(s: &str, max: usize) -> bool {
    s.chars().count() > max
}

// ── Implementation ────────────────────────────────────────────────

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    let base = params
        .base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let (path, body) = match params.kind {
        AudioKind::Speech => {
            let voice_id = params
                .voice
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Kling TTS requires a voice_id — set a default voice on the model or \
                         provider (list them via GET /v1/general/presets-voices)"
                    )
                })?;

            if params.prompt.trim().is_empty() {
                anyhow::bail!("Kling TTS: text is empty");
            }
            if char_count_over(params.prompt, TTS_MAX_CHARS) {
                anyhow::bail!(
                    "Kling TTS: text is {} characters, exceeding the {}-character limit",
                    params.prompt.chars().count(),
                    TTS_MAX_CHARS
                );
            }

            let language = resolve_voice_language(params.extra)?;
            let speed = resolve_voice_speed(params.extra);
            (
                TTS_PATH,
                build_tts_body(params.prompt, voice_id, language, speed),
            )
        }
        AudioKind::Sfx => {
            if params.prompt.trim().is_empty() {
                anyhow::bail!("Kling sound effects: prompt is empty");
            }
            if char_count_over(params.prompt, SFX_MAX_CHARS) {
                anyhow::bail!(
                    "Kling sound effects: prompt is {} characters, exceeding the {}-character limit",
                    params.prompt.chars().count(),
                    SFX_MAX_CHARS
                );
            }
            let duration = clamp_sfx_duration(params.duration_seconds);
            (SFX_PATH, build_sfx_body(params.prompt, duration))
        }
        AudioKind::Music => anyhow::bail!(
            "Kling has no music generation API — it offers TTS and 3–10s sound effects only"
        ),
    };

    let submit_url = format!("{}{}", base, path);
    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(&submit_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    app_info!(
        "tool",
        "audio_generate",
        "Kling audio submit: kind={}, path={}",
        params.kind.as_str(),
        path
    );

    let started = Instant::now();
    let deadline = started + Duration::from_secs(params.timeout_secs);

    let resp = client
        .post(&submit_url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "Kling audio submit failed ({}): {}",
            status.as_u16(),
            crate::truncate_utf8(&text, 300)
        );
    }

    let envelope: Envelope = serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!(
            "Kling audio: unparseable submit response ({}): {}",
            e,
            crate::truncate_utf8(&text, 300)
        )
    })?;
    let data = check_envelope(envelope, "submit")?;

    // Some Kling audio jobs can already carry their result on the submit
    // response; only fall through to polling when they don't.
    if let Some(url) = data.task_result.as_ref().and_then(extract_audio_url) {
        return download(&url, &params).await;
    }

    let task_id = data
        .task_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Kling audio: submit response carried no task_id"))?;

    let poll_url = format!("{}{}/{}", base, path, task_id);
    crate::security::ssrf::check_url(&poll_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let mut poll_interval_ms = POLL_INITIAL_MS;
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!(
                "Kling audio task timed out after {}s (task_id={})",
                params.timeout_secs,
                task_id
            );
        }
        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;

        let poll_resp = client
            .get(&poll_url)
            .header("Authorization", format!("Bearer {}", params.api_key))
            .send()
            .await?;

        let poll_status = poll_resp.status();
        let poll_text = poll_resp.text().await.unwrap_or_default();
        if !poll_status.is_success() {
            // The SFX query sub-path is documented; the TTS one is inferred by
            // symmetry. A 404 here most likely means that inference is wrong,
            // so say so rather than leaving a bare status code — the task did
            // submit successfully and the audio may well exist elsewhere.
            if poll_status.as_u16() == 404 && path == TTS_PATH {
                anyhow::bail!(
                    "Kling TTS poll returned 404 for {} — the TTS task-query path is not \
                     documented and may differ from the sound-effect one; the task itself was \
                     submitted (task_id={})",
                    poll_url,
                    task_id
                );
            }
            anyhow::bail!(
                "Kling audio poll failed ({}, task_id={}): {}",
                poll_status.as_u16(),
                task_id,
                crate::truncate_utf8(&poll_text, 300)
            );
        }

        let envelope: Envelope = serde_json::from_str(&poll_text).map_err(|e| {
            anyhow::anyhow!(
                "Kling audio: unparseable poll response ({}): {}",
                e,
                crate::truncate_utf8(&poll_text, 300)
            )
        })?;
        let data = check_envelope(envelope, "poll")?;
        let task_status = data.task_status.as_deref().unwrap_or("unknown");

        match classify_status(task_status) {
            TaskState::Succeeded => {
                let url = data
                    .task_result
                    .as_ref()
                    .and_then(extract_audio_url)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Kling audio task succeeded but carried no audio URL (task_id={})",
                            task_id
                        )
                    })?;
                app_info!(
                    "tool",
                    "audio_generate",
                    "Kling audio task completed in {}ms (task_id={})",
                    started.elapsed().as_millis() as u64,
                    task_id
                );
                return download(&url, &params).await;
            }
            TaskState::Failed => {
                anyhow::bail!(
                    "Kling audio task failed (task_id={}): {}",
                    task_id,
                    data.task_status_msg
                        .as_deref()
                        .map(|m| crate::truncate_utf8(m, 300).to_string())
                        .unwrap_or_else(|| "no reason reported".to_string())
                );
            }
            TaskState::Unknown => {
                app_warn!(
                    "tool",
                    "audio_generate",
                    "Kling audio unknown task status: {} (task_id={})",
                    task_status,
                    task_id
                );
                poll_interval_ms = (poll_interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);
            }
            TaskState::Pending => {
                poll_interval_ms = (poll_interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);
            }
        }
    }
}

/// `code != 0` is a business error even under HTTP 200.
fn check_envelope(envelope: Envelope, stage: &str) -> Result<TaskData> {
    let code = envelope.code.unwrap_or(0);
    if code != 0 {
        let msg = envelope.message.unwrap_or_default();
        anyhow::bail!(
            "Kling audio {} error (code={}): {}",
            stage,
            code,
            crate::truncate_utf8(&msg, 300)
        );
    }
    envelope
        .data
        .ok_or_else(|| anyhow::anyhow!("Kling audio: {} response carried no data", stage))
}

async fn download(url: &str, params: &AudioGenParams<'_>) -> Result<AudioGenResult> {
    let (data, mime) = fetch_asset(url, params.ssrf, FALLBACK_MIME).await?;
    if data.is_empty() {
        anyhow::bail!("Kling audio: downloaded asset was empty");
    }
    Ok(AudioGenResult { data, mime })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfx_duration_clamps_to_legal_range() {
        assert_eq!(clamp_sfx_duration(None), SFX_DEFAULT_DURATION);
        assert_eq!(clamp_sfx_duration(Some(0.5)), SFX_MIN_DURATION);
        assert_eq!(clamp_sfx_duration(Some(30.0)), SFX_MAX_DURATION);
        assert_eq!(clamp_sfx_duration(Some(f64::NAN)), SFX_DEFAULT_DURATION);
        // One decimal place, per the documented format.
        assert_eq!(clamp_sfx_duration(Some(7.26)), 7.3);
    }

    #[test]
    fn tts_body_omits_model_and_optional_speed() {
        let body = build_tts_body("你好", "voice-1", "zh", None);
        assert_eq!(body["text"], "你好");
        assert_eq!(body["voice_id"], "voice-1");
        assert_eq!(body["voice_language"], "zh");
        assert!(body.get("voice_speed").is_none());
        // Kling publishes no audio model ids; sending one is a 400.
        assert!(body.get("model").is_none());
        assert!(body.get("model_name").is_none());

        let with_speed = build_tts_body("hi", "v", "en", Some(1.5));
        assert_eq!(with_speed["voice_speed"], 1.5);
    }

    #[test]
    fn voice_extras_default_and_clamp() {
        let mut extra = HashMap::new();
        assert_eq!(resolve_voice_language(&extra).unwrap(), "zh");
        assert_eq!(resolve_voice_speed(&extra), None);

        extra.insert("voice_language".into(), " EN ".into());
        extra.insert("voice_speed".into(), "9.9".into());
        assert_eq!(resolve_voice_language(&extra).unwrap(), "en");
        assert_eq!(resolve_voice_speed(&extra), Some(VOICE_SPEED_MAX));

        // An unsupported language is an error, not a silent fallback to zh:
        // Chinese audio for `voice_language=ja` looks like a honoured request.
        extra.insert("voice_language".into(), "ja".into());
        extra.insert("voice_speed".into(), "not-a-number".into());
        assert!(resolve_voice_language(&extra).is_err());
        assert_eq!(resolve_voice_speed(&extra), None);

        // Empty still means "unset" → documented zh default.
        extra.insert("voice_language".into(), "  ".into());
        assert_eq!(resolve_voice_language(&extra).unwrap(), "zh");
    }

    #[test]
    fn status_classification_covers_kling_terminal_states() {
        assert_eq!(classify_status("submitted"), TaskState::Pending);
        assert_eq!(classify_status("processing"), TaskState::Pending);
        assert_eq!(classify_status("succeed"), TaskState::Succeeded);
        assert_eq!(classify_status("SUCCEED"), TaskState::Succeeded);
        assert_eq!(classify_status("failed"), TaskState::Failed);
        assert_eq!(classify_status("weird"), TaskState::Unknown);
    }

    #[test]
    fn audio_url_extraction_walks_nested_result() {
        let nested = serde_json::json!({
            "audios": [{ "index": 0, "url": "https://cdn.example.com/a.mp3" }]
        });
        assert_eq!(
            extract_audio_url(&nested).as_deref(),
            Some("https://cdn.example.com/a.mp3")
        );

        let flat = serde_json::json!({ "audio_url": "http://x/y.wav" });
        assert_eq!(extract_audio_url(&flat).as_deref(), Some("http://x/y.wav"));

        // Non-URL strings must not be mistaken for assets.
        let bogus = serde_json::json!({ "url": "not-a-url", "id": "abc" });
        assert_eq!(extract_audio_url(&bogus), None);
    }

    #[test]
    fn sfx_body_carries_required_duration() {
        let body = build_sfx_body("thunder", clamp_sfx_duration(Some(12.0)));
        assert_eq!(body["prompt"], "thunder");
        assert_eq!(body["duration"], 10.0);
    }
}
