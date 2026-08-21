//! Deterministic Knowledge retrieval/evidence contract used only by the
//! explicit hope-agent-eval runner. It performs no network or model calls and
//! uses a disposable index cache.

use anyhow::Result;
use serde::Deserialize;

use super::db::{IndexDb, NoteIndexInput};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeRetrievalFixture {
    #[serde(default)]
    pub chunk_max_chars: Option<usize>,
    pub notes: Vec<KnowledgeEvalNote>,
    pub queries: Vec<KnowledgeEvalQuery>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeEvalNote {
    pub kb_id: String,
    pub rel_path: String,
    pub markdown: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeEvalQuery {
    pub id: String,
    pub kb_ids: Vec<String>,
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub expected_paths: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    #[serde(default)]
    pub expected_heading: Option<String>,
    #[serde(default)]
    pub require_source_coordinates: bool,
}

fn default_limit() -> usize {
    8
}

#[derive(Debug, Clone)]
pub struct KnowledgeEvalOutcome {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct KnowledgeRetrievalEvalReport {
    pub outcomes: Vec<KnowledgeEvalOutcome>,
}

impl KnowledgeRetrievalEvalReport {
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(|outcome| outcome.passed)
    }
}

pub fn evaluate(
    db: &IndexDb,
    fixture: &KnowledgeRetrievalFixture,
) -> Result<KnowledgeRetrievalEvalReport> {
    let chunk_config = super::chunker::ChunkConfig {
        max_chars: fixture.chunk_max_chars.unwrap_or(1_500),
        overlap_chars: 80,
    }
    .clamped();
    for note in &fixture.notes {
        let parsed = super::parser::parse_document(&note.markdown);
        let chunks = super::chunker::chunk(&note.markdown, &parsed, &chunk_config);
        db.replace_note_index(NoteIndexInput {
            kb_id: note.kb_id.clone(),
            rel_path: note.rel_path.clone(),
            title: parsed
                .title
                .clone()
                .unwrap_or_else(|| note.rel_path.clone()),
            frontmatter_json: parsed.frontmatter_json,
            mtime: 1,
            size: note.markdown.len() as i64,
            content_hash: super::blake3_hex(note.markdown.as_bytes()),
            chunks,
            chunk_embeddings: None,
            embedding_signature: None,
            links: parsed.links,
            tags: parsed.tags,
        })?;
    }

    let mut outcomes = Vec::with_capacity(fixture.queries.len());
    for query in &fixture.queries {
        let hits =
            super::search::search_notes(db, &query.kb_ids, &query.query, query.limit.clamp(1, 50))?;
        let actual = hits
            .iter()
            .map(|hit| format!("{}:{}", hit.kb_id, hit.rel_path))
            .collect::<Vec<_>>();
        let missing = query
            .expected_paths
            .iter()
            .filter(|expected| !actual.contains(expected))
            .cloned()
            .collect::<Vec<_>>();
        let leaked = query
            .forbidden_paths
            .iter()
            .filter(|forbidden| actual.contains(forbidden))
            .cloned()
            .collect::<Vec<_>>();
        let heading_ok = query.expected_heading.as_ref().is_none_or(|expected| {
            hits.iter()
                .any(|hit| hit.heading_path.as_deref() == Some(expected.as_str()))
        });
        let coordinates_ok = !query.require_source_coordinates
            || hits
                .iter()
                .filter(|hit| {
                    query
                        .expected_paths
                        .contains(&format!("{}:{}", hit.kb_id, hit.rel_path))
                })
                .all(|hit| hit.start_line > 0 && !hit.snippet.trim().is_empty());
        let passed = missing.is_empty() && leaked.is_empty() && heading_ok && coordinates_ok;
        outcomes.push(KnowledgeEvalOutcome {
            name: query.id.clone(),
            passed,
            detail: format!(
                "actual={actual:?}; missing={missing:?}; leaked={leaked:?}; heading_ok={heading_ok}; coordinates_ok={coordinates_ok}"
            ),
        });
    }
    Ok(KnowledgeRetrievalEvalReport { outcomes })
}
