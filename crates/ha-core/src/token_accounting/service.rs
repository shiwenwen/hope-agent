use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Context;
use serde_json::Value;

use super::heuristic;
use super::{
    CalibrationKey, CapacityProofError, PartCacheKind, PreflightCapacityProof,
    ProviderCountCapabilityCache, ProviderFamily, RequestShape, SyncTextTokenizer,
    TokenAccountingObservation, TokenBreakdown, TokenCalibrationStore, TokenCount,
    TokenCountConfidence, TokenCountRequest, TokenCountSource, TokenCountUnknown,
    TokenizedPartCache, TokenizerResolver, UsageCoverage, TOKENIZER_REGISTRY_VERSION,
};

#[derive(Debug)]
pub struct TokenAccountingService {
    resolver: TokenizerResolver,
    calibrations: TokenCalibrationStore,
    capabilities: ProviderCountCapabilityCache,
    parts: TokenizedPartCache,
    preload_started: AtomicBool,
}

impl Default for TokenAccountingService {
    fn default() -> Self {
        Self {
            resolver: TokenizerResolver,
            calibrations: TokenCalibrationStore::default(),
            capabilities: ProviderCountCapabilityCache::default(),
            parts: TokenizedPartCache::default(),
            preload_started: AtomicBool::new(false),
        }
    }
}

impl TokenAccountingService {
    pub fn count_local(&self, request: &TokenCountRequest<'_>) -> TokenCount {
        let resolved = self.resolver.resolve(request.provider, request.model);
        let tokenizer_failed = Cell::new(false);
        let mut unknowns = Vec::new();
        let count_part = |text: &str| -> u64 {
            let Some(resolved) = resolved.as_ref() else {
                return heuristic::count_text(text);
            };
            let tokenizer_id = resolved.tokenizer.id();
            if let Some(tokens) = self.parts.get(
                tokenizer_id,
                resolved.registry_version,
                PartCacheKind::Text,
                text.as_bytes(),
            ) {
                return tokens;
            }
            match resolved.tokenizer.count_text(text) {
                Ok(tokens) => {
                    self.parts.put(
                        tokenizer_id,
                        resolved.registry_version,
                        PartCacheKind::Text,
                        text.as_bytes(),
                        tokens,
                    );
                    tokens
                }
                Err(_) => {
                    tokenizer_failed.set(true);
                    heuristic::count_text(text)
                }
            }
        };
        let count_json = |value: &Value, unknowns: &mut Vec<TokenCountUnknown>| -> u64 {
            if heuristic::contains_media(value) {
                // Never feed inline Base64 or document payload bytes into a
                // text tokenizer. Modality rules provide a bounded estimate.
                heuristic::count_json(value, unknowns)
            } else {
                let Some(resolved) = resolved.as_ref() else {
                    return heuristic::count_json(value, unknowns);
                };
                let Ok(serialized) = serde_json::to_vec(value) else {
                    return heuristic::count_json(value, unknowns);
                };
                let tokenizer_id = resolved.tokenizer.id();
                if let Some(tokens) = self.parts.get(
                    tokenizer_id,
                    resolved.registry_version,
                    PartCacheKind::Json,
                    &serialized,
                ) {
                    return tokens;
                }
                match resolved
                    .tokenizer
                    .count_text(std::str::from_utf8(&serialized).unwrap_or_default())
                {
                    Ok(tokens) => {
                        self.parts.put(
                            tokenizer_id,
                            resolved.registry_version,
                            PartCacheKind::Json,
                            &serialized,
                            tokens,
                        );
                        tokens
                    }
                    Err(_) => {
                        tokenizer_failed.set(true);
                        heuristic::count_json(value, unknowns)
                    }
                }
            }
        };

        let stable_prompt = count_part(request.stable_prompt);
        let dynamic_prompt = count_part(request.dynamic_prompt);
        let history_with_media: u64 = request
            .history
            .iter()
            .map(|value| count_json(value, &mut unknowns))
            .sum();
        let eager_tool_schemas = request
            .eager_tool_schemas
            .iter()
            .map(|value| count_json(value, &mut unknowns))
            .sum();
        let activated_tool_schemas = request
            .activated_tool_schemas
            .iter()
            .map(|value| count_json(value, &mut unknowns))
            .sum();
        let protocol_overhead = request_protocol_overhead(request);
        let media = media_tokens(&unknowns);
        let history = history_with_media.saturating_sub(media);
        let breakdown = TokenBreakdown {
            stable_prompt,
            dynamic_prompt,
            history,
            eager_tool_schemas,
            activated_tool_schemas,
            media,
            protocol_overhead,
        };
        let estimated = breakdown.total();
        let (source, confidence, tokenizer_id, registry_version, lower, upper) =
            if let Some(resolved) = resolved.filter(|_| !tokenizer_failed.get()) {
                (
                    TokenCountSource::LocalTokenizer,
                    if unknowns.is_empty() {
                        TokenCountConfidence::High
                    } else {
                        TokenCountConfidence::Medium
                    },
                    Some(resolved.tokenizer.id()),
                    resolved.registry_version,
                    estimated.saturating_mul(95) / 100,
                    estimated.saturating_add(estimated.div_ceil(if unknowns.is_empty() {
                        10
                    } else {
                        5
                    })),
                )
            } else {
                if !unknowns.contains(&TokenCountUnknown::TokenizerUnavailable) {
                    unknowns.push(TokenCountUnknown::TokenizerUnavailable);
                }
                (
                    TokenCountSource::Heuristic,
                    TokenCountConfidence::Low,
                    None,
                    TOKENIZER_REGISTRY_VERSION,
                    estimated.saturating_mul(75) / 100,
                    estimated.saturating_add(estimated.div_ceil(2)),
                )
            };
        let count = TokenCount::new(
            estimated,
            lower,
            upper,
            source,
            confidence,
            tokenizer_id,
            registry_version,
            request.request_shape,
            breakdown,
            unknowns,
        );
        self.calibrations
            .apply(&self.calibration_key(request, &count), count)
    }

