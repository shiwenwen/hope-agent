//! Hybrid note-search ranking configuration — the tunable half of
//! `knowledge::search`; the FTS5 + vector + RRF/MMR pipeline stays in ha-core.

use serde::{Deserialize, Serialize};

// Defaults (mirror the memory backend's fusion constants for parity). Exposed as
// tunables via `KnowledgeSearchConfig`; these are the reset-to values.
// `DEFAULT_TEXT_WEIGHT` / `DEFAULT_VECTOR_WEIGHT` 为 pub：ha-core 侧测试引用
// （可见性升级，原为模块私有）。
pub const DEFAULT_TEXT_WEIGHT: f64 = 0.4;
pub const DEFAULT_VECTOR_WEIGHT: f64 = 0.6;
const DEFAULT_RRF_K: f64 = 60.0;
const DEFAULT_MMR_LAMBDA: f32 = 0.7;
const DEFAULT_CANDIDATE_MULTIPLIER: usize = 3;

fn default_text_weight() -> f64 {
    DEFAULT_TEXT_WEIGHT
}
fn default_vector_weight() -> f64 {
    DEFAULT_VECTOR_WEIGHT
}
fn default_rrf_k() -> f64 {
    DEFAULT_RRF_K
}
fn default_mmr_lambda() -> f32 {
    DEFAULT_MMR_LAMBDA
}
fn default_candidate_multiplier() -> usize {
    DEFAULT_CANDIDATE_MULTIPLIER
}

/// User-tunable ranking parameters for the hybrid `note_search` pipeline
/// (`AppConfig.knowledge_search`). Pure query-time — no reindex side effect — so
/// unlike `knowledge_chunk` / `knowledge_embedding` it is a normal MEDIUM setting
/// (GUI + `ha-settings`). Only affects `search_notes`; `note_similar` is
/// vector-only and `note_related` uses its own fusion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchConfig {
    /// Weight of the keyword (FTS5/BM25) arm in rank fusion. Relative to
    /// `vector_weight` — only the ratio matters.
    #[serde(default = "default_text_weight")]
    pub text_weight: f64,
    /// Weight of the semantic (vector) arm in rank fusion.
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// RRF smoothing constant: larger flattens the influence of top ranks
    /// (gentler fusion); smaller sharpens it toward each arm's #1.
    #[serde(default = "default_rrf_k")]
    pub rrf_k: f64,
    /// MMR relevance↔diversity tradeoff: 1.0 = pure relevance, 0.0 = pure
    /// diversity (de-duplicates near-identical notes harder).
    #[serde(default = "default_mmr_lambda")]
    pub mmr_lambda: f32,
    /// Candidate pool before MMR = requested `limit` × this multiplier.
    #[serde(default = "default_candidate_multiplier")]
    pub candidate_multiplier: usize,
}

impl Default for KnowledgeSearchConfig {
    fn default() -> Self {
        Self {
            text_weight: DEFAULT_TEXT_WEIGHT,
            vector_weight: DEFAULT_VECTOR_WEIGHT,
            rrf_k: DEFAULT_RRF_K,
            mmr_lambda: DEFAULT_MMR_LAMBDA,
            candidate_multiplier: DEFAULT_CANDIDATE_MULTIPLIER,
        }
    }
}

impl KnowledgeSearchConfig {
    /// Clamp to sane bounds. Weights to `[0, 1]`; if both end up ~0 (a footgun
    /// that would flatten all scores), reset to defaults. `rrf_k` to `[1, 1000]`,
    /// `mmr_lambda` to `[0, 1]`, `candidate_multiplier` to `[1, 10]`.
    pub fn clamped(&self) -> KnowledgeSearchConfig {
        let mut text_weight = self.text_weight.clamp(0.0, 1.0);
        let mut vector_weight = self.vector_weight.clamp(0.0, 1.0);
        if text_weight + vector_weight < f64::EPSILON {
            text_weight = DEFAULT_TEXT_WEIGHT;
            vector_weight = DEFAULT_VECTOR_WEIGHT;
        }
        KnowledgeSearchConfig {
            text_weight,
            vector_weight,
            rrf_k: self.rrf_k.clamp(1.0, 1000.0),
            mmr_lambda: self.mmr_lambda.clamp(0.0, 1.0),
            candidate_multiplier: self.candidate_multiplier.clamp(1, 10),
        }
    }
}
