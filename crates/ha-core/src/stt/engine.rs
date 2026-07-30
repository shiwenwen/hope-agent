//! STT engine trait, provider factory, and failover orchestration.

use std::sync::Arc;

use crate::config::cached_config;
use crate::provider::AuthProfile;

use super::errors::SttError;
use super::types::{
    ActiveSttModel, AudioPayload, SttModelConfig, SttProviderConfig, Transcript, TranscriptOptions,
};

/// Resolve an `ActiveSttModel` to a concrete `(provider, model, profile)`.
/// Returns the first enabled profile; legacy single-key providers
/// synthesise one via `effective_profiles`.
pub fn resolve_active(
    cfg: &crate::config::AppConfig,
    active: &ActiveSttModel,
) -> Option<(SttProviderConfig, SttModelConfig, AuthProfile)> {
    let provider = cfg
        .stt
        .providers
        .iter()
        .find(|p| p.id == active.provider_id && p.enabled)?
        .clone();
    let model = provider.model_config(&active.model_id)?.clone();
    let profile = provider.effective_profiles().into_iter().next()?;
    Some((provider, model, profile))
}

/// Result of a failover transcribe attempt — useful for telemetry / logs.
#[derive(Debug, Clone)]
pub struct AttemptedModel {
    pub provider_id: String,
    pub model_id: String,
    pub error_code: &'static str,
    pub error_message: String,
}

/// 批量转写 failover 链（真实现在 ha-media，经 `super::register_stt_transcriber`
/// 装配期注册；调用方：channel 语音消息 / knowledge 音频收集）。未接线
/// 返回 `NoActiveModel` 终态并 `app_warn` 审计——语义上等价于「无可用
/// 转写模型」，调用方的既有降级路径（保留原始音频/报错提示）原样生效。
pub async fn failover_transcribe_batch(
    primary: Option<ActiveSttModel>,
    fallback: Vec<ActiveSttModel>,
    audio: AudioPayload,
    options: &TranscriptOptions,
    session_id: Option<&str>,
) -> Result<Transcript, FailoverError> {
    let Some(transcriber) = super::stt_transcriber() else {
        crate::app_warn!(
            "stt",
            "transcribe_skipped",
            "ha-media not wired; STT transcription unavailable in this process"
        );
        return Err(FailoverError {
            attempts: Vec::new(),
            terminal: SttError::NoActiveModel,
        });
    };
    transcriber(
        primary,
        fallback,
        audio,
        options.clone(),
        session_id.map(str::to_string),
    )
    .await
}

#[derive(Debug)]
pub struct FailoverError {
    pub attempts: Vec<AttemptedModel>,
    pub terminal: SttError,
}

impl std::fmt::Display for FailoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.attempts.is_empty() {
            return write!(f, "{}", self.terminal);
        }
        let chain: Vec<String> = self
            .attempts
            .iter()
            .map(|a| format!("{}::{} → {}", a.provider_id, a.model_id, a.error_code))
            .collect();
        write!(
            f,
            "STT failover exhausted ({}): {}",
            chain.join(", "),
            self.terminal
        )
    }
}

impl std::error::Error for FailoverError {}

/// Snapshot the current desktop chain (active + fallback). Cheap clones from
/// `cached_config()`.
pub fn current_desktop_chain() -> (Option<ActiveSttModel>, Vec<ActiveSttModel>) {
    let cfg = cached_config();
    snapshot_chain(&cfg)
}

/// Snapshot the IM chain: prefer `im_fallback_model` over `active_model`.
/// When both are set, the desktop `active_model` is folded into the recovery
/// list (after `fallback_models`) so an IM-specific primary doesn't strand the
/// user's main model out of the chain. Duplicates are removed in order.
pub fn current_im_chain() -> (Option<ActiveSttModel>, Vec<ActiveSttModel>) {
    let cfg = cached_config();
    let im = cfg.stt.im_fallback_model.clone();
    let active = cfg.stt.active_model.clone();
    let (primary, extra_recovery) = match (im, active) {
        (Some(im_primary), Some(desktop_active)) => (Some(im_primary), Some(desktop_active)),
        (Some(im_primary), None) => (Some(im_primary), None),
        (None, active) => (active, None),
    };
    let mut recovery: Vec<ActiveSttModel> = cfg.stt.fallback_models.clone();
    if let Some(extra) = extra_recovery {
        if Some(&extra) != primary.as_ref() && !recovery.contains(&extra) {
            recovery.push(extra);
        }
    }
    (primary, recovery)
}

fn snapshot_chain(
    cfg: &Arc<crate::config::AppConfig>,
) -> (Option<ActiveSttModel>, Vec<ActiveSttModel>) {
    (
        cfg.stt.active_model.clone(),
        cfg.stt.fallback_models.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::stt::crud::add_stt_provider_in_config;
    use crate::stt::types::SttProviderKind;

    fn provider_with_model() -> SttProviderConfig {
        let mut p = SttProviderConfig::new(
            "OpenAI",
            SttProviderKind::OpenaiTranscriptions,
            "https://api.openai.com",
        );
        p.api_key = "sk-test".into();
        p.models.push(SttModelConfig::new("whisper-1", "Whisper"));
        p
    }

    #[test]
    fn resolve_active_finds_provider_model_profile() {
        let mut cfg = AppConfig::default();
        let inserted = add_stt_provider_in_config(&mut cfg, provider_with_model());
        let active = ActiveSttModel {
            provider_id: inserted.id.clone(),
            model_id: "whisper-1".into(),
        };
        let resolved = resolve_active(&cfg, &active);
        assert!(resolved.is_some());
        let (_, model, profile) = resolved.unwrap();
        assert_eq!(model.id, "whisper-1");
        assert_eq!(profile.api_key, "sk-test");
    }

    #[test]
    fn resolve_active_skips_disabled_provider() {
        let mut cfg = AppConfig::default();
        let mut p = provider_with_model();
        p.enabled = false;
        let inserted = add_stt_provider_in_config(&mut cfg, p);
        let active = ActiveSttModel {
            provider_id: inserted.id.clone(),
            model_id: "whisper-1".into(),
        };
        assert!(resolve_active(&cfg, &active).is_none());
    }

    #[test]
    fn failover_error_renders_chain() {
        let err = FailoverError {
            attempts: vec![
                AttemptedModel {
                    provider_id: "p1".into(),
                    model_id: "whisper-1".into(),
                    error_code: "network",
                    error_message: "boom".into(),
                },
                AttemptedModel {
                    provider_id: "p2".into(),
                    model_id: "nova-3".into(),
                    error_code: "auth",
                    error_message: "bad key".into(),
                },
            ],
            terminal: SttError::Auth("bad key".into()),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("p1::whisper-1"));
        assert!(rendered.contains("network"));
        assert!(rendered.contains("auth"));
    }
}