    pub fn count_text(&self, provider: ProviderFamily, model: &str, text: &str) -> TokenCount {
        self.count_local(&TokenCountRequest::text(provider, model, text))
    }

    /// Capture the local complete-request accounting state needed for a later
    /// Tier-4 capacity proof. The proof contains no request content.
    pub fn preflight_capacity_proof(
        &self,
        request: &TokenCountRequest<'_>,
        count: &TokenCount,
        max_input_tokens: u64,
    ) -> Option<PreflightCapacityProof> {
        // A local proof is only valid when every unknown is the tokenizer
        // availability marker. Media/unsupported blocks can change Provider
        // accounting non-linearly when history is rewritten.
        if count
            .unknowns
            .iter()
            .any(|unknown| !matches!(unknown, TokenCountUnknown::TokenizerUnavailable))
        {
            return None;
        }
        let original_raw_tokens = count.breakdown.total();
        let original_history_raw_tokens =
            count
                .breakdown
                .history
                .saturating_add(history_protocol_overhead(
                    request.request_shape,
                    request.history.len(),
                ));
        let fixed_raw_tokens = original_raw_tokens
            .checked_sub(count.breakdown.history)?
            // Leave tool-schema protocol overhead in the frozen fixed lane.
            // A compact-history recount intentionally has no tool schemas, so
            // it contributes only the base + per-message protocol cost.
            .checked_sub(history_protocol_overhead(
                request.request_shape,
                request.history.len(),
            ))?;
        let fixed_non_history_upper_bound =
            scaled_upper_bound(fixed_raw_tokens, count.upper_bound, original_raw_tokens)?;
        let original_history_upper_bound = scaled_upper_bound(
            original_history_raw_tokens,
            count.upper_bound,
            original_raw_tokens,
        )?;
        Some(PreflightCapacityProof {
            provider: request.provider,
            model: request.model.to_string(),
            request_shape: request.request_shape,
            tokenizer_id: count.tokenizer_id,
            tokenizer_registry_version: count.tokenizer_registry_version,
            original_history_fingerprint: history_fingerprint(request.history).ok()?,
            fixed_non_history_upper_bound,
            original_history_upper_bound,
            original_raw_tokens,
            original_local_upper_bound: count.upper_bound,
            max_input_tokens,
        })
    }

