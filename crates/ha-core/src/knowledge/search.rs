//! Hybrid note search (design D12 / Layer 3): chunk-level FTS5 + vector KNN →
//! weighted RRF → MMR diversity → aggregate best chunk back to its note.
//!
//! Independent store with its own ranking — the RRF/MMR *algorithms* are reused
//! from the memory backend, but notes never mix into `recall_memory` (D7).

use std::collections::HashMap;

use super::db::IndexDb;
use super::types::NoteSearchHit;
use crate::util::truncate_utf8;

// Mirror the memory backend's fusion weights / constants for parity.
const TEXT_WEIGHT: f64 = 0.4;
const VECTOR_WEIGHT: f64 = 0.6;
const RRF_K: f64 = 60.0;
const MMR_LAMBDA: f32 = 0.7;
const SNIPPET_BYTES: usize = 320;

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
    let fetch = (limit * 3).max(10);

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
        *chunk_score.entry(*chunk_id).or_insert(0.0) += TEXT_WEIGHT / (RRF_K + rank as f64 + 1.0);
        chunk_note.insert(*chunk_id, *note_id);
    }
    for (rank, (chunk_id, note_id, _)) in vec.iter().enumerate() {
        *chunk_score.entry(*chunk_id).or_insert(0.0) += VECTOR_WEIGHT / (RRF_K + rank as f64 + 1.0);
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
    let reranked = crate::memory::mmr::mmr_rerank(&candidate_refs, limit, MMR_LAMBDA);

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
            note_id,
            rel_path: rel_path.clone(),
            title: title.clone(),
            score,
            snippet,
            heading_path,
            start_line,
        });
    }
    Ok(hits)
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
    Ok(out)
}
