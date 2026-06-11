//! Hybrid note search (design D12 / Layer 3): chunk-level FTS5 + vector KNN →
//! weighted RRF → MMR diversity → aggregate best chunk back to its note.
//!
//! Independent store with its own ranking — the RRF/MMR *algorithms* are reused
//! from the memory backend, but notes never mix into `recall_memory` (D7).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::db::IndexDb;
use super::types::NoteSearchHit;
use crate::util::truncate_utf8;

// Defaults (mirror the memory backend's fusion constants for parity). Exposed as
// tunables via `KnowledgeSearchConfig`; these are the reset-to values.
const DEFAULT_TEXT_WEIGHT: f64 = 0.4;
const DEFAULT_VECTOR_WEIGHT: f64 = 0.6;
const DEFAULT_RRF_K: f64 = 60.0;
const DEFAULT_MMR_LAMBDA: f32 = 0.7;
const DEFAULT_CANDIDATE_MULTIPLIER: usize = 3;
const SNIPPET_BYTES: usize = 320;

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

/// Search `kb_ids` for `query`, returning up to `limit` note hits ordered by
/// relevance (MMR-diversified), each carrying its best-matching chunk snippet.
pub fn search_notes(
    db: &IndexDb,
    kb_ids: &[String],
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<NoteSearchHit>> {
    if kb_ids.is_empty() || query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let cfg = crate::config::cached_config().knowledge_search.clamped();
    let fetch = (limit * cfg.candidate_multiplier).max(10);

    // Step 1: FTS5 BM25 over chunks.
    let fts = db.fts_search(kb_ids, query, fetch)?;

    // Step 2: vector KNN over chunks (if an embedder + signature are active).
    // Knowledge has its own signature (D7) — independent of memory_embedding.
    let vec = match (
        db.embedder(),
        super::embedding::knowledge_active_embedding_signature(),
    ) {
        (Some(embedder), Some(signature)) => match embedder.embed(query) {
            Ok(q) => db
                .vec_search(kb_ids, &q, &signature, fetch)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };

    if fts.is_empty() && vec.is_empty() {
        return Ok(Vec::new());
    }

    // Step 3: weighted RRF over chunk ids (ordinal position only).
    let mut chunk_score: HashMap<i64, f64> = HashMap::new();
    let mut chunk_note: HashMap<i64, i64> = HashMap::new();
    for (rank, (chunk_id, note_id, _)) in fts.iter().enumerate() {
        *chunk_score.entry(*chunk_id).or_insert(0.0) +=
            cfg.text_weight / (cfg.rrf_k + rank as f64 + 1.0);
        chunk_note.insert(*chunk_id, *note_id);
    }
    for (rank, (chunk_id, note_id, _)) in vec.iter().enumerate() {
        *chunk_score.entry(*chunk_id).or_insert(0.0) +=
            cfg.vector_weight / (cfg.rrf_k + rank as f64 + 1.0);
        chunk_note.insert(*chunk_id, *note_id);
    }

    // Step 4: aggregate to note — keep the best (chunk_id, score) per note.
    let mut best_per_note: HashMap<i64, (i64, f64)> = HashMap::new();
    for (chunk_id, score) in &chunk_score {
        let Some(note_id) = chunk_note.get(chunk_id) else {
            continue;
        };
        best_per_note
            .entry(*note_id)
            .and_modify(|(bc, bs)| {
                if *score > *bs {
                    *bc = *chunk_id;
                    *bs = *score;
                }
            })
            .or_insert((*chunk_id, *score));
    }

    // Sort notes by best score desc, take a generous slice for MMR.
    let mut ranked: Vec<(i64, i64, f64)> = best_per_note
        .into_iter()
        .map(|(note_id, (chunk_id, score))| (note_id, chunk_id, score))
        .collect();
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(fetch);
    if ranked.is_empty() {
        return Ok(Vec::new());
    }

    // Load snippets + note metadata.
    let chunk_ids: Vec<i64> = ranked.iter().map(|(_, c, _)| *c).collect();
    let note_ids: Vec<i64> = ranked.iter().map(|(n, _, _)| *n).collect();
    let snippets = db.chunk_snippets(&chunk_ids)?;
    let notes = db.notes_for_ids(&note_ids)?;

    // Step 5: MMR diversity over the note candidates (by best-chunk text),
    // reusing the memory implementation.
    let candidates: Vec<(i64, f32, String)> = ranked
        .iter()
        .map(|(note_id, chunk_id, score)| {
            let body = snippets
                .get(chunk_id)
                .map(|(b, _, _)| b.clone())
                .unwrap_or_default();
            (*note_id, *score as f32, body)
        })
        .collect();
    let candidate_refs: Vec<(i64, f32, &str)> = candidates
        .iter()
        .map(|(id, s, body)| (*id, *s, body.as_str()))
        .collect();
    let reranked = crate::memory::mmr::mmr_rerank(&candidate_refs, limit, cfg.mmr_lambda);

    // Build hits in MMR order.
    let score_by_note: HashMap<i64, (i64, f64)> =
        ranked.iter().map(|(n, c, s)| (*n, (*c, *s))).collect();
    let mut hits = Vec::new();
    for (note_id, score) in reranked {
        let Some((chunk_id, _)) = score_by_note.get(&note_id) else {
            continue;
        };
        let Some((kb_id, rel_path, title)) = notes.get(&note_id) else {
            continue;
        };
        let (snippet, heading_path, start_line) = snippets
            .get(chunk_id)
            .map(|(b, h, l)| (truncate_utf8(b, SNIPPET_BYTES).to_string(), h.clone(), *l))
            .unwrap_or_default();
        hits.push(NoteSearchHit {
            kb_id: kb_id.clone(),
            kb_name: String::new(),
            kb_emoji: None,
            note_id,
            rel_path: rel_path.clone(),
            title: title.clone(),
            score,
            snippet,
            heading_path,
            start_line,
        });
    }
    enrich_kb_names(&mut hits);
    Ok(hits)
}

/// Fill `kb_name` / `kb_emoji` on each hit from the KB registry — the single
/// truth source for KB display data (index.db only stores `kb_id`, D9). Resolves
/// each distinct `kb_id` once; falls back to `kb_id` as the name when the KB is
/// missing so a hit is never dropped. No-op when the registry is unavailable
/// (kb_name stays empty → callers/UI fall back to kb_id).
pub fn enrich_kb_names(hits: &mut [NoteSearchHit]) {
    if hits.is_empty() {
        return;
    }
    let Some(reg) = crate::get_knowledge_db() else {
        return;
    };
    let mut cache: HashMap<String, (String, Option<String>)> = HashMap::new();
    for h in hits.iter_mut() {
        let (name, emoji) = cache
            .entry(h.kb_id.clone())
            .or_insert_with(|| match reg.get(&h.kb_id) {
                Ok(Some(kb)) => (kb.name, kb.emoji),
                _ => (h.kb_id.clone(), None),
            })
            .clone();
        h.kb_name = name;
        h.kb_emoji = emoji;
    }
}

/// Vector-only "similar notes" (WS4 `note_similar`): embed `source_text`, KNN over
/// chunks, aggregate to the best chunk per note, exclude `source_note_id`, return
/// up to `k` notes ordered by similarity. Returns empty when vector search is not
/// enabled (no embedder / no active signature) — the tool layer surfaces that.
/// **Errors** (rather than returning empty) when the embedding call itself fails,
/// so a transient outage isn't reported to the user as "no similar notes"; this is
/// the vector-only path with no FTS fallback. `note_related` tolerates the error
/// (degrades to link/tag recall); `note_similar` surfaces it.
pub fn similar_notes(
    db: &IndexDb,
    kb_ids: &[String],
    source_note_id: i64,
    source_text: &str,
    k: usize,
) -> anyhow::Result<Vec<NoteSearchHit>> {
    if kb_ids.is_empty() || k == 0 || source_text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let (Some(embedder), Some(signature)) = (
        db.embedder(),
        super::embedding::knowledge_active_embedding_signature(),
    ) else {
        return Ok(Vec::new());
    };
    let query = embedder
        .embed(source_text)
        .map_err(|e| anyhow::anyhow!("knowledge embedding failed: {e}"))?;
    // Over-fetch generously: the source note's own chunks (excluded below) sit at
    // the top of its own similarity ranking, so a multi-chunk source would starve
    // the k budget with a tighter window.
    let fetch = (k * 8).max(48);
    let hits = db.vec_search(kb_ids, &query, &signature, fetch)?;

    // Best (lowest distance) chunk per note, excluding the source note itself.
    let mut best: HashMap<i64, (i64, f64)> = HashMap::new();
    for (chunk_id, note_id, dist) in hits {
        if note_id == source_note_id {
            continue;
        }
        best.entry(note_id)
            .and_modify(|(bc, bd)| {
                if dist < *bd {
                    *bc = chunk_id;
                    *bd = dist;
                }
            })
            .or_insert((chunk_id, dist));
    }
    let mut ranked: Vec<(i64, i64, f64)> = best.into_iter().map(|(n, (c, d))| (n, c, d)).collect();
    // Distance asc, then note_id for a deterministic tiebreak (HashMap iteration
    // order is randomized, so equal-distance notes would otherwise swap per run).
    ranked.sort_by(|a, b| {
        a.2.partial_cmp(&b.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(k);
    if ranked.is_empty() {
        return Ok(Vec::new());
    }

    let chunk_ids: Vec<i64> = ranked.iter().map(|(_, c, _)| *c).collect();
    let note_ids: Vec<i64> = ranked.iter().map(|(n, _, _)| *n).collect();
    let snippets = db.chunk_snippets(&chunk_ids)?;
    let notes = db.notes_for_ids(&note_ids)?;

    let mut out = Vec::new();
    for (note_id, chunk_id, dist) in ranked {
        let Some((kb_id, rel_path, title)) = notes.get(&note_id) else {
            continue;
        };
        let (snippet, heading_path, start_line) = snippets
            .get(&chunk_id)
            .map(|(b, h, l)| (truncate_utf8(b, SNIPPET_BYTES).to_string(), h.clone(), *l))
            .unwrap_or_default();
        out.push(NoteSearchHit {
            kb_id: kb_id.clone(),
            kb_name: String::new(),
            kb_emoji: None,
            note_id,
            rel_path: rel_path.clone(),
            title: title.clone(),
            // Map cosine/L2 distance to a 0–1 similarity for display/ranking.
            score: (1.0 / (1.0 + dist.max(0.0))) as f32,
            snippet,
            heading_path,
            start_line,
        });
    }
    enrich_kb_names(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_config_clamps_and_recovers_zero_weights() {
        let c = KnowledgeSearchConfig {
            text_weight: 5.0,
            vector_weight: -1.0,
            rrf_k: 0.0,
            mmr_lambda: 9.0,
            candidate_multiplier: 999,
        }
        .clamped();
        assert_eq!(c.text_weight, 1.0); // clamped to [0,1]
        assert_eq!(c.vector_weight, 0.0);
        assert_eq!(c.rrf_k, 1.0); // clamped to [1,1000]
        assert_eq!(c.mmr_lambda, 1.0); // clamped to [0,1]
        assert_eq!(c.candidate_multiplier, 10);

        // Both weights zero → reset to defaults (avoid flattening all scores).
        let z = KnowledgeSearchConfig {
            text_weight: 0.0,
            vector_weight: 0.0,
            ..Default::default()
        }
        .clamped();
        assert_eq!(z.text_weight, DEFAULT_TEXT_WEIGHT);
        assert_eq!(z.vector_weight, DEFAULT_VECTOR_WEIGHT);
    }
}