    /// Verify an immutable local-preflight certificate against the exact
    /// original history and prove the complete compacted request upper bound.
    /// Dynamic prompt and tool-schema bodies are intentionally not rebuilt:
    /// their frozen conservative cost is carried by `proof.fixed_raw_tokens`.
    pub fn verify_compacted_capacity(
        &self,
        proof: &PreflightCapacityProof,
        original_history: &[Value],
        compacted_history: &[Value],
    ) -> Result<u64, CapacityProofError> {
        if proof.max_input_tokens == 0
            || proof.original_local_upper_bound <= proof.max_input_tokens
            || proof.original_raw_tokens == 0
            || proof
                .fixed_non_history_upper_bound
                .saturating_add(proof.original_history_upper_bound)
                < proof.original_local_upper_bound
        {
            return Err(CapacityProofError::InvalidCertificate);
        }
        let original_fingerprint = history_fingerprint(original_history)
            .map_err(|_| CapacityProofError::OriginalHistoryMismatch)?;
        if original_fingerprint != proof.original_history_fingerprint {
            return Err(CapacityProofError::OriginalHistoryMismatch);
        }

        let compact_request = TokenCountRequest {
            provider: proof.provider,
            model: &proof.model,
            request_shape: proof.request_shape,
            stable_prompt: "",
            dynamic_prompt: "",
            history: compacted_history,
            eager_tool_schemas: &[],
            activated_tool_schemas: &[],
        };
        let compact_count = self.count_local(&compact_request);
        if compact_count.tokenizer_id != proof.tokenizer_id
            || compact_count.tokenizer_registry_version != proof.tokenizer_registry_version
        {
            return Err(CapacityProofError::TokenizerDrift);
        }
        if compact_count
            .unknowns
            .iter()
            .any(|unknown| !matches!(unknown, TokenCountUnknown::TokenizerUnavailable))
        {
            return Err(CapacityProofError::UnsupportedUnknownContent);
        }

        // Keep every non-history lane at its frozen conservative upper cost.
        // Only the compacted history + its per-message protocol overhead is
        // freshly counted, then lifted by the exact conservative multiplier
        // encoded in the original complete local count.
        let compact_history_raw = compact_count
            .breakdown
            .history
            .saturating_add(compact_count.breakdown.protocol_overhead);
        let compact_history_upper = scaled_upper_bound(
            compact_history_raw,
            proof.original_local_upper_bound,
            proof.original_raw_tokens,
        )
        .ok_or(CapacityProofError::InvalidCertificate)?;
        let projected_input_upper = proof
            .fixed_non_history_upper_bound
            .saturating_add(compact_history_upper);
        if projected_input_upper > proof.max_input_tokens {
            return Err(CapacityProofError::DoesNotFit {
                projected_input_upper,
                max_input_tokens: proof.max_input_tokens,
            });
        }
        Ok(projected_input_upper)
    }

    pub fn observe(&self, request: &TokenCountRequest<'_>, predicted: &TokenCount, actual: u64) {
        if actual == 0 || predicted.estimated == 0 {
            return;
        }
        self.calibrations.observe(
            self.calibration_key(request, predicted),
            // Breakdown intentionally remains the uncalibrated local count.
            // Training against an already-calibrated estimate would make the
            // EMA decay back toward 1.0 instead of learning actual/raw.
            predicted.breakdown.total(),
            actual,
        );
    }

