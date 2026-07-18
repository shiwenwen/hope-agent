//! Unified failover executor for media generation.
//!
//! The single place where candidates are tried, capability-validated,
//! retried, SSRF-gated, and usage-accounted. All consumers — the
//! `image_generate` / `audio_generate` chat tools and the design space's
//! image / audio artifact paths — call [`execute_image`] / [`execute_audio`]
//! instead of rolling their own provider loops (the pre-refactor codebase
//! had three divergent copies).
//!
//! Loop shape (extracted from the retired `tools/image_generate/generate.rs`):
//! per candidate → lenient capability validation → up to 1 retry on
//! retryable errors (`failover::classify_error` + `retry_delay_ms`) → next
//! candidate; every attempt records a `model_usage` event.

use std::collections::HashMap;

use anyhow::{bail, Result};

use crate::truncate_utf8;

use super::adapters::{
    audio_adapter, image_adapter, AudioGenParams, AudioGenResult, ImageGenParams, ImageGenResult,
    InputImage,
};
use super::resolve::{resolve_candidates, validate_image_request, ImageRequestSpec};
use super::types::{
    AudioKind, MediaFunction, MediaGenConfig, MediaModelConfig, MediaProviderConfig,
};

const MAX_RETRIES_PER_CANDIDATE: u32 = 1;

/// Accounting context. `operation`/`source` follow the existing
/// `model_usage` conventions (`"tool.image_generate"`, `"design.audio"`, …).
#[derive(Debug, Clone, Default)]
pub struct UsageMeta {
    pub operation: &'static str,
    pub source: &'static str,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
}

/// Successful outcome + which candidate won and what was skipped on the way.
pub struct MediaExecOutcome<T> {
    pub result: T,
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    pub failover_log: Vec<String>,
}

/// One image generation request. `size`/`aspect_ratio`/`resolution` are the
/// caller's explicit asks; `None` falls back to `image_defaults`.
#[derive(Default)]
pub struct ImageRequest<'a> {
    pub prompt: &'a str,
    pub size: Option<&'a str>,
    pub n: u32,
    pub aspect_ratio: Option<&'a str>,
    pub resolution: Option<&'a str>,
    pub input_images: &'a [InputImage],
    pub mask: Option<&'a [u8]>,
    /// `"provider::model"` or bare model id; pins a single candidate.
    pub explicit_model: Option<&'a str>,
}

/// One audio generation request.
#[derive(Default)]
pub struct AudioRequest<'a> {
    pub prompt: &'a str,
    pub kind: Option<AudioKind>,
    /// Call-level voice override (highest tier of the voice cascade).
    pub voice: Option<&'a str>,
    pub duration_seconds: Option<f64>,
    pub explicit_model: Option<&'a str>,
}

impl<'a> AudioRequest<'a> {
    pub fn effective_kind(&self) -> AudioKind {
        self.kind.unwrap_or(AudioKind::Speech)
    }
}

/// Merged extra map: provider-level ← model-level (model wins).
fn merged_extra(
    provider: &MediaProviderConfig,
    model: &MediaModelConfig,
) -> HashMap<String, String> {
    let mut extra = provider.extra.clone();
    for (k, v) in &model.extra {
        extra.insert(k.clone(), v.clone());
    }
    extra
}

/// Base-endpoint SSRF gate, one per candidate. Audio adapters additionally
/// check their final URLs with the same policy; image adapters rely on this
/// single choke point (their sub-paths share the base host).
async fn check_candidate_ssrf(provider: &MediaProviderConfig) -> Result<()> {
    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(
        provider.effective_base_url(),
        provider.ssrf_policy(),
        &cfg.ssrf.trusted_hosts,
    )
    .await
    .map(|_| ())
}

