//! Provider-aware, content-free request token diagnostics.
//!
//! The manifest stores only byte counts, estimates, and short BLAKE3
//! fingerprints. It is safe to persist in logs without copying prompts,
//! memories, tool arguments, or conversation text.

use std::num::NonZeroUsize;
use std::sync::{LazyLock, Mutex};

use serde::Serialize;

use super::streaming_adapter::RoundRequest;

const LAST_MANIFEST_CAPACITY: usize = 128;

static LAST_ROUND_MANIFESTS: LazyLock<Mutex<lru::LruCache<String, RoundTokenManifest>>> =
    LazyLock::new(|| {
        Mutex::new(lru::LruCache::new(
            NonZeroUsize::new(LAST_MANIFEST_CAPACITY).expect("manifest capacity is non-zero"),
        ))
    });

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoundTokenManifest {
    provider: &'static str,
    model: String,
    round: u32,
    request_shape: &'static str,
    stable_prompt_bytes: usize,
    stable_prompt_tokens_estimate: u32,
    stable_prompt_fingerprint: String,
    dynamic_prompt_bytes: usize,
    dynamic_prompt_tokens_estimate: u32,
    dynamic_prompt_fingerprint: String,
    history_bytes: usize,
    history_tokens_estimate: u32,
    history_fingerprint: String,
    tool_schema_bytes: usize,
    tool_schema_tokens_estimate: u32,
    tool_schema_fingerprint: String,
    eager_tool_schema_bytes: usize,
    eager_tool_schema_tokens_estimate: u32,
    eager_tool_schema_fingerprint: String,
    activated_tool_schema_bytes: usize,
    activated_tool_schema_tokens_estimate: u32,
    activated_tool_schema_fingerprint: String,
    deferred_tool_schema_bytes: usize,
    deferred_tool_schema_tokens_estimate: u32,
    deferred_tool_schema_fingerprint: String,
    eager_tool_count: usize,
    deferred_tool_count: usize,
    activated_tool_count: usize,
    cacheable_stable_tokens_estimate: u32,
    request_input_tokens_estimate: u32,
    transport_body_bytes: usize,
    native_deferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fresh_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttft_ms: Option<u64>,
}

/// Content-free view consumed by `/context`. All category fields are built by
/// the same request manifest as the adapter body; actual usage is populated
/// after the stream completes when the Provider reports it.
#[derive(Debug, Clone)]
pub(crate) struct RoundContextSnapshot {
    pub provider: String,
    pub model: String,
    pub stable_prompt_tokens_estimate: u32,
    pub dynamic_prompt_tokens_estimate: u32,
    pub history_tokens_estimate: u32,
    pub tool_schema_tokens_estimate: u32,
    pub eager_tool_schema_tokens_estimate: u32,
    pub activated_tool_schema_tokens_estimate: u32,
    pub deferred_tool_schema_tokens_estimate: u32,
    pub cacheable_stable_tokens_estimate: u32,
    pub request_input_tokens_estimate: u32,
    pub eager_tool_count: usize,
    pub deferred_tool_count: usize,
    pub activated_tool_count: usize,
    pub native_deferred: bool,
    pub stable_prompt_fingerprint: String,
    pub dynamic_prompt_fingerprint: String,
    pub context_input_tokens: Option<u64>,
    pub fresh_input_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub ttft_ms: Option<u64>,
}

fn token_estimate_text(value: &str) -> u32 {
    crate::system_prompt::conservative_core_token_estimate(value).min(u32::MAX as usize) as u32
}

fn fingerprint(value: &[u8]) -> String {
    blake3::hash(value).to_hex()[..16].to_string()
}

fn json_bytes(values: &[serde_json::Value]) -> usize {
    values
        .iter()
        .map(|value| {
            serde_json::to_vec(value)
                .map(|v| v.len())
                .unwrap_or_default()
        })
        .sum()
}

fn json_fingerprint(values: &[serde_json::Value]) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in values {
        if let Ok(encoded) = serde_json::to_vec(value) {
            hasher.update(&encoded);
        }
    }
    hasher.finalize().to_hex()[..16].to_string()
}