    pub fn should_refine(&self, local: &TokenCount, thresholds: &[u64]) -> bool {
        local
            .unknowns
            .iter()
            .any(|unknown| !matches!(unknown, TokenCountUnknown::TokenizerUnavailable))
            || thresholds
                .iter()
                .any(|threshold| local.lower_bound <= *threshold && *threshold <= local.upper_bound)
    }

    pub fn provider_count_should_attempt(&self, capability_key: &str) -> bool {
        self.capabilities.should_attempt(capability_key)
    }

    pub fn begin_provider_count(
        &self,
        capability_key: &str,
    ) -> Option<super::ProviderCountAttempt<'_>> {
        self.capabilities.begin_attempt(capability_key)
    }

    pub fn provider_count_profile_allowed(&self, profile_key: &str) -> bool {
        self.capabilities.profile_allowed(profile_key)
    }

    pub fn suppress_provider_count_profile(
        &self,
        profile_key: String,
        duration: std::time::Duration,
    ) {
        self.capabilities.suppress_profile(profile_key, duration);
    }

    pub fn record_provider_count_supported(&self, capability_key: String) {
        self.capabilities.record_supported(capability_key);
    }

    pub fn record_provider_count_unsupported(&self, capability_key: String) {
        self.capabilities.record_unsupported(capability_key);
    }

    pub async fn preload_recent_calibrations(&self, db: Arc<crate::session::SessionDB>) {
        if self
            .preload_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let loaded = db
            .run(|db| db.load_recent_token_accounting_observations(256))
            .await;
        match loaded {
            Ok(observations) => {
                for observation in observations {
                    self.observe_persisted(observation);
                }
            }
            Err(error) => {
                self.preload_started.store(false, Ordering::Release);
                crate::app_debug!(
                    "agent",
                    "token_accounting",
                    "token calibration preload unavailable: {}",
                    error
                );
            }
        }
    }

    fn observe_persisted(&self, observation: TokenAccountingObservation) {
        if observation.input_coverage != UsageCoverage::Complete {
            return;
        }
        let Some(actual) = observation.actual_input_tokens else {
            return;
        };
        self.calibrations.observe(
            CalibrationKey {
                provider: observation.provider,
                model: observation.model,
                request_shape: observation.request_shape,
                tokenizer_id: observation.tokenizer_id,
                tokenizer_registry_version: observation.tokenizer_registry_version,
                has_media: observation.has_media,
            },
            observation.raw_estimated,
            actual,
        );
    }

    fn calibration_key(
        &self,
        request: &TokenCountRequest<'_>,
        count: &TokenCount,
    ) -> CalibrationKey {
        CalibrationKey {
            provider: request.provider,
            model: request.model.to_string(),
            request_shape: request.request_shape,
            tokenizer_id: count.tokenizer_id,
            tokenizer_registry_version: count.tokenizer_registry_version,
            has_media: count
                .unknowns
                .iter()
                .any(|unknown| !matches!(unknown, TokenCountUnknown::TokenizerUnavailable)),
        }
    }
}

fn scaled_upper_bound(raw: u64, complete_upper: u64, complete_raw: u64) -> Option<u64> {
    (complete_raw > 0).then(|| {
        let scaled =
            (u128::from(raw) * u128::from(complete_upper)).div_ceil(u128::from(complete_raw));
        scaled.min(u128::from(u64::MAX)) as u64
    })
}

fn history_fingerprint(history: &[Value]) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(history).context("serialize history for capacity proof")?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

pub fn service() -> &'static TokenAccountingService {
    static SERVICE: OnceLock<TokenAccountingService> = OnceLock::new();
    SERVICE.get_or_init(TokenAccountingService::default)
}

/// Small synchronous model/profile snapshot injected into context compaction.
/// It owns no credentials and performs no IO.
#[derive(Debug, Clone)]
pub struct CompactionTokenCounter<'a> {
    provider: ProviderFamily,
    model: &'a str,
    request_shape: RequestShape,
    tool_schemas: &'a [Value],
}

