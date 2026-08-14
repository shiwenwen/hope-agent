use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFamily {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    Codex,
    Unknown,
}

impl ProviderFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponses => "openai_responses",
            Self::Codex => "codex",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestShape {
    AnthropicMessages,
    OpenAiChat,
    OpenAiResponses,
    CodexResponses,
    Text,
    Json,
}

impl RequestShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponses => "openai_responses",
            Self::CodexResponses => "codex_responses",
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerId {
    O200kBase,
    Cl100kBase,
}

impl TokenizerId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::O200kBase => "o200k_base",
            Self::Cl100kBase => "cl100k_base",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountSource {
    ProviderPreflight,
    LocalTokenizer,
    CalibratedTokenizer,
    CalibratedHeuristic,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountUnknown {
    Image,
    Document,
    Audio,
    UnsupportedContentBlock(String),
    TokenizerUnavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBreakdown {
    pub stable_prompt: u64,
    pub dynamic_prompt: u64,
    pub history: u64,
    pub eager_tool_schemas: u64,
    pub activated_tool_schemas: u64,
    pub media: u64,
    pub protocol_overhead: u64,
}

impl TokenBreakdown {
    pub fn total(&self) -> u64 {
        self.stable_prompt
            .saturating_add(self.dynamic_prompt)
            .saturating_add(self.history)
            .saturating_add(self.eager_tool_schemas)
            .saturating_add(self.activated_tool_schemas)
            .saturating_add(self.media)
            .saturating_add(self.protocol_overhead)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCount {
    pub lower_bound: u64,
    pub estimated: u64,
    pub upper_bound: u64,
    pub source: TokenCountSource,
    pub confidence: TokenCountConfidence,
    pub tokenizer_id: Option<TokenizerId>,
    pub tokenizer_registry_version: u32,
    pub request_shape: RequestShape,
    pub breakdown: TokenBreakdown,
    pub unknowns: Vec<TokenCountUnknown>,
}

/// Content-free observation persisted inside the final chat usage row. A turn
/// may contain several of these (one per Provider sampling round) while still
/// producing exactly one billing/usage ledger row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountingObservation {
    pub operation_key: String,
    pub provider: ProviderFamily,
    pub model: String,
    pub request_shape: RequestShape,
    pub tokenizer_id: Option<TokenizerId>,
    pub tokenizer_registry_version: u32,
    pub source: TokenCountSource,
    pub raw_estimated: u64,
    pub lower_bound: u64,
    pub estimated: u64,
    pub upper_bound: u64,
    pub actual_input_tokens: Option<u64>,
    pub input_coverage: UsageCoverage,
    #[serde(default)]
    pub output_coverage: UsageCoverage,
    /// Conservative output reservation used only when this round's Provider
    /// response omitted authoritative output usage.
    #[serde(default)]
    pub reserved_output_tokens: u64,
    pub has_media: bool,
}

impl TokenCount {
    pub fn new(
        estimated: u64,
        lower_bound: u64,
        upper_bound: u64,
        source: TokenCountSource,
        confidence: TokenCountConfidence,
        tokenizer_id: Option<TokenizerId>,
        tokenizer_registry_version: u32,
        request_shape: RequestShape,
        breakdown: TokenBreakdown,
        unknowns: Vec<TokenCountUnknown>,
    ) -> Self {
        let lower_bound = lower_bound.min(estimated);
        let upper_bound = upper_bound.max(estimated);
        Self {
            lower_bound,
            estimated,
            upper_bound,
            source,
            confidence,
            tokenizer_id,
            tokenizer_registry_version,
            request_shape,
            breakdown,
            unknowns,
        }
    }

    pub fn with_provider_total(mut self, total: u64) -> Self {
        self.lower_bound = total.saturating_mul(98) / 100;
        self.estimated = total;
        self.upper_bound = total.saturating_add(total.div_ceil(50)).max(total);
        self.source = TokenCountSource::ProviderPreflight;
        self.confidence = TokenCountConfidence::High;
        self
    }

    pub fn add_reserved_output(mut self, tokens: u64) -> Self {
        self.lower_bound = self.lower_bound.saturating_add(tokens);
        self.estimated = self.estimated.saturating_add(tokens);
        self.upper_bound = self.upper_bound.saturating_add(tokens);
        self
    }
}

pub struct TokenCountRequest<'a> {
    pub provider: ProviderFamily,
    pub model: &'a str,
    pub request_shape: RequestShape,
    pub stable_prompt: &'a str,
    pub dynamic_prompt: &'a str,
    pub history: &'a [Value],
    pub eager_tool_schemas: &'a [Value],
    pub activated_tool_schemas: &'a [Value],
}

impl<'a> TokenCountRequest<'a> {
    pub fn text(provider: ProviderFamily, model: &'a str, text: &'a str) -> Self {
        Self {
            provider,
            model,
            request_shape: RequestShape::Text,
            stable_prompt: text,
            dynamic_prompt: "",
            history: &[],
            eager_tool_schemas: &[],
            activated_tool_schemas: &[],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageCoverage {
    Complete,
    Partial,
    #[default]
    Missing,
}

impl UsageCoverage {
    pub fn accumulate(self, next: Self) -> Self {
        match (self, next) {
            (Self::Missing, Self::Missing) => Self::Missing,
            (Self::Complete, Self::Complete) => Self::Complete,
            _ => Self::Partial,
        }
    }

    pub const fn is_present(self) -> bool {
        !matches!(self, Self::Missing)
    }
}

/// Immutable evidence captured from the exact local preflight request that
/// overflowed. Tier 4 may use this certificate to prove that replacing only
/// the history lane with a compacted projection brings the complete request
/// back under the same input ceiling without rebuilding dynamic prompt or
/// tool-schema lanes.
///
/// The certificate deliberately carries counts and a content fingerprint, not
/// prompt/tool text. A fingerprint mismatch, tokenizer drift, media, or an
/// insufficient reduction invalidates the proof and must fail closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreflightCapacityProof {
    pub provider: ProviderFamily,
    pub model: String,
    pub request_shape: RequestShape,
    pub tokenizer_id: Option<TokenizerId>,
    pub tokenizer_registry_version: u32,
    pub original_history_fingerprint: String,
    pub fixed_non_history_upper_bound: u64,
    pub original_history_upper_bound: u64,
    pub original_raw_tokens: u64,
    pub original_local_upper_bound: u64,
    pub max_input_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapacityProofError {
    OriginalHistoryMismatch,
    TokenizerDrift,
    UnsupportedUnknownContent,
    InvalidCertificate,
    DoesNotFit {
        projected_input_upper: u64,
        max_input_tokens: u64,
    },
}

impl fmt::Display for CapacityProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OriginalHistoryMismatch => {
                f.write_str("preflight capacity proof history fingerprint mismatch")
            }
            Self::TokenizerDrift => {
                f.write_str("preflight capacity proof tokenizer identity changed")
            }
            Self::UnsupportedUnknownContent => f.write_str(
                "preflight capacity proof cannot conservatively account for media or unknown content",
            ),
            Self::InvalidCertificate => {
                f.write_str("preflight capacity proof certificate is internally inconsistent")
            }
            Self::DoesNotFit {
                projected_input_upper,
                max_input_tokens,
            } => write!(
                f,
                "compacted request upper bound {projected_input_upper} still exceeds max input {max_input_tokens}"
            ),
        }
    }
}

impl std::error::Error for CapacityProofError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightOverflow {
    pub input_tokens: u64,
    pub max_input_tokens: u64,
    pub source: TokenCountSource,
    /// Present only when the *local complete request* count itself overflowed.
    /// A provider-only count cannot prove how much a local history rewrite
    /// removes and therefore cannot authorize Tier 4 by itself.
    pub capacity_proof: Option<PreflightCapacityProof>,
}

impl fmt::Display for PreflightOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "maximum context length would be exceeded before provider request: input upper bound {} > max input {} (source={:?})",
            self.input_tokens, self.max_input_tokens, self.source
        )
    }
}

impl std::error::Error for PreflightOverflow {}