impl RoundTokenManifest {
    pub(crate) fn from_request(
        provider: &'static str,
        model: &str,
        request_shape: &'static str,
        req: &RoundRequest<'_>,
        transport_body_bytes: usize,
        native_deferred: bool,
    ) -> Self {
        let dynamic_parts = [
            req.awareness_suffix,
            req.active_memory_suffix,
            req.coding_profile_suffix,
            req.procedure_memory_suffix,
            req.related_notes_suffix,
            req.task_reminder_suffix,
        ];
        let mut dynamic_hasher = blake3::Hasher::new();
        let dynamic_values = dynamic_parts.iter().flatten().copied().collect::<Vec<_>>();
        let dynamic_prompt_bytes = dynamic_values
            .iter()
            .map(|value| {
                dynamic_hasher.update(value.as_bytes());
                value.len()
            })
            .sum();
        let history_bytes = json_bytes(req.history_for_api);
        let tool_schema_bytes = json_bytes(req.tool_schemas);
        let eager_end = req.eager_tool_count.min(req.tool_schemas.len());
        let eager_tools = &req.tool_schemas[..eager_end];
        let activated_tools = &req.tool_schemas[eager_end..];
        let eager_tool_schema_bytes = json_bytes(eager_tools);
        let activated_tool_schema_bytes = json_bytes(activated_tools);
        let deferred_tool_schema_bytes = json_bytes(req.deferred_tool_schemas);
        let stable_prompt_tokens_estimate = token_estimate_text(req.system_prompt);
        let tool_schema_tokens_estimate = req
            .tool_schemas
            .iter()
            .map(crate::context_compact::estimate_tokens)
            .sum();
        let eager_tool_schema_tokens_estimate = eager_tools
            .iter()
            .map(crate::context_compact::estimate_tokens)
            .sum();
        let activated_tool_schema_tokens_estimate = activated_tools
            .iter()
            .map(crate::context_compact::estimate_tokens)
            .sum();
        let deferred_tool_schema_tokens_estimate = req
            .deferred_tool_schemas
            .iter()
            .map(crate::context_compact::estimate_tokens)
            .sum();
        let dynamic_prompt_tokens_estimate = dynamic_values
            .iter()
            .map(|value| token_estimate_text(value))
            .sum();
        let history_tokens_estimate = req
            .history_for_api
            .iter()
            .map(crate::context_compact::estimate_tokens)
            .sum();
        let cacheable_stable_tokens_estimate =
            stable_prompt_tokens_estimate.saturating_add(eager_tool_schema_tokens_estimate);
        let request_input_tokens_estimate = cacheable_stable_tokens_estimate
            .saturating_add(activated_tool_schema_tokens_estimate)
            .saturating_add(dynamic_prompt_tokens_estimate)
            .saturating_add(history_tokens_estimate);

        Self {
            provider,
            model: model.to_string(),
            round: req.round,
            request_shape,
            stable_prompt_bytes: req.system_prompt.len(),
            stable_prompt_tokens_estimate,
            stable_prompt_fingerprint: fingerprint(req.system_prompt.as_bytes()),
            dynamic_prompt_bytes,
            dynamic_prompt_tokens_estimate,
            dynamic_prompt_fingerprint: dynamic_hasher.finalize().to_hex()[..16].to_string(),
            history_bytes,
            history_tokens_estimate,
            history_fingerprint: json_fingerprint(req.history_for_api),
            tool_schema_bytes,
            tool_schema_tokens_estimate,
            tool_schema_fingerprint: json_fingerprint(req.tool_schemas),
            eager_tool_schema_bytes,
            eager_tool_schema_tokens_estimate,
            eager_tool_schema_fingerprint: json_fingerprint(eager_tools),
            activated_tool_schema_bytes,
            activated_tool_schema_tokens_estimate,
            activated_tool_schema_fingerprint: json_fingerprint(activated_tools),
            deferred_tool_schema_bytes,
            deferred_tool_schema_tokens_estimate,
            deferred_tool_schema_fingerprint: json_fingerprint(req.deferred_tool_schemas),
            eager_tool_count: req.eager_tool_count,
            deferred_tool_count: req.deferred_tool_count,
            activated_tool_count: req.activated_tool_count,
            cacheable_stable_tokens_estimate,
            request_input_tokens_estimate,
            transport_body_bytes,
            native_deferred,
            context_input_tokens: None,
            fresh_input_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            ttft_ms: None,
        }
    }