impl<'a> CompactionTokenCounter<'a> {
    pub const fn new(
        provider: ProviderFamily,
        model: &'a str,
        request_shape: RequestShape,
        tool_schemas: &'a [Value],
    ) -> Self {
        Self {
            provider,
            model,
            request_shape,
            tool_schemas,
        }
    }

    pub fn count_request_upper(
        &self,
        system_prompt: &str,
        messages: &[Value],
        max_output_tokens: u32,
    ) -> u32 {
        self.count_request_with_tools_upper(
            system_prompt,
            messages,
            self.tool_schemas,
            max_output_tokens,
        )
    }

    pub fn count_request_with_tools_upper(
        &self,
        system_prompt: &str,
        messages: &[Value],
        tool_schemas: &[Value],
        max_output_tokens: u32,
    ) -> u32 {
        let request = TokenCountRequest {
            provider: self.provider,
            model: self.model,
            request_shape: self.request_shape,
            stable_prompt: system_prompt,
            dynamic_prompt: "",
            history: messages,
            eager_tool_schemas: tool_schemas,
            activated_tool_schemas: &[],
        };
        service()
            .count_local(&request)
            .upper_bound
            .saturating_add(u64::from(max_output_tokens))
            .min(u64::from(u32::MAX)) as u32
    }
}

fn request_protocol_overhead(request: &TokenCountRequest<'_>) -> u64 {
    let tool_count = request
        .eager_tool_schemas
        .len()
        .saturating_add(request.activated_tool_schemas.len()) as u64;
    history_protocol_overhead(request.request_shape, request.history.len()).saturating_add(
        match request.request_shape {
            RequestShape::AnthropicMessages | RequestShape::OpenAiChat => tool_count * 12,
            RequestShape::OpenAiResponses | RequestShape::CodexResponses => tool_count * 10,
            RequestShape::Text | RequestShape::Json => 0,
        },
    )
}

fn history_protocol_overhead(request_shape: RequestShape, message_count: usize) -> u64 {
    let message_count = message_count as u64;
    match request_shape {
        RequestShape::AnthropicMessages => 8 + message_count.saturating_mul(4),
        RequestShape::OpenAiChat => 3 + message_count.saturating_mul(4),
        RequestShape::OpenAiResponses | RequestShape::CodexResponses => {
            5 + message_count.saturating_mul(3)
        }
        RequestShape::Text => 0,
        RequestShape::Json => 1,
    }
}

