use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use super::heuristic;
use super::{
    CalibrationKey, PartCacheKind, ProviderCountCapabilityCache, ProviderFamily, RequestShape,
    SyncTextTokenizer, TokenAccountingObservation, TokenBreakdown, TokenCalibrationStore,
    TokenCount, TokenCountConfidence, TokenCountRequest, TokenCountSource, TokenCountUnknown,
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
    let message_count = request.history.len() as u64;
    let tool_count = request
        .eager_tool_schemas
        .len()
        .saturating_add(request.activated_tool_schemas.len()) as u64;
    match request.request_shape {
        RequestShape::AnthropicMessages => 8 + message_count.saturating_mul(4) + tool_count * 12,
        RequestShape::OpenAiChat => 3 + message_count.saturating_mul(4) + tool_count * 12,
        RequestShape::OpenAiResponses | RequestShape::CodexResponses => {
            5 + message_count.saturating_mul(3) + tool_count * 10
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
