//! Unified token prediction and post-request accounting.
//!
//! Capacity decisions consume [`TokenCount::upper_bound`]. Provider usage is
//! authoritative after a request; local tokenizers and heuristics are only
//! predictions and retain their provenance.

mod calibration;
mod capability_cache;
mod heuristic;
mod part_cache;
mod resolver;
mod service;
mod tokenizer;
mod types;

pub use calibration::{CalibrationKey, TokenCalibrationStore};
pub use capability_cache::{
    profile_suppression_key, ProviderCountAttempt, ProviderCountCapabilityCache,
};
pub use part_cache::{PartCacheKind, TokenizedPartCache};
pub use resolver::{ResolvedTokenizer, TokenizerResolver, TOKENIZER_REGISTRY_VERSION};
pub use service::{service, CompactionTokenCounter, TokenAccountingService};
pub use tokenizer::{SyncTextTokenizer, TiktokenTokenizer, TokenizerError};
pub use types::{
    CapacityProofError, PreflightCapacityProof, PreflightOverflow, ProviderFamily, RequestShape,
    TokenAccountingObservation, TokenBreakdown, TokenCount, TokenCountConfidence,
    TokenCountRequest, TokenCountSource, TokenCountUnknown, TokenizerId, UsageCoverage,
};