fn record_usage(
    kind: &'static str,
    usage: &UsageMeta,
    provider: &MediaProviderConfig,
    model_id: &str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    metadata: serde_json::Value,
) {
    let mut event = crate::model_usage::ModelUsageEvent::new(kind);
    event.operation = Some(usage.operation.to_string());
    event.source = Some(usage.source.to_string());
    event.provider_id = Some(provider.id.clone());
    event.provider_name = Some(provider.name.clone());
    event.model_id = Some(model_id.to_string());
    event.session_id = usage.session_id.clone();
    event.agent_id = usage.agent_id.clone();
    event.duration_ms = Some(duration_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(metadata);
    crate::model_usage::record_model_usage_best_effort(event);
}

pub async fn execute_image(
    cfg: &MediaGenConfig,
    req: ImageRequest<'_>,
    usage: UsageMeta,
) -> Result<MediaExecOutcome<ImageGenResult>> {
    let candidates = resolve_candidates(cfg, MediaFunction::Image, req.explicit_model)?;
    let defaults = &cfg.image_defaults;
    let timeout = defaults.effective_timeout_secs();

    let size = req.size.unwrap_or(&defaults.default_size);
    let aspect_ratio = req
        .aspect_ratio
        .or(defaults.default_aspect_ratio.as_deref());
    let resolution = req.resolution.or(defaults.default_resolution.as_deref());
    let n = req.n.max(1);
    let is_edit = !req.input_images.is_empty();

    // Capability spec: size only counts as a constraint when it differs
    // from the global default (mirrors the old validate semantics — a
    // size-less provider still serves default-size requests).
    let spec = ImageRequestSpec {
        size: Some(size).filter(|s| **s != defaults.default_size),
        aspect_ratio,
        resolution,
        n,
        input_images: req.input_images.len(),
        has_mask: req.mask.is_some(),
    };

    let mut failover_log: Vec<String> = Vec::new();
    let mut last_error = String::new();

    for candidate in &candidates {
        let provider = candidate.provider;
        let model = candidate.model;
        let label = candidate.label();

        if let Err(reason) = validate_image_request(model.image.as_ref(), &spec) {
            failover_log.push(format!("{label} skipped: {reason}"));
            app_info!(
                "media_gen",
                "execute",
                "{label} skipped (capability mismatch): {reason}"
            );
            continue;
        }

        let Some(adapter) = image_adapter(provider.kind) else {
            failover_log.push(format!("{label} skipped: vendor has no image adapter"));
            continue;
        };

        if let Err(e) = check_candidate_ssrf(provider).await {
            failover_log.push(format!("{label} skipped: endpoint blocked ({e})"));
            app_warn!("media_gen", "execute", "{label} endpoint blocked: {e}");
            continue;
        }

        let extra = merged_extra(provider, model);
        app_info!(
            "media_gen",
            "execute",
            "image [{label}]: prompt='{}', size={size}, n={n}, edit={is_edit}, ar={aspect_ratio:?}, res={resolution:?}",
            truncate_utf8(req.prompt, 80)
        );

        for attempt in 0..=MAX_RETRIES_PER_CANDIDATE {
            let params = ImageGenParams {
                api_key: &provider.api_key,
                base_url: Some(provider.effective_base_url()),
                model: &model.id,
                prompt: req.prompt,
                size,
                n,
                timeout_secs: timeout,
                extra: &extra,
                aspect_ratio,
                resolution,
                input_images: req.input_images,
                mask: req.mask,
                ssrf: provider.ssrf_policy(),
            };
            let started = std::time::Instant::now();
            match adapter.generate(params).await {
                Ok(result) => {
                    record_usage(
                        crate::model_usage::KIND_IMAGE_GENERATION,
                        &usage,
                        provider,
                        &model.id,
                        started.elapsed().as_millis() as u64,
                        true,
                        None,
                        serde_json::json!({
                            "size": size,
                            "n": n,
                            "aspect_ratio": aspect_ratio,
                            "resolution": resolution,
                            "is_edit": is_edit,
                            "input_image_count": req.input_images.len(),
                            "attempt": attempt,
                            "output_image_count": result.images.len(),
                        }),
                    );
                    return Ok(MediaExecOutcome {
                        result,
                        provider_id: provider.id.clone(),
                        provider_name: provider.name.clone(),
                        model_id: model.id.clone(),
                        failover_log,
                    });
                }
                Err(e) => {
                    let err_string = e.to_string();
                    record_usage(
                        crate::model_usage::KIND_IMAGE_GENERATION,
                        &usage,
                        provider,
                        &model.id,
                        started.elapsed().as_millis() as u64,
                        false,
                        Some(err_string.clone()),
                        serde_json::json!({
                            "size": size,
                            "n": n,
                            "aspect_ratio": aspect_ratio,
                            "resolution": resolution,
                            "is_edit": is_edit,
                            "input_image_count": req.input_images.len(),
                            "attempt": attempt,
                        }),
                    );
                    if !handle_attempt_error(
                        &label,
                        &err_string,
                        attempt,
                        &mut failover_log,
                        &mut last_error,
                    )
                    .await
                    {
                        break; // → next candidate
                    }
                }
            }
        }
    }

    let log_summary = failover_log.join("\n");
    bail!(
        "All image generation candidates failed.\n{}\nLast error: {}",
        log_summary,
        truncate_utf8(&last_error, 300)
    )
}

pub async fn execute_audio(
    cfg: &MediaGenConfig,
    req: AudioRequest<'_>,
    usage: UsageMeta,
) -> Result<MediaExecOutcome<AudioGenResult>> {
    let kind = req.effective_kind();
    let candidates = resolve_candidates(cfg, MediaFunction::Audio(kind), req.explicit_model)?;
    let defaults = &cfg.audio_defaults;
    let timeout = defaults.effective_timeout_secs();
    let duration_seconds = req.duration_seconds.or(defaults.default_duration_secs);

    let mut failover_log: Vec<String> = Vec::new();
    let mut last_error = String::new();

    for candidate in &candidates {
        let provider = candidate.provider;
        let model = candidate.model;
        let label = candidate.label();

        let Some(adapter) = audio_adapter(provider.kind) else {
            failover_log.push(format!("{label} skipped: vendor has no audio adapter"));
            continue;
        };

        if let Err(e) = check_candidate_ssrf(provider).await {
            failover_log.push(format!("{label} skipped: endpoint blocked ({e})"));
            app_warn!("media_gen", "execute", "{label} endpoint blocked: {e}");
            continue;
        }

        // Voice cascade: call-level → model default → provider default.
        // Adapter built-ins remain the last resort when this is None.
        let voice = req
            .voice
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                model
                    .audio
                    .as_ref()
                    .and_then(|caps| caps.default_voice.as_deref())
                    .filter(|v| !v.is_empty())
            })
            .or_else(|| provider.default_voice.as_deref().filter(|v| !v.is_empty()));

        let extra = merged_extra(provider, model);
        app_info!(
            "media_gen",
            "execute",
            "audio [{label}]: kind={}, duration={duration_seconds:?}, prompt='{}'",
            kind.as_str(),
            truncate_utf8(req.prompt, 80)
        );

        for attempt in 0..=MAX_RETRIES_PER_CANDIDATE {
            let params = AudioGenParams {
                api_key: &provider.api_key,
                base_url: Some(provider.effective_base_url()),
                model: &model.id,
                prompt: req.prompt,
                kind,
                timeout_secs: timeout,
                duration_seconds,
                voice,
                extra: &extra,
                ssrf: provider.ssrf_policy(),
            };
            let started = std::time::Instant::now();
            match adapter.generate(params).await {
                Ok(result) => {
                    record_usage(
                        crate::model_usage::KIND_AUDIO_GENERATION,
                        &usage,
                        provider,
                        &model.id,
                        started.elapsed().as_millis() as u64,
                        true,
                        None,
                        serde_json::json!({
                            "audio_kind": kind.as_str(),
                            "duration_seconds": duration_seconds,
                            "voice_set": voice.is_some(),
                            "attempt": attempt,
                        }),
                    );
                    return Ok(MediaExecOutcome {
                        result,
                        provider_id: provider.id.clone(),
                        provider_name: provider.name.clone(),
                        model_id: model.id.clone(),
                        failover_log,
                    });
                }
                Err(e) => {
                    let err_string = e.to_string();
                    record_usage(
                        crate::model_usage::KIND_AUDIO_GENERATION,
                        &usage,
                        provider,
                        &model.id,
                        started.elapsed().as_millis() as u64,
                        false,
                        Some(err_string.clone()),
                        serde_json::json!({
                            "audio_kind": kind.as_str(),
                            "duration_seconds": duration_seconds,
                            "voice_set": voice.is_some(),
                            "attempt": attempt,
                        }),
                    );
                    if !handle_attempt_error(
                        &label,
                        &err_string,
                        attempt,
                        &mut failover_log,
                        &mut last_error,
                    )
                    .await
                    {
                        break; // → next candidate
                    }
                }
            }
        }
    }

    let log_summary = failover_log.join("\n");
    bail!(
        "All audio generation candidates failed for `{}`.\n{}\nLast error: {}",
        kind.as_str(),
        log_summary,
        truncate_utf8(&last_error, 300)
    )
}

/// Shared per-attempt error handling. Returns `true` to retry the same
/// candidate (after sleeping the backoff), `false` to move on.
async fn handle_attempt_error(
    label: &str,
    err_string: &str,
    attempt: u32,
    failover_log: &mut Vec<String>,
    last_error: &mut String,
) -> bool {
    let reason = crate::failover::classify_error(err_string);
    let reason_label = format!("{:?}", reason);
    if reason.is_retryable() && attempt < MAX_RETRIES_PER_CANDIDATE {
        let delay = crate::failover::retry_delay_ms(attempt, 2000, 10000);
        failover_log.push(format!(
            "{label} failed ({reason_label}), retrying in {delay}ms..."
        ));
        app_warn!(
            "media_gen",
            "execute",
            "{label} failed ({reason_label}), retrying in {delay}ms"
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        return true;
    }
    let err_preview = truncate_utf8(err_string, 200);
    failover_log.push(format!("{label} failed ({reason_label}): {err_preview}"));
    *last_error = err_string.to_string();
    app_warn!(
        "media_gen",
        "execute",
        "{label} failed ({reason_label}): {err_preview}"
    );
    false
}