fn media_tokens(unknowns: &[TokenCountUnknown]) -> u64 {
    unknowns
        .iter()
        .map(|unknown| match unknown {
            TokenCountUnknown::Image => 2_000,
            TokenCountUnknown::Document | TokenCountUnknown::Audio => 8_000,
            TokenCountUnknown::UnsupportedContentBlock(_)
            | TokenCountUnknown::TokenizerUnavailable => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_openai_model_uses_local_tokenizer_with_bounds() {
        let service = TokenAccountingService::default();
        let count = service.count_text(ProviderFamily::OpenAiResponses, "gpt-5", "hello 世界");
        assert_eq!(count.source, TokenCountSource::LocalTokenizer);
        assert!(count.lower_bound <= count.estimated);
        assert!(count.estimated <= count.upper_bound);
        assert!(count.estimated > 0);
    }

    #[test]
    fn compaction_counter_keeps_bound_tool_schemas() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "name": "large_tool",
            "description": "x".repeat(20_000),
        })];
        let counter = CompactionTokenCounter::new(
            ProviderFamily::OpenAiResponses,
            "gpt-5",
            RequestShape::OpenAiResponses,
            &tools,
        );
        let messages = vec![serde_json::json!({ "role": "user", "content": "hello" })];

        let with_tools = counter.count_request_upper("system", &messages, 0);
        let without_tools = counter.count_request_with_tools_upper("system", &messages, &[], 0);
        assert!(with_tools > without_tools);
    }

    #[test]
    fn tier4_capacity_proof_covers_frozen_non_history_lanes() {
        let service = TokenAccountingService::default();
        let original_history = vec![serde_json::json!({
            "role": "user",
            "content": "overflow ".repeat(60_000),
        })];
        let compacted_history = vec![serde_json::json!({
            "role": "user",
            "content": "overflow summary",
        })];
        let tools = vec![serde_json::json!({
            "type": "function",
            "name": "large_tool",
            "description": "schema ".repeat(5_000),
        })];
        let request = TokenCountRequest {
            provider: ProviderFamily::OpenAiResponses,
            model: "gpt-5",
            request_shape: RequestShape::OpenAiResponses,
            stable_prompt: &"system ".repeat(5_000),
            dynamic_prompt: "dynamic lane",
            history: &original_history,
            eager_tool_schemas: &tools,
            activated_tool_schemas: &[],
        };
        let count = service.count_local(&request);
        let max_input_tokens = count.upper_bound.saturating_sub(10_000);
        assert!(count.upper_bound > max_input_tokens);
        let proof = service
            .preflight_capacity_proof(&request, &count, max_input_tokens)
            .expect("plain text request should produce a proof");

        let projected = service
            .verify_compacted_capacity(&proof, &original_history, &compacted_history)
            .expect("large history reduction should prove full request capacity");

        assert!(projected <= max_input_tokens);
        assert!(
            projected > 1_000,
            "fixed prompt/tool lanes must remain counted"
        );
    }

    #[test]
    fn tier4_capacity_proof_rejects_wrong_source_history_and_insufficient_reduction() {
        let service = TokenAccountingService::default();
        let original_history = vec![serde_json::json!({
            "role": "user",
            "content": "overflow ".repeat(20_000),
        })];
        let request = TokenCountRequest {
            provider: ProviderFamily::OpenAiResponses,
            model: "gpt-5",
            request_shape: RequestShape::OpenAiResponses,
            stable_prompt: "system",
            dynamic_prompt: "",
            history: &original_history,
            eager_tool_schemas: &[],
            activated_tool_schemas: &[],
        };
        let count = service.count_local(&request);
        let max_input_tokens = count.upper_bound.saturating_sub(1);
        let proof = service
            .preflight_capacity_proof(&request, &count, max_input_tokens)
            .expect("plain text request should produce a proof");

        let wrong_history = vec![serde_json::json!({
            "role": "user",
            "content": "different",
        })];
        assert_eq!(
            service.verify_compacted_capacity(&proof, &wrong_history, &[]),
            Err(CapacityProofError::OriginalHistoryMismatch)
        );
        assert!(matches!(
            service.verify_compacted_capacity(&proof, &original_history, &original_history),
            Err(CapacityProofError::DoesNotFit { .. })
        ));
    }

    #[test]
    fn tier4_capacity_proof_refuses_media_unknowns() {
        let service = TokenAccountingService::default();
        let history = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "input_image",
                "image_url": "data:image/png;base64,AAAA",
            }],
        })];
        let request = TokenCountRequest {
            provider: ProviderFamily::OpenAiResponses,
            model: "gpt-5",
            request_shape: RequestShape::OpenAiResponses,
            stable_prompt: "",
            dynamic_prompt: "",
            history: &history,
            eager_tool_schemas: &[],
            activated_tool_schemas: &[],
        };
        let count = service.count_local(&request);
        assert!(service
            .preflight_capacity_proof(&request, &count, 1)
            .is_none());
    }

    #[test]
    fn unknown_model_is_honest_heuristic() {
        let service = TokenAccountingService::default();
        let count = service.count_text(ProviderFamily::Unknown, "mystery", "你好");
        assert_eq!(count.source, TokenCountSource::Heuristic);
        assert_eq!(count.confidence, TokenCountConfidence::Low);
        assert!(count
            .unknowns
            .contains(&TokenCountUnknown::TokenizerUnavailable));
    }
}