    fn context_snapshot(&self) -> RoundContextSnapshot {
        RoundContextSnapshot {
            provider: self.provider.to_string(),
            model: self.model.clone(),
            stable_prompt_tokens_estimate: self.stable_prompt_tokens_estimate,
            dynamic_prompt_tokens_estimate: self.dynamic_prompt_tokens_estimate,
            history_tokens_estimate: self.history_tokens_estimate,
            tool_schema_tokens_estimate: self.tool_schema_tokens_estimate,
            eager_tool_schema_tokens_estimate: self.eager_tool_schema_tokens_estimate,
            activated_tool_schema_tokens_estimate: self.activated_tool_schema_tokens_estimate,
            deferred_tool_schema_tokens_estimate: self.deferred_tool_schema_tokens_estimate,
            cacheable_stable_tokens_estimate: self.cacheable_stable_tokens_estimate,
            request_input_tokens_estimate: self.request_input_tokens_estimate,
            eager_tool_count: self.eager_tool_count,
            deferred_tool_count: self.deferred_tool_count,
            activated_tool_count: self.activated_tool_count,
            native_deferred: self.native_deferred,
            stable_prompt_fingerprint: self.stable_prompt_fingerprint.clone(),
            dynamic_prompt_fingerprint: self.dynamic_prompt_fingerprint.clone(),
            context_input_tokens: self.context_input_tokens,
            fresh_input_tokens: self.fresh_input_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
            output_tokens: self.output_tokens,
            ttft_ms: self.ttft_ms,
        }
    }

    pub(crate) fn log(&self) {
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "info",
                "agent",
                "agent::round_token_manifest",
                &format!(
                    "{} round {}: estimated {} input tokens ({} eager / {} deferred / {} activated)",
                    self.provider,
                    self.round,
                    self.request_input_tokens_estimate,
                    self.eager_tool_count,
                    self.deferred_tool_count,
                    self.activated_tool_count,
                ),
                serde_json::to_string(self).ok(),
                None,
                None,
            );
        }
    }
}

pub(crate) fn log_round_manifest(
    provider: &'static str,
    model: &str,
    request_shape: &'static str,
    req: &RoundRequest<'_>,
    transport_body_bytes: usize,
    native_deferred: bool,
) {
    let manifest = RoundTokenManifest::from_request(
        provider,
        model,
        request_shape,
        req,
        transport_body_bytes,
        native_deferred,
    );
    if let Some(session_id) = req.session_id {
        LAST_ROUND_MANIFESTS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .put(session_id.to_string(), manifest.clone());
    }
    manifest.log();
}

pub(crate) fn latest_round_context(session_id: &str) -> Option<RoundContextSnapshot> {
    LAST_ROUND_MANIFESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(session_id)
        .map(RoundTokenManifest::context_snapshot)
}

pub(crate) fn invalidate_round_context(session_id: &str) {
    LAST_ROUND_MANIFESTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .pop(session_id);
}

/// Complete the request-side manifest with the provider's authoritative
/// usage counters. Kept as a separate correlated log row because streaming
/// adapters only know usage/TTFT after the request manifest has been emitted.
pub(crate) fn log_round_usage(
    provider: &'static str,
    model: &str,
    round: u32,
    session_id: Option<&str>,
    usage: &super::types::ChatUsage,
    ttft_ms: Option<u64>,
) {
    if let Some(session_id) = session_id {
        let mut manifests = LAST_ROUND_MANIFESTS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(manifest) = manifests.get_mut(session_id).filter(|manifest| {
            manifest.provider == provider && manifest.model == model && manifest.round == round
        }) {
            let has_input_usage = usage.context_input_tokens > 0;
            manifest.context_input_tokens = has_input_usage.then_some(usage.context_input_tokens);
            manifest.fresh_input_tokens = has_input_usage.then_some(usage.fresh_input_tokens);
            manifest.cache_read_tokens = has_input_usage.then_some(usage.cache_read_input_tokens);
            manifest.cache_write_tokens =
                has_input_usage.then_some(usage.cache_creation_input_tokens);
            manifest.output_tokens = (usage.output_tokens > 0).then_some(usage.output_tokens);
            manifest.ttft_ms = ttft_ms;
        }
    }
    if let Some(logger) = crate::get_logger() {
        let details = serde_json::json!({
            "provider": provider,
            "model": model,
            "round": round,
            "contextInputTokens": usage.context_input_tokens,
            "freshInputTokens": usage.fresh_input_tokens,
            "cacheReadTokens": usage.cache_read_input_tokens,
            "cacheWriteTokens": usage.cache_creation_input_tokens,
            "outputTokens": usage.output_tokens,
            "ttftMs": ttft_ms,
        });
        logger.log(
            "info",
            "agent",
            "agent::round_token_usage",
            &format!(
                "{} round {}: actual {} context / {} fresh / {} cache-read tokens",
                provider,
                round,
                usage.context_input_tokens,
                usage.fresh_input_tokens,
                usage.cache_read_input_tokens,
            ),
            Some(details.to_string()),
            None,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::RoundTokenManifest;
    use crate::agent::streaming_adapter::RoundRequest;

    #[test]
    fn legacy_empty_hi_baseline_is_locked() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/context/empty_hi_legacy_32738.json"
        ))
        .unwrap();
        assert_eq!(fixture["observedUsage"]["inputTokens"], 32_738);
        assert_eq!(fixture["historyMessagesBeforeTurn"], 0);
        assert_eq!(fixture["v2Acceptance"]["contextInputTokensMax"], 10_000);
        assert_eq!(fixture["v2Acceptance"]["eagerToolSchemaTokensMax"], 4_000);
        assert_eq!(fixture["v2Acceptance"]["staticPromptTokensMax"], 6_000);
    }

    #[test]
    fn dynamic_suffix_and_activation_do_not_change_stable_fingerprint() {
        let eager = vec![serde_json::json!({ "type": "function", "name": "read" })];
        let with_activation = vec![
            serde_json::json!({ "type": "function", "name": "read" }),
            serde_json::json!({ "type": "function", "name": "browser__snapshot" }),
        ];
        let deferred = Vec::new();
        let history = vec![serde_json::json!({ "role": "user", "content": "hi" })];
        let make = |tools: &[serde_json::Value], dynamic: &str| {
            let req = RoundRequest {
                session_id: Some("session"),
                system_prompt: "stable system",
                awareness_suffix: Some(dynamic),
                active_memory_suffix: None,
                coding_profile_suffix: None,
                procedure_memory_suffix: None,
                related_notes_suffix: None,
                task_reminder_suffix: None,
                tool_schemas: tools,
                deferred_tool_schemas: &deferred,
                eager_tool_count: 1,
                deferred_tool_count: 0,
                activated_tool_count: tools.len().saturating_sub(1),
                prompt_cache_key: Some("stable-key"),
                history_for_api: &history,
                reasoning_effort: None,
                temperature: None,
                max_tokens: 100,
                is_final_round: false,
                round: 0,
            };
            RoundTokenManifest::from_request("test", "model", "shape", &req, 0, false)
        };
        let first = make(&eager, "dynamic-a");
        let second = make(&with_activation, "dynamic-b");
        assert_eq!(
            first.stable_prompt_fingerprint,
            second.stable_prompt_fingerprint
        );
        assert_eq!(
            first.eager_tool_schema_fingerprint,
            second.eager_tool_schema_fingerprint
        );
        assert_eq!(
            first.cacheable_stable_tokens_estimate,
            second.cacheable_stable_tokens_estimate
        );
        assert_ne!(
            first.dynamic_prompt_fingerprint,
            second.dynamic_prompt_fingerprint
        );
        assert!(second.activated_tool_schema_tokens_estimate > 0);
    }
}
