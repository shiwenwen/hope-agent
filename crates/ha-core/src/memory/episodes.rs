//! Episodic / procedural memory foundation (P5).
//!
//! This module is intentionally owner-plane only in the first iteration. It
//! gives the Memory Center a durable place to store "what happened / what we
//! learned" and user-promoted procedures. Episodes remain owner-plane and
//! trace-only; high-confidence user-saved procedures can be injected later as
//! bounded soft workflow guidance by the agent layer. No agent tools are
//! exposed from this module.

use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row, Transaction,
};

use crate::memory::{MemoryScope, SqliteMemoryBackend};
use crate::util::now_rfc3339;

const DEFAULT_PAGE_LIMIT: usize = 20;
const MAX_PAGE_LIMIT: usize = 100;
const MAX_QUERY_CHARS: usize = 200;
const MAX_TITLE_CHARS: usize = 160;
const MAX_TEXT_CHARS: usize = 4_000;
const MAX_LIST_ITEMS: usize = 20;
const MAX_TAGS: usize = 20;
const EXPERIENCE_SHORTLIST_POOL_LIMIT: usize = 100;
const MAX_HISTORY_PREVIEW_CHARS: usize = 600;

static EPISODE_STORE: OnceLock<EpisodeStore> = OnceLock::new();

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEpisodeRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub title: String,
    pub situation: String,
    pub actions: Vec<String>,
    pub outcome: String,
    pub lesson: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
    pub success_score: f32,
    #[serde(default)]
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemoryEpisode {
    pub scope: MemoryScope,
    pub title: String,
    pub situation: String,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub lesson: String,
    #[serde(default)]
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub source_message_ids: Vec<String>,
    #[serde(default)]
    pub success_score: Option<f32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEpisodePatch {
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub situation: Option<String>,
    #[serde(default)]
    pub actions: Option<Vec<String>>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub lesson: Option<String>,
    #[serde(default)]
    pub success_score: Option<f32>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEpisodeQuery {
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEpisodeListPage {
    pub items: Vec<MemoryEpisodeRecord>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProcedureRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub title: String,
    pub trigger: String,
    pub steps_markdown: String,
    pub constraints_markdown: String,
    pub confidence: f32,
    pub status: String,
    #[serde(default)]
    pub source_episode_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemoryProcedure {
    pub scope: MemoryScope,
    pub title: String,
    pub trigger: String,
    pub steps_markdown: String,
    #[serde(default)]
    pub constraints_markdown: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub source_episode_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProcedurePatch {
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub steps_markdown: Option<String>,
    #[serde(default)]
    pub constraints_markdown: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub source_episode_ids: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProcedureQuery {
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProcedureListPage {
    pub items: Vec<MemoryProcedureRecord>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExperienceCandidate {
    /// `episode` or `procedure`.
    pub kind: String,
    pub id: String,
    pub scope: MemoryScope,
    pub title: String,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteEpisodeOptions {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub steps_markdown: Option<String>,
    #[serde(default)]
    pub constraints_markdown: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExperienceHistoryQuery {
    #[serde(default)]
    pub target_kind: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub actions: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExperienceHistoryRecord {
    pub id: String,
    /// `episode` or `procedure`.
    pub target_kind: String,
    pub target_id: String,
    /// `add`, `promote`, `update`, `archive`, `restore`, or `restore_import`.
    pub action: String,
    pub scope: MemoryScope,
    pub title_preview: String,
    pub content_preview: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExperienceHistoryListPage {
    pub items: Vec<MemoryExperienceHistoryRecord>,
    pub total: usize,
    #[serde(default)]
    pub total_truncated: bool,
}

pub struct EpisodeStore {
    backend: Arc<SqliteMemoryBackend>,
}

impl EpisodeStore {
    fn new(backend: Arc<SqliteMemoryBackend>) -> Self {
        Self { backend }
    }
}

pub fn init_episode_store(backend: Arc<SqliteMemoryBackend>) {
    let _ = EPISODE_STORE.set(EpisodeStore::new(backend));
}

fn store() -> Result<&'static EpisodeStore> {
    EPISODE_STORE
        .get()
        .ok_or_else(|| anyhow!("episode store not initialised"))
}

pub fn add_episode(input: NewMemoryEpisode) -> Result<MemoryEpisodeRecord> {
    store()?.add_episode(input)
}

pub fn list_episodes_page(query: MemoryEpisodeQuery) -> Result<MemoryEpisodeListPage> {
    store()?.list_episodes_page(&query)
}

pub fn get_episode(id: &str) -> Result<Option<MemoryEpisodeRecord>> {
    store()?.get_episode(id)
}

pub fn update_episode(id: &str, patch: MemoryEpisodePatch) -> Result<Option<MemoryEpisodeRecord>> {
    store()?.update_episode(id, patch)
}

pub fn archive_episode(id: &str) -> Result<bool> {
    store()?.archive_episode(id)
}

pub fn restore_episode(id: &str) -> Result<bool> {
    store()?.restore_episode(id)
}

pub fn restore_episode_record(record: &MemoryEpisodeRecord) -> Result<bool> {
    store()?.restore_episode_record(record)
}

pub fn add_procedure(input: NewMemoryProcedure) -> Result<MemoryProcedureRecord> {
    store()?.add_procedure(input)
}

pub fn promote_episode_to_procedure(
    id: &str,
    options: PromoteEpisodeOptions,
) -> Result<MemoryProcedureRecord> {
    store()?.promote_episode_to_procedure(id, options)
}

pub fn list_procedures_page(query: MemoryProcedureQuery) -> Result<MemoryProcedureListPage> {
    store()?.list_procedures_page(&query)
}

pub fn get_procedure(id: &str) -> Result<Option<MemoryProcedureRecord>> {
    store()?.get_procedure(id)
}

pub fn update_procedure(
    id: &str,
    patch: MemoryProcedurePatch,
) -> Result<Option<MemoryProcedureRecord>> {
    store()?.update_procedure(id, patch)
}

pub fn archive_procedure(id: &str) -> Result<bool> {
    store()?.archive_procedure(id)
}

pub fn restore_procedure(id: &str) -> Result<bool> {
    store()?.restore_procedure(id)
}

pub fn restore_procedure_record(record: &MemoryProcedureRecord) -> Result<bool> {
    store()?.restore_procedure_record(record)
}

pub fn restore_experience_history_record(record: &MemoryExperienceHistoryRecord) -> Result<bool> {
    store()?.restore_experience_history_record(record)
}

pub fn list_experience_history_page(
    query: MemoryExperienceHistoryQuery,
) -> Result<MemoryExperienceHistoryListPage> {
    store()?.list_experience_history_page(&query)
}

pub fn shortlist_experience_candidates(
    query: &str,
    scopes: &[MemoryScope],
    limit: usize,
) -> Vec<MemoryExperienceCandidate> {
    store()
        .and_then(|s| s.shortlist_experience_candidates(query, scopes, limit))
        .unwrap_or_default()
}

impl EpisodeStore {
    fn add_episode(&self, input: NewMemoryEpisode) -> Result<MemoryEpisodeRecord> {
        let (scope_type, scope_id) = scope_to_parts(&input.scope);
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let title = clean_required(&input.title, MAX_TITLE_CHARS, "episode title")?;
        let situation = clean_required(&input.situation, MAX_TEXT_CHARS, "episode situation")?;
        let actions = clean_vec(input.actions, MAX_LIST_ITEMS, MAX_TEXT_CHARS);
        let outcome = clean_text(&input.outcome, MAX_TEXT_CHARS);
        let lesson = clean_text(&input.lesson, MAX_TEXT_CHARS);
        let source_message_ids = clean_vec(input.source_message_ids, MAX_LIST_ITEMS, 128);
        let tags = clean_vec(input.tags, MAX_TAGS, 64);
        let success_score = input.success_score.unwrap_or(0.5).clamp(0.0, 1.0);
        let actions_json = serde_json::to_string(&actions)?;
        let source_message_ids_json = serde_json::to_string(&source_message_ids)?;
        let tags_json = serde_json::to_string(&tags)?;
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO memory_episodes (
                id, scope_type, scope_id, title, situation, actions_json,
                outcome, lesson, source_session_id, source_message_ids_json,
                success_score, tags_json, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14)",
            params![
                id,
                scope_type,
                scope_id,
                title,
                situation,
                actions_json,
                outcome,
                lesson,
                input.source_session_id,
                source_message_ids_json,
                success_score,
                tags_json,
                now,
                now,
            ],
        )?;
        append_experience_history(
            &tx,
            "episode",
            &id,
            "add",
            &input.scope,
            &title,
            &episode_history_preview(&title, &situation, &actions, &outcome, &lesson),
            &now,
        )?;
        tx.commit()?;
        drop(conn);
        self.get_episode(&id)?
            .ok_or_else(|| anyhow!("episode inserted but not found: {id}"))
    }

    fn list_episodes_page(&self, query: &MemoryEpisodeQuery) -> Result<MemoryEpisodeListPage> {
        let (where_sql, args) = episode_where(query);
        let limit = page_limit(query.limit);
        let offset = query.offset.unwrap_or(0);
        let total = self.count_rows("memory_episodes", &where_sql, &args)?;
        let mut page_args = args.clone();
        page_args.push(SqlValue::Integer(limit as i64));
        page_args.push(SqlValue::Integer(offset as i64));
        let order_sql = episode_order_sql(query.sort.as_deref());
        let sql = format!(
            "SELECT id, scope_type, scope_id, title, situation, actions_json,
                    outcome, lesson, source_session_id, source_message_ids_json,
                    success_score, tags_json, status, created_at, updated_at
             FROM memory_episodes
             {where_sql}
             ORDER BY {order_sql}
             LIMIT ?{} OFFSET ?{}",
            page_args.len() - 1,
            page_args.len()
        );
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(page_args), row_to_episode)?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(MemoryEpisodeListPage {
            items,
            total,
            total_truncated: false,
        })
    }

    fn get_episode(&self, id: &str) -> Result<Option<MemoryEpisodeRecord>> {
        let conn = self.backend.read_conn()?;
        conn.query_row(
            "SELECT id, scope_type, scope_id, title, situation, actions_json,
                    outcome, lesson, source_session_id, source_message_ids_json,
                    success_score, tags_json, status, created_at, updated_at
             FROM memory_episodes
             WHERE id = ?1",
            params![id],
            row_to_episode,
        )
        .optional()
        .map_err(Into::into)
    }

    fn update_episode(
        &self,
        id: &str,
        patch: MemoryEpisodePatch,
    ) -> Result<Option<MemoryEpisodeRecord>> {
        let Some(existing) = self.get_episode(id)? else {
            return Ok(None);
        };
        let scope = patch.scope.unwrap_or(existing.scope);
        let (scope_type, scope_id) = scope_to_parts(&scope);
        let title = match patch.title {
            Some(value) => clean_required(&value, MAX_TITLE_CHARS, "episode title")?,
            None => existing.title,
        };
        let situation = match patch.situation {
            Some(value) => clean_required(&value, MAX_TEXT_CHARS, "episode situation")?,
            None => existing.situation,
        };
        let actions = patch
            .actions
            .map(|values| clean_vec(values, MAX_LIST_ITEMS, MAX_TEXT_CHARS))
            .unwrap_or(existing.actions);
        let outcome = patch
            .outcome
            .map(|value| clean_text(&value, MAX_TEXT_CHARS))
            .unwrap_or(existing.outcome);
        let lesson = patch
            .lesson
            .map(|value| clean_text(&value, MAX_TEXT_CHARS))
            .unwrap_or(existing.lesson);
        let success_score = patch
            .success_score
            .unwrap_or(existing.success_score)
            .clamp(0.0, 1.0);
        let tags = patch
            .tags
            .map(|values| clean_vec(values, MAX_TAGS, 64))
            .unwrap_or(existing.tags);
        let actions_json = serde_json::to_string(&actions)?;
        let tags_json = serde_json::to_string(&tags)?;
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE memory_episodes
             SET scope_type = ?2, scope_id = ?3, title = ?4, situation = ?5,
                 actions_json = ?6, outcome = ?7, lesson = ?8,
                 success_score = ?9, tags_json = ?10, updated_at = ?11
             WHERE id = ?1",
            params![
                id,
                scope_type,
                scope_id,
                title,
                situation,
                actions_json,
                outcome,
                lesson,
                success_score,
                tags_json,
                now,
            ],
        )?;
        append_experience_history(
            &tx,
            "episode",
            id,
            "update",
            &scope,
            &title,
            &episode_history_preview(&title, &situation, &actions, &outcome, &lesson),
            &now,
        )?;
        tx.commit()?;
        drop(conn);
        self.get_episode(id)
    }

    fn archive_episode(&self, id: &str) -> Result<bool> {
        let Some(existing) = self.get_episode(id)? else {
            return Ok(false);
        };
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE memory_episodes
             SET status = 'archived', updated_at = ?2
             WHERE id = ?1 AND status != 'archived'",
            params![id, now],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "episode",
                id,
                "archive",
                &existing.scope,
                &existing.title,
                &episode_record_history_preview(&existing),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn restore_episode(&self, id: &str) -> Result<bool> {
        let Some(existing) = self.get_episode(id)? else {
            return Ok(false);
        };
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE memory_episodes
             SET status = 'active', updated_at = ?2
             WHERE id = ?1 AND status = 'archived'",
            params![id, now],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "episode",
                id,
                "restore",
                &existing.scope,
                &existing.title,
                &episode_record_history_preview(&existing),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn restore_episode_record(&self, record: &MemoryEpisodeRecord) -> Result<bool> {
        let id = clean_required(&record.id, 128, "episode id")?;
        let (scope_type, scope_id) = scope_to_parts(&record.scope);
        let title = clean_required(&record.title, MAX_TITLE_CHARS, "episode title")?;
        let situation = clean_required(&record.situation, MAX_TEXT_CHARS, "episode situation")?;
        let actions = clean_vec(record.actions.clone(), MAX_LIST_ITEMS, MAX_TEXT_CHARS);
        let outcome = clean_text(&record.outcome, MAX_TEXT_CHARS);
        let lesson = clean_text(&record.lesson, MAX_TEXT_CHARS);
        let source_message_ids = clean_vec(record.source_message_ids.clone(), MAX_LIST_ITEMS, 128);
        let tags = clean_vec(record.tags.clone(), MAX_TAGS, 64);
        let status = normalize_status(&record.status);
        let created_at = non_empty_or_now(&record.created_at);
        let updated_at = non_empty_or_now(&record.updated_at);
        let actions_json = serde_json::to_string(&actions)?;
        let source_message_ids_json = serde_json::to_string(&source_message_ids)?;
        let tags_json = serde_json::to_string(&tags)?;
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "INSERT OR IGNORE INTO memory_episodes (
                id, scope_type, scope_id, title, situation, actions_json,
                outcome, lesson, source_session_id, source_message_ids_json,
                success_score, tags_json, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                scope_type,
                scope_id,
                title,
                situation,
                actions_json,
                outcome,
                lesson,
                record.source_session_id,
                source_message_ids_json,
                record.success_score.clamp(0.0, 1.0),
                tags_json,
                status,
                created_at,
                updated_at,
            ],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "episode",
                &id,
                "restore_import",
                &record.scope,
                &title,
                &episode_history_preview(&title, &situation, &actions, &outcome, &lesson),
                &updated_at,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn add_procedure(&self, input: NewMemoryProcedure) -> Result<MemoryProcedureRecord> {
        self.insert_procedure(input, "add")
    }

    fn insert_procedure(
        &self,
        input: NewMemoryProcedure,
        history_action: &str,
    ) -> Result<MemoryProcedureRecord> {
        let (scope_type, scope_id) = scope_to_parts(&input.scope);
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let title = clean_required(&input.title, MAX_TITLE_CHARS, "procedure title")?;
        let trigger = clean_required(&input.trigger, MAX_TEXT_CHARS, "procedure trigger")?;
        let steps_markdown =
            clean_required(&input.steps_markdown, MAX_TEXT_CHARS, "procedure steps")?;
        let constraints_markdown = clean_text(&input.constraints_markdown, MAX_TEXT_CHARS);
        let source_episode_ids = clean_vec(input.source_episode_ids, MAX_LIST_ITEMS, 128);
        let tags = clean_vec(input.tags, MAX_TAGS, 64);
        let confidence = input.confidence.unwrap_or(0.5).clamp(0.0, 1.0);
        let source_episode_ids_json = serde_json::to_string(&source_episode_ids)?;
        let tags_json = serde_json::to_string(&tags)?;
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO memory_procedures (
                id, scope_type, scope_id, title, trigger, steps_markdown,
                constraints_markdown, confidence, status, source_episode_ids_json,
                tags_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?10, ?11, ?12)",
            params![
                id,
                scope_type,
                scope_id,
                title,
                trigger,
                steps_markdown,
                constraints_markdown,
                confidence,
                source_episode_ids_json,
                tags_json,
                now,
                now,
            ],
        )?;
        append_experience_history(
            &tx,
            "procedure",
            &id,
            history_action,
            &input.scope,
            &title,
            &procedure_history_preview(
                &title,
                &trigger,
                &steps_markdown,
                &constraints_markdown,
                confidence,
            ),
            &now,
        )?;
        tx.commit()?;
        drop(conn);
        self.get_procedure(&id)?
            .ok_or_else(|| anyhow!("procedure inserted but not found: {id}"))
    }

    fn promote_episode_to_procedure(
        &self,
        id: &str,
        options: PromoteEpisodeOptions,
    ) -> Result<MemoryProcedureRecord> {
        let episode = self
            .get_episode(id)?
            .ok_or_else(|| anyhow!("episode not found: {id}"))?;
        let steps_markdown = options
            .steps_markdown
            .map(|s| clean_text(&s, MAX_TEXT_CHARS))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| actions_to_markdown(&episode.actions, &episode.outcome));
        let constraints_markdown = options
            .constraints_markdown
            .map(|s| clean_text(&s, MAX_TEXT_CHARS))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let mut parts = Vec::new();
                if !episode.lesson.trim().is_empty() {
                    parts.push(format!("Lesson: {}", episode.lesson.trim()));
                }
                if !episode.situation.trim().is_empty() {
                    parts.push(format!("Context: {}", episode.situation.trim()));
                }
                parts.join("\n")
            });
        self.insert_procedure(
            NewMemoryProcedure {
                scope: episode.scope,
                title: options
                    .title
                    .map(|s| clean_text(&s, MAX_TITLE_CHARS))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| episode.title.clone()),
                trigger: options
                    .trigger
                    .map(|s| clean_text(&s, MAX_TEXT_CHARS))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| episode.situation.clone()),
                steps_markdown,
                constraints_markdown,
                confidence: Some(episode.success_score),
                source_episode_ids: vec![episode.id],
                tags: episode.tags,
            },
            "promote",
        )
    }

    fn list_procedures_page(
        &self,
        query: &MemoryProcedureQuery,
    ) -> Result<MemoryProcedureListPage> {
        let (where_sql, args) = procedure_where(query);
        let limit = page_limit(query.limit);
        let offset = query.offset.unwrap_or(0);
        let total = self.count_rows("memory_procedures", &where_sql, &args)?;
        let mut page_args = args.clone();
        page_args.push(SqlValue::Integer(limit as i64));
        page_args.push(SqlValue::Integer(offset as i64));
        let order_sql = procedure_order_sql(query.sort.as_deref());
        let sql = format!(
            "SELECT id, scope_type, scope_id, title, trigger, steps_markdown,
                    constraints_markdown, confidence, status, source_episode_ids_json,
                    tags_json, created_at, updated_at
             FROM memory_procedures
             {where_sql}
             ORDER BY {order_sql}
             LIMIT ?{} OFFSET ?{}",
            page_args.len() - 1,
            page_args.len()
        );
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(page_args), row_to_procedure)?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(MemoryProcedureListPage {
            items,
            total,
            total_truncated: false,
        })
    }

    fn get_procedure(&self, id: &str) -> Result<Option<MemoryProcedureRecord>> {
        let conn = self.backend.read_conn()?;
        conn.query_row(
            "SELECT id, scope_type, scope_id, title, trigger, steps_markdown,
                    constraints_markdown, confidence, status, source_episode_ids_json,
                    tags_json, created_at, updated_at
             FROM memory_procedures
             WHERE id = ?1",
            params![id],
            row_to_procedure,
        )
        .optional()
        .map_err(Into::into)
    }

    fn update_procedure(
        &self,
        id: &str,
        patch: MemoryProcedurePatch,
    ) -> Result<Option<MemoryProcedureRecord>> {
        let Some(existing) = self.get_procedure(id)? else {
            return Ok(None);
        };
        let scope = patch.scope.unwrap_or(existing.scope);
        let (scope_type, scope_id) = scope_to_parts(&scope);
        let title = match patch.title {
            Some(value) => clean_required(&value, MAX_TITLE_CHARS, "procedure title")?,
            None => existing.title,
        };
        let trigger = match patch.trigger {
            Some(value) => clean_required(&value, MAX_TEXT_CHARS, "procedure trigger")?,
            None => existing.trigger,
        };
        let steps_markdown = match patch.steps_markdown {
            Some(value) => clean_required(&value, MAX_TEXT_CHARS, "procedure steps")?,
            None => existing.steps_markdown,
        };
        let constraints_markdown = patch
            .constraints_markdown
            .map(|value| clean_text(&value, MAX_TEXT_CHARS))
            .unwrap_or(existing.constraints_markdown);
        let confidence = patch
            .confidence
            .unwrap_or(existing.confidence)
            .clamp(0.0, 1.0);
        let source_episode_ids = patch
            .source_episode_ids
            .map(|values| clean_vec(values, MAX_LIST_ITEMS, 128))
            .unwrap_or(existing.source_episode_ids);
        let tags = patch
            .tags
            .map(|values| clean_vec(values, MAX_TAGS, 64))
            .unwrap_or(existing.tags);
        let source_episode_ids_json = serde_json::to_string(&source_episode_ids)?;
        let tags_json = serde_json::to_string(&tags)?;
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE memory_procedures
             SET scope_type = ?2, scope_id = ?3, title = ?4, trigger = ?5,
                 steps_markdown = ?6, constraints_markdown = ?7,
                 confidence = ?8, source_episode_ids_json = ?9,
                 tags_json = ?10, updated_at = ?11
             WHERE id = ?1",
            params![
                id,
                scope_type,
                scope_id,
                title,
                trigger,
                steps_markdown,
                constraints_markdown,
                confidence,
                source_episode_ids_json,
                tags_json,
                now,
            ],
        )?;
        append_experience_history(
            &tx,
            "procedure",
            id,
            "update",
            &scope,
            &title,
            &procedure_history_preview(
                &title,
                &trigger,
                &steps_markdown,
                &constraints_markdown,
                confidence,
            ),
            &now,
        )?;
        tx.commit()?;
        drop(conn);
        self.get_procedure(id)
    }

    fn archive_procedure(&self, id: &str) -> Result<bool> {
        let Some(existing) = self.get_procedure(id)? else {
            return Ok(false);
        };
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE memory_procedures
             SET status = 'archived', updated_at = ?2
             WHERE id = ?1 AND status != 'archived'",
            params![id, now],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "procedure",
                id,
                "archive",
                &existing.scope,
                &existing.title,
                &procedure_record_history_preview(&existing),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn restore_procedure(&self, id: &str) -> Result<bool> {
        let Some(existing) = self.get_procedure(id)? else {
            return Ok(false);
        };
        let now = now_rfc3339();
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE memory_procedures
             SET status = 'active', updated_at = ?2
             WHERE id = ?1 AND status = 'archived'",
            params![id, now],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "procedure",
                id,
                "restore",
                &existing.scope,
                &existing.title,
                &procedure_record_history_preview(&existing),
                &now,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn restore_procedure_record(&self, record: &MemoryProcedureRecord) -> Result<bool> {
        let id = clean_required(&record.id, 128, "procedure id")?;
        let (scope_type, scope_id) = scope_to_parts(&record.scope);
        let title = clean_required(&record.title, MAX_TITLE_CHARS, "procedure title")?;
        let trigger = clean_required(&record.trigger, MAX_TEXT_CHARS, "procedure trigger")?;
        let steps_markdown =
            clean_required(&record.steps_markdown, MAX_TEXT_CHARS, "procedure steps")?;
        let constraints_markdown = clean_text(&record.constraints_markdown, MAX_TEXT_CHARS);
        let source_episode_ids = clean_vec(record.source_episode_ids.clone(), MAX_LIST_ITEMS, 128);
        let tags = clean_vec(record.tags.clone(), MAX_TAGS, 64);
        let status = normalize_status(&record.status);
        let created_at = non_empty_or_now(&record.created_at);
        let updated_at = non_empty_or_now(&record.updated_at);
        let source_episode_ids_json = serde_json::to_string(&source_episode_ids)?;
        let tags_json = serde_json::to_string(&tags)?;
        let mut conn = self.backend.write_conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "INSERT OR IGNORE INTO memory_procedures (
                id, scope_type, scope_id, title, trigger, steps_markdown,
                constraints_markdown, confidence, status, source_episode_ids_json,
                tags_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                scope_type,
                scope_id,
                title,
                trigger,
                steps_markdown,
                constraints_markdown,
                record.confidence.clamp(0.0, 1.0),
                status,
                source_episode_ids_json,
                tags_json,
                created_at,
                updated_at,
            ],
        )?;
        if changed > 0 {
            append_experience_history(
                &tx,
                "procedure",
                &id,
                "restore_import",
                &record.scope,
                &title,
                &procedure_history_preview(
                    &title,
                    &trigger,
                    &steps_markdown,
                    &constraints_markdown,
                    record.confidence.clamp(0.0, 1.0),
                ),
                &updated_at,
            )?;
        }
        tx.commit()?;
        Ok(changed > 0)
    }

    fn restore_experience_history_record(
        &self,
        record: &MemoryExperienceHistoryRecord,
    ) -> Result<bool> {
        let id = clean_required(&record.id, 128, "experience history id")?;
        let target_kind = normalize_experience_kind(&record.target_kind);
        let target_id = clean_required(&record.target_id, 128, "experience history target id")?;
        let action = normalize_history_action(&record.action);
        let (scope_type, scope_id) = scope_to_parts(&record.scope);
        let title_preview = truncate_history_preview(&clean_text(
            &record.title_preview,
            MAX_HISTORY_PREVIEW_CHARS,
        ));
        let content_preview = truncate_history_preview(&clean_text(
            &record.content_preview,
            MAX_HISTORY_PREVIEW_CHARS,
        ));
        let created_at = non_empty_or_now(&record.created_at);
        let conn = self.backend.write_conn()?;
        let changed = conn.execute(
            "INSERT OR IGNORE INTO memory_experience_history (
                id, target_kind, target_id, action, scope_type, scope_id,
                title_preview, content_preview, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                target_kind,
                target_id,
                action,
                scope_type,
                scope_id,
                title_preview,
                content_preview,
                created_at,
            ],
        )?;
        Ok(changed > 0)
    }

    fn list_experience_history_page(
        &self,
        query: &MemoryExperienceHistoryQuery,
    ) -> Result<MemoryExperienceHistoryListPage> {
        let (where_sql, args) = experience_history_where(query);
        let limit = page_limit(query.limit);
        let offset = query.offset.unwrap_or(0);
        let total = self.count_rows("memory_experience_history", &where_sql, &args)?;
        let mut page_args = args.clone();
        page_args.push(SqlValue::Integer(limit as i64));
        page_args.push(SqlValue::Integer(offset as i64));
        let sql = format!(
            "SELECT id, target_kind, target_id, action, scope_type, scope_id,
                    title_preview, content_preview, created_at
             FROM memory_experience_history
             {where_sql}
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?{} OFFSET ?{}",
            page_args.len() - 1,
            page_args.len()
        );
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(page_args), row_to_experience_history)?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(MemoryExperienceHistoryListPage {
            items,
            total,
            total_truncated: false,
        })
    }

    fn shortlist_experience_candidates(
        &self,
        query: &str,
        scopes: &[MemoryScope],
        limit: usize,
    ) -> Result<Vec<MemoryExperienceCandidate>> {
        let trimmed = query.trim();
        if trimmed.is_empty() || scopes.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let per_scope = EXPERIENCE_SHORTLIST_POOL_LIMIT.min(MAX_PAGE_LIMIT);

        for scope in scopes {
            let procedures = self.list_procedures_page(&MemoryProcedureQuery {
                scope: Some(scope.clone()),
                status: Some("active".to_string()),
                query: None,
                sort: Some("updated_desc".to_string()),
                limit: Some(per_scope),
                offset: Some(0),
            })?;
            let episodes = self.list_episodes_page(&MemoryEpisodeQuery {
                scope: Some(scope.clone()),
                status: Some("active".to_string()),
                query: None,
                sort: Some("updated_desc".to_string()),
                limit: Some(per_scope),
                offset: Some(0),
            })?;

            let mut scoped = Vec::new();
            for procedure in procedures.items {
                let score = score_procedure_for_query(&procedure, trimmed);
                if score > 0.0 {
                    scoped.push(ScoredExperience::Procedure {
                        record: procedure,
                        score,
                    });
                }
            }
            for episode in episodes.items {
                let score = score_episode_for_query(&episode, trimmed);
                if score > 0.0 {
                    scoped.push(ScoredExperience::Episode {
                        record: episode,
                        score,
                    });
                }
            }
            scoped.sort_by(compare_scored_experience);

            for item in scoped {
                let key = item.key();
                if !seen.insert(key) {
                    continue;
                }
                out.push(item.into_candidate());
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }

        Ok(out)
    }

    fn count_rows(&self, table: &str, where_sql: &str, args: &[SqlValue]) -> Result<usize> {
        let sql = format!("SELECT COUNT(*) FROM {table} {where_sql}");
        let conn = self.backend.read_conn()?;
        let count = conn.query_row(&sql, params_from_iter(args.iter().cloned()), |row| {
            row.get::<_, i64>(0)
        })?;
        Ok(count.max(0) as usize)
    }
}

fn append_experience_history(
    tx: &Transaction<'_>,
    target_kind: &str,
    target_id: &str,
    action: &str,
    scope: &MemoryScope,
    title: &str,
    content: &str,
    created_at: &str,
) -> Result<()> {
    let (scope_type, scope_id) = scope_to_parts(scope);
    let id = uuid::Uuid::new_v4().to_string();
    let title_preview = truncate_history_preview(title);
    let content_preview = truncate_history_preview(content);
    tx.execute(
        "INSERT INTO memory_experience_history (
            id, target_kind, target_id, action, scope_type, scope_id,
            title_preview, content_preview, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            normalize_experience_kind(target_kind),
            target_id,
            normalize_history_action(action),
            scope_type,
            scope_id,
            title_preview,
            content_preview,
            created_at,
        ],
    )?;
    Ok(())
}

fn page_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT)
}

fn truncate_history_preview(value: &str) -> String {
    crate::truncate_utf8(value.trim(), MAX_HISTORY_PREVIEW_CHARS).to_string()
}

fn clean_text(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn clean_required(value: &str, max_chars: usize, label: &str) -> Result<String> {
    let cleaned = clean_text(value, max_chars);
    if cleaned.is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(cleaned)
}

fn normalize_status(value: &str) -> &'static str {
    match value.trim() {
        "archived" => "archived",
        _ => "active",
    }
}

fn normalize_experience_kind(value: &str) -> &'static str {
    match value.trim() {
        "procedure" => "procedure",
        _ => "episode",
    }
}

fn normalize_history_action(value: &str) -> &'static str {
    match value.trim() {
        "promote" => "promote",
        "update" => "update",
        "archive" => "archive",
        "restore" => "restore",
        "restore_import" => "restore_import",
        _ => "add",
    }
}

fn history_action_filter_value(value: &str) -> Option<&'static str> {
    match value.trim() {
        "add" => Some("add"),
        "promote" => Some("promote"),
        "update" => Some("update"),
        "archive" => Some("archive"),
        "restore" => Some("restore"),
        "restore_import" => Some("restore_import"),
        _ => None,
    }
}

fn non_empty_or_now(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        now_rfc3339()
    } else {
        trimmed.to_string()
    }
}

fn clean_vec(values: Vec<String>, max_items: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let cleaned = clean_text(&value, max_chars);
        if cleaned.is_empty() || out.iter().any(|existing| existing == &cleaned) {
            continue;
        }
        out.push(cleaned);
        if out.len() >= max_items {
            break;
        }
    }
    out
}

fn scope_to_parts(scope: &MemoryScope) -> (&'static str, Option<String>) {
    match scope {
        MemoryScope::Global => ("global", None),
        MemoryScope::Agent { id } => ("agent", Some(id.clone())),
        MemoryScope::Project { id } => ("project", Some(id.clone())),
    }
}

fn scope_from_parts(scope_type: String, scope_id: Option<String>) -> MemoryScope {
    match scope_type.as_str() {
        "agent" => MemoryScope::Agent {
            id: scope_id.unwrap_or_default(),
        },
        "project" => MemoryScope::Project {
            id: scope_id.unwrap_or_default(),
        },
        _ => MemoryScope::Global,
    }
}

fn parse_vec_json(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

enum ScoredExperience {
    Procedure {
        record: MemoryProcedureRecord,
        score: f32,
    },
    Episode {
        record: MemoryEpisodeRecord,
        score: f32,
    },
}

impl ScoredExperience {
    fn key(&self) -> String {
        match self {
            Self::Procedure { record, .. } => format!("procedure:{}", record.id),
            Self::Episode { record, .. } => format!("episode:{}", record.id),
        }
    }

    fn rank_score(&self) -> f32 {
        match self {
            Self::Procedure { record, score } => {
                *score + record.confidence.clamp(0.0, 1.0) * 0.25 + 0.08
            }
            Self::Episode { record, score } => *score + record.success_score.clamp(0.0, 1.0) * 0.2,
        }
    }

    fn updated_at(&self) -> &str {
        match self {
            Self::Procedure { record, .. } => &record.updated_at,
            Self::Episode { record, .. } => &record.updated_at,
        }
    }

    fn into_candidate(self) -> MemoryExperienceCandidate {
        match self {
            Self::Procedure { record, .. } => experience_candidate_from_procedure(record),
            Self::Episode { record, .. } => experience_candidate_from_episode(record),
        }
    }
}

fn compare_scored_experience(a: &ScoredExperience, b: &ScoredExperience) -> std::cmp::Ordering {
    b.rank_score()
        .partial_cmp(&a.rank_score())
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.updated_at().cmp(a.updated_at()))
        .then_with(|| a.key().cmp(&b.key()))
}

fn query_terms(query: &str) -> Vec<String> {
    let lowered = query
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect::<String>()
        .to_lowercase();
    let mut terms = Vec::new();
    let mut current = String::new();
    for ch in lowered.chars() {
        if ch.is_alphanumeric() || ch == '_' || ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            current.push(ch);
        } else if current.chars().count() >= 2 {
            terms.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if current.chars().count() >= 2 {
        terms.push(current);
    }
    terms.sort();
    terms.dedup();
    terms
}

fn lexical_score(query: &str, weighted_fields: &[(&str, f32)]) -> f32 {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    let lowered_query = trimmed
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect::<String>()
        .to_lowercase();
    let terms = query_terms(trimmed);
    if terms.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    for (value, weight) in weighted_fields {
        let haystack = value.to_lowercase();
        if haystack.is_empty() {
            continue;
        }
        if haystack.contains(&lowered_query) {
            score += weight * 2.0;
        }
        for term in &terms {
            if haystack.contains(term) {
                score += *weight;
            }
        }
    }
    score
}

fn score_episode_for_query(episode: &MemoryEpisodeRecord, query: &str) -> f32 {
    let actions = episode.actions.join(" ");
    let tags = episode.tags.join(" ");
    lexical_score(
        query,
        &[
            (&episode.title, 3.0),
            (&episode.lesson, 2.5),
            (&episode.situation, 1.6),
            (&episode.outcome, 1.4),
            (&actions, 1.1),
            (&tags, 1.0),
        ],
    )
}

fn score_procedure_for_query(procedure: &MemoryProcedureRecord, query: &str) -> f32 {
    let tags = procedure.tags.join(" ");
    lexical_score(
        query,
        &[
            (&procedure.title, 3.2),
            (&procedure.trigger, 2.4),
            (&procedure.steps_markdown, 1.8),
            (&procedure.constraints_markdown, 1.1),
            (&tags, 1.0),
        ],
    )
}

fn experience_candidate_from_episode(episode: MemoryEpisodeRecord) -> MemoryExperienceCandidate {
    let mut parts = Vec::new();
    if !episode.lesson.trim().is_empty() {
        parts.push(episode.lesson.trim().to_string());
    }
    if !episode.outcome.trim().is_empty() {
        parts.push(episode.outcome.trim().to_string());
    }
    if !episode.situation.trim().is_empty() {
        parts.push(episode.situation.trim().to_string());
    }
    MemoryExperienceCandidate {
        kind: "episode".to_string(),
        id: episode.id,
        scope: episode.scope,
        title: episode.title.clone(),
        preview: preview_experience(&episode.title, &parts.join(" ")),
        score: Some(episode.success_score),
        confidence: None,
    }
}

fn experience_candidate_from_procedure(
    procedure: MemoryProcedureRecord,
) -> MemoryExperienceCandidate {
    let mut parts = Vec::new();
    if !procedure.trigger.trim().is_empty() {
        parts.push(procedure.trigger.trim().to_string());
    }
    if !procedure.steps_markdown.trim().is_empty() {
        parts.push(procedure.steps_markdown.trim().to_string());
    }
    MemoryExperienceCandidate {
        kind: "procedure".to_string(),
        id: procedure.id,
        scope: procedure.scope,
        title: procedure.title.clone(),
        preview: preview_experience(&procedure.title, &parts.join(" ")),
        score: None,
        confidence: Some(procedure.confidence),
    }
}

fn preview_experience(title: &str, body: &str) -> String {
    let mut text = title.trim().to_string();
    let body = body.trim();
    if !body.is_empty() {
        if !text.is_empty() {
            text.push_str(" - ");
        }
        text.push_str(body);
    }
    crate::truncate_utf8(text.trim(), 180).to_string()
}

fn escape_like_pattern(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn query_pattern(query: &Option<String>) -> Option<String> {
    let trimmed = query.as_deref()?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lowered = trimmed
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect::<String>()
        .to_lowercase();
    Some(format!("%{}%", escape_like_pattern(&lowered)))
}

fn push_scope_filter(conditions: &mut Vec<String>, args: &mut Vec<SqlValue>, scope: &MemoryScope) {
    match scope {
        MemoryScope::Global => conditions.push("scope_type = 'global'".to_string()),
        MemoryScope::Agent { id } => {
            conditions.push("scope_type = 'agent' AND scope_id = ?".to_string());
            args.push(SqlValue::Text(id.clone()));
        }
        MemoryScope::Project { id } => {
            conditions.push("scope_type = 'project' AND scope_id = ?".to_string());
            args.push(SqlValue::Text(id.clone()));
        }
    }
}

fn episode_where(query: &MemoryEpisodeQuery) -> (String, Vec<SqlValue>) {
    let mut conditions = Vec::new();
    let mut args = Vec::new();
    if let Some(scope) = query.scope.as_ref() {
        push_scope_filter(&mut conditions, &mut args, scope);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push("status = ?".to_string());
        args.push(SqlValue::Text(status.to_string()));
    }
    if let Some(pattern) = query_pattern(&query.query) {
        conditions.push(
            "(lower(title) LIKE ? ESCAPE '\\'
              OR lower(situation) LIKE ? ESCAPE '\\'
              OR lower(outcome) LIKE ? ESCAPE '\\'
              OR lower(lesson) LIKE ? ESCAPE '\\'
              OR lower(tags_json) LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        for _ in 0..5 {
            args.push(SqlValue::Text(pattern.clone()));
        }
    }
    where_sql(conditions)
        .map(|sql| (sql, args))
        .unwrap_or_default()
}

fn procedure_where(query: &MemoryProcedureQuery) -> (String, Vec<SqlValue>) {
    let mut conditions = Vec::new();
    let mut args = Vec::new();
    if let Some(scope) = query.scope.as_ref() {
        push_scope_filter(&mut conditions, &mut args, scope);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push("status = ?".to_string());
        args.push(SqlValue::Text(status.to_string()));
    }
    if let Some(pattern) = query_pattern(&query.query) {
        conditions.push(
            "(lower(title) LIKE ? ESCAPE '\\'
              OR lower(trigger) LIKE ? ESCAPE '\\'
              OR lower(steps_markdown) LIKE ? ESCAPE '\\'
              OR lower(constraints_markdown) LIKE ? ESCAPE '\\'
              OR lower(tags_json) LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        for _ in 0..5 {
            args.push(SqlValue::Text(pattern.clone()));
        }
    }
    where_sql(conditions)
        .map(|sql| (sql, args))
        .unwrap_or_default()
}

fn experience_history_where(query: &MemoryExperienceHistoryQuery) -> (String, Vec<SqlValue>) {
    let mut conditions = Vec::new();
    let mut args = Vec::new();
    if let Some(scope) = query.scope.as_ref() {
        push_scope_filter(&mut conditions, &mut args, scope);
    }
    if let Some(kind) = query
        .target_kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push("target_kind = ?".to_string());
        args.push(SqlValue::Text(normalize_experience_kind(kind).to_string()));
    }
    if let Some(target_id) = query
        .target_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push("target_id = ?".to_string());
        args.push(SqlValue::Text(target_id.chars().take(128).collect()));
    }
    if let Some(actions) = query.actions.as_ref() {
        let mut normalized = Vec::new();
        for action in actions {
            if let Some(value) = history_action_filter_value(action) {
                if !normalized.contains(&value) {
                    normalized.push(value);
                }
            }
        }
        if !normalized.is_empty() {
            let placeholders = vec!["?"; normalized.len()].join(", ");
            conditions.push(format!("action IN ({placeholders})"));
            for action in normalized {
                args.push(SqlValue::Text(action.to_string()));
            }
        } else if !actions.is_empty() {
            conditions.push("1 = 0".to_string());
        }
    }
    if let Some(pattern) = query_pattern(&query.query) {
        conditions.push(
            "(lower(title_preview) LIKE ? ESCAPE '\\'
              OR lower(content_preview) LIKE ? ESCAPE '\\'
              OR lower(target_id) LIKE ? ESCAPE '\\'
              OR lower(action) LIKE ? ESCAPE '\\'
              OR lower(target_kind) LIKE ? ESCAPE '\\'
              OR lower(scope_id) LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        for _ in 0..6 {
            args.push(SqlValue::Text(pattern.clone()));
        }
    }
    where_sql(conditions)
        .map(|sql| (sql, args))
        .unwrap_or_default()
}

fn episode_order_sql(sort: Option<&str>) -> &'static str {
    match sort.map(str::trim).filter(|s| !s.is_empty()) {
        Some("updated_asc") => "updated_at ASC, created_at ASC, id ASC",
        Some("created_desc") => "created_at DESC, updated_at DESC, id DESC",
        Some("created_asc") => "created_at ASC, updated_at ASC, id ASC",
        Some("title_asc") => "lower(title) ASC, updated_at DESC, id ASC",
        Some("title_desc") => "lower(title) DESC, updated_at DESC, id DESC",
        Some("score_desc" | "quality_desc") => "success_score DESC, updated_at DESC, id DESC",
        Some("score_asc" | "quality_asc") => "success_score ASC, updated_at DESC, id ASC",
        _ => "updated_at DESC, created_at DESC, id DESC",
    }
}

fn procedure_order_sql(sort: Option<&str>) -> &'static str {
    match sort.map(str::trim).filter(|s| !s.is_empty()) {
        Some("updated_asc") => "updated_at ASC, created_at ASC, id ASC",
        Some("created_desc") => "created_at DESC, updated_at DESC, id DESC",
        Some("created_asc") => "created_at ASC, updated_at ASC, id ASC",
        Some("title_asc") => "lower(title) ASC, updated_at DESC, id ASC",
        Some("title_desc") => "lower(title) DESC, updated_at DESC, id DESC",
        Some("confidence_desc" | "quality_desc") => "confidence DESC, updated_at DESC, id DESC",
        Some("confidence_asc" | "quality_asc") => "confidence ASC, updated_at DESC, id ASC",
        _ => "updated_at DESC, created_at DESC, id DESC",
    }
}

fn where_sql(conditions: Vec<String>) -> Option<String> {
    if conditions.is_empty() {
        None
    } else {
        Some(format!("WHERE {}", conditions.join(" AND ")))
    }
}

fn actions_to_markdown(actions: &[String], outcome: &str) -> String {
    let mut lines = Vec::new();
    for (idx, action) in actions.iter().filter(|a| !a.trim().is_empty()).enumerate() {
        lines.push(format!("{}. {}", idx + 1, action.trim()));
    }
    if lines.is_empty() {
        lines.push("1. Recreate the useful steps from the original episode.".to_string());
    }
    if !outcome.trim().is_empty() {
        lines.push(format!("\nExpected outcome: {}", outcome.trim()));
    }
    lines.join("\n")
}

fn episode_history_preview(
    title: &str,
    situation: &str,
    actions: &[String],
    outcome: &str,
    lesson: &str,
) -> String {
    let mut parts = vec![format!("Situation: {}", situation.trim())];
    if !actions.is_empty() {
        parts.push(format!("Actions: {}", actions.join("; ")));
    }
    if !outcome.trim().is_empty() {
        parts.push(format!("Outcome: {}", outcome.trim()));
    }
    if !lesson.trim().is_empty() {
        parts.push(format!("Lesson: {}", lesson.trim()));
    }
    let body = parts.join(" ");
    if title.trim().is_empty() {
        truncate_history_preview(&body)
    } else {
        truncate_history_preview(&format!("{} - {}", title.trim(), body))
    }
}

fn episode_record_history_preview(record: &MemoryEpisodeRecord) -> String {
    episode_history_preview(
        &record.title,
        &record.situation,
        &record.actions,
        &record.outcome,
        &record.lesson,
    )
}

fn procedure_history_preview(
    title: &str,
    trigger: &str,
    steps_markdown: &str,
    constraints_markdown: &str,
    confidence: f32,
) -> String {
    let mut parts = vec![
        format!("Trigger: {}", trigger.trim()),
        format!(
            "Confidence: {}%",
            (confidence.clamp(0.0, 1.0) * 100.0).round()
        ),
        format!("Steps: {}", steps_markdown.trim()),
    ];
    if !constraints_markdown.trim().is_empty() {
        parts.push(format!("Constraints: {}", constraints_markdown.trim()));
    }
    let body = parts.join(" ");
    if title.trim().is_empty() {
        truncate_history_preview(&body)
    } else {
        truncate_history_preview(&format!("{} - {}", title.trim(), body))
    }
}

fn procedure_record_history_preview(record: &MemoryProcedureRecord) -> String {
    procedure_history_preview(
        &record.title,
        &record.trigger,
        &record.steps_markdown,
        &record.constraints_markdown,
        record.confidence,
    )
}

fn row_to_episode(row: &Row) -> rusqlite::Result<MemoryEpisodeRecord> {
    let scope_type: String = row.get(1)?;
    let scope_id: Option<String> = row.get(2)?;
    let actions_json: String = row.get(5)?;
    let source_message_ids_json: String = row.get(9)?;
    let tags_json: String = row.get(11)?;
    Ok(MemoryEpisodeRecord {
        id: row.get(0)?,
        scope: scope_from_parts(scope_type, scope_id),
        title: row.get(3)?,
        situation: row.get(4)?,
        actions: parse_vec_json(&actions_json),
        outcome: row.get(6)?,
        lesson: row.get(7)?,
        source_session_id: row.get(8)?,
        source_message_ids: parse_vec_json(&source_message_ids_json),
        success_score: row.get::<_, f64>(10)? as f32,
        tags: parse_vec_json(&tags_json),
        status: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_procedure(row: &Row) -> rusqlite::Result<MemoryProcedureRecord> {
    let scope_type: String = row.get(1)?;
    let scope_id: Option<String> = row.get(2)?;
    let source_episode_ids_json: String = row.get(9)?;
    let tags_json: String = row.get(10)?;
    Ok(MemoryProcedureRecord {
        id: row.get(0)?,
        scope: scope_from_parts(scope_type, scope_id),
        title: row.get(3)?,
        trigger: row.get(4)?,
        steps_markdown: row.get(5)?,
        constraints_markdown: row.get(6)?,
        confidence: row.get::<_, f64>(7)? as f32,
        status: row.get(8)?,
        source_episode_ids: parse_vec_json(&source_episode_ids_json),
        tags: parse_vec_json(&tags_json),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn row_to_experience_history(row: &Row) -> rusqlite::Result<MemoryExperienceHistoryRecord> {
    let scope_type: String = row.get(4)?;
    let scope_id: Option<String> = row.get(5)?;
    Ok(MemoryExperienceHistoryRecord {
        id: row.get(0)?,
        target_kind: row.get(1)?,
        target_id: row.get(2)?,
        action: row.get(3)?,
        scope: scope_from_parts(scope_type, scope_id),
        title_preview: row.get(6)?,
        content_preview: row.get(7)?,
        created_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> Arc<SqliteMemoryBackend> {
        let dir = std::env::temp_dir().join(format!("ha-episodes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(SqliteMemoryBackend::open(&dir.join("memory.db")).unwrap())
    }

    #[test]
    fn episode_store_adds_lists_and_promotes_procedure() {
        let store = EpisodeStore::new(backend());
        let episode = store
            .add_episode(NewMemoryEpisode {
                scope: MemoryScope::Project {
                    id: "proj-1".to_string(),
                },
                title: "Fixed flaky release".to_string(),
                situation: "Release check failed after package signing".to_string(),
                actions: vec![
                    "Inspected CI logs".to_string(),
                    "Rebuilt package metadata".to_string(),
                    "Re-ran release verify".to_string(),
                ],
                outcome: "Release passed".to_string(),
                lesson: "Check package metadata before retrying CI".to_string(),
                source_session_id: Some("s1".to_string()),
                source_message_ids: vec!["m1".to_string(), "m2".to_string()],
                success_score: Some(0.9),
                tags: vec!["release".to_string(), "ci".to_string()],
            })
            .unwrap();

        let page = store
            .list_episodes_page(&MemoryEpisodeQuery {
                scope: Some(MemoryScope::Project {
                    id: "proj-1".to_string(),
                }),
                query: Some("metadata".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, episode.id);

        let procedure = store
            .promote_episode_to_procedure(&episode.id, PromoteEpisodeOptions::default())
            .unwrap();
        assert_eq!(procedure.scope, episode.scope);
        assert_eq!(procedure.source_episode_ids, vec![episode.id.clone()]);
        assert!(procedure.steps_markdown.contains("Inspected CI logs"));
        assert!(procedure
            .constraints_markdown
            .contains("Check package metadata"));

        let procedures = store
            .list_procedures_page(&MemoryProcedureQuery {
                query: Some("release".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(procedures.total, 1);

        let low_episode = store
            .add_episode(NewMemoryEpisode {
                scope: MemoryScope::Project {
                    id: "proj-1".to_string(),
                },
                title: "Alpha low confidence lesson".to_string(),
                situation: "Release issue looked similar but had a weaker outcome".to_string(),
                actions: vec!["Tried a workaround".to_string()],
                outcome: "Only partially helped".to_string(),
                lesson: "Prefer the metadata fix first".to_string(),
                source_session_id: None,
                source_message_ids: Vec::new(),
                success_score: Some(0.2),
                tags: vec!["release".to_string()],
            })
            .unwrap();
        let quality_sorted = store
            .list_episodes_page(&MemoryEpisodeQuery {
                scope: Some(MemoryScope::Project {
                    id: "proj-1".to_string(),
                }),
                status: Some("active".to_string()),
                sort: Some("quality_desc".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(quality_sorted.items[0].id, episode.id);
        let title_sorted = store
            .list_episodes_page(&MemoryEpisodeQuery {
                scope: Some(MemoryScope::Project {
                    id: "proj-1".to_string(),
                }),
                status: Some("active".to_string()),
                sort: Some("title_asc".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(title_sorted.items[0].id, low_episode.id);

        let low_procedure = store
            .add_procedure(NewMemoryProcedure {
                scope: MemoryScope::Project {
                    id: "proj-1".to_string(),
                },
                title: "Alpha fallback workflow".to_string(),
                trigger: "Weak release signal".to_string(),
                steps_markdown: "- Double check before using".to_string(),
                constraints_markdown: String::new(),
                confidence: Some(0.1),
                source_episode_ids: vec![low_episode.id.clone()],
                tags: vec!["release".to_string()],
            })
            .unwrap();
        let procedure_quality_sorted = store
            .list_procedures_page(&MemoryProcedureQuery {
                scope: Some(MemoryScope::Project {
                    id: "proj-1".to_string(),
                }),
                status: Some("active".to_string()),
                sort: Some("quality_desc".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(procedure_quality_sorted.items[0].id, procedure.id);
        assert!(procedure_quality_sorted
            .items
            .iter()
            .any(|item| item.id == low_procedure.id));

        let updated_episode = store
            .update_episode(
                &low_episode.id,
                MemoryEpisodePatch {
                    scope: Some(MemoryScope::Agent {
                        id: "agent-1".to_string(),
                    }),
                    title: Some("Agent scoped release lesson".to_string()),
                    situation: Some("Agent-specific release issue".to_string()),
                    actions: Some(vec!["Use the agent checklist".to_string()]),
                    outcome: Some("Recovered".to_string()),
                    lesson: Some("Do not reuse this globally".to_string()),
                    success_score: Some(0.7),
                    tags: Some(vec!["agent".to_string(), "release".to_string()]),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            updated_episode.scope,
            MemoryScope::Agent {
                id: "agent-1".to_string()
            }
        );
        assert_eq!(updated_episode.title, "Agent scoped release lesson");
        assert_eq!(updated_episode.actions, vec!["Use the agent checklist"]);
        assert!((updated_episode.success_score - 0.7).abs() < f32::EPSILON);

        let updated_procedure = store
            .update_procedure(
                &low_procedure.id,
                MemoryProcedurePatch {
                    scope: Some(MemoryScope::Global),
                    title: Some("Global fallback workflow".to_string()),
                    trigger: Some("Fallback release signal".to_string()),
                    steps_markdown: Some("- Verify scope\n- Retry narrowly".to_string()),
                    constraints_markdown: Some(
                        "Only when project-specific workflow is absent".to_string(),
                    ),
                    confidence: Some(0.6),
                    source_episode_ids: Some(vec![updated_episode.id.clone()]),
                    tags: Some(vec!["fallback".to_string()]),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated_procedure.scope, MemoryScope::Global);
        assert_eq!(updated_procedure.title, "Global fallback workflow");
        assert!(updated_procedure.steps_markdown.contains("Retry narrowly"));
        assert!((updated_procedure.confidence - 0.6).abs() < f32::EPSILON);
        assert_eq!(
            updated_procedure.source_episode_ids,
            vec![updated_episode.id.clone()]
        );

        let shortlist = store
            .shortlist_experience_candidates(
                "metadata",
                &[MemoryScope::Project {
                    id: "proj-1".to_string(),
                }],
                4,
            )
            .unwrap();
        assert_eq!(shortlist.len(), 2);
        assert!(shortlist.iter().any(|item| item.kind == "procedure"
            && item.id == procedure.id
            && (item.confidence.unwrap_or_default() - 0.9).abs() < f32::EPSILON));
        assert!(shortlist.iter().any(|item| item.kind == "episode"
            && item.id == episode.id
            && (item.score.unwrap_or_default() - 0.9).abs() < f32::EPSILON));

        let natural_shortlist = store
            .shortlist_experience_candidates(
                "help me build a release metadata workflow after package signing",
                &[MemoryScope::Project {
                    id: "proj-1".to_string(),
                }],
                4,
            )
            .unwrap();
        assert!(!natural_shortlist.is_empty());
        assert_eq!(natural_shortlist[0].kind, "procedure");
        assert_eq!(natural_shortlist[0].id, procedure.id);
        assert!(natural_shortlist
            .iter()
            .any(|item| item.kind == "episode" && item.id == episode.id));

        let cross_scope = store
            .shortlist_experience_candidates("metadata", &[MemoryScope::Global], 4)
            .unwrap();
        assert!(
            cross_scope.is_empty(),
            "project episodes must not leak globally"
        );

        let global_metadata_procedure = store
            .add_procedure(NewMemoryProcedure {
                scope: MemoryScope::Global,
                title: "Global metadata workflow".to_string(),
                trigger: "Package metadata release workflow".to_string(),
                steps_markdown: "- Use only when no project workflow exists".to_string(),
                constraints_markdown: "Project workflows take precedence".to_string(),
                confidence: Some(1.0),
                source_episode_ids: Vec::new(),
                tags: vec!["metadata".to_string(), "release".to_string()],
            })
            .unwrap();
        let scoped_priority = store
            .shortlist_experience_candidates(
                "metadata workflow",
                &[
                    MemoryScope::Project {
                        id: "proj-1".to_string(),
                    },
                    MemoryScope::Global,
                ],
                1,
            )
            .unwrap();
        assert_eq!(scoped_priority.len(), 1);
        assert_eq!(
            scoped_priority[0].scope,
            MemoryScope::Project {
                id: "proj-1".to_string(),
            },
            "project-scoped candidates must fill the limited shortlist before global fallback {}",
            global_metadata_procedure.id
        );
        assert_ne!(scoped_priority[0].id, global_metadata_procedure.id);
        let global_only = store
            .shortlist_experience_candidates("metadata workflow", &[MemoryScope::Global], 4)
            .unwrap();
        assert!(global_only
            .iter()
            .any(|item| item.kind == "procedure" && item.id == global_metadata_procedure.id));
        assert!(global_only
            .iter()
            .all(|item| item.scope == MemoryScope::Global));

        assert!(store.archive_episode(&episode.id).unwrap());
        assert_eq!(
            store.get_episode(&episode.id).unwrap().unwrap().status,
            "archived"
        );
        assert!(store.restore_episode(&episode.id).unwrap());
        assert_eq!(
            store.get_episode(&episode.id).unwrap().unwrap().status,
            "active"
        );
        assert!(store.archive_procedure(&procedure.id).unwrap());
        assert_eq!(
            store.get_procedure(&procedure.id).unwrap().unwrap().status,
            "archived"
        );
        let archived_shortlist = store
            .shortlist_experience_candidates(
                "metadata workflow",
                &[MemoryScope::Project {
                    id: "proj-1".to_string(),
                }],
                4,
            )
            .unwrap();
        assert!(
            archived_shortlist
                .iter()
                .all(|item| item.id != procedure.id),
            "archived procedures must not remain retrieval candidates"
        );
        assert!(store.restore_procedure(&procedure.id).unwrap());
        assert_eq!(
            store.get_procedure(&procedure.id).unwrap().unwrap().status,
            "active"
        );

        let episode_history = store
            .list_experience_history_page(&MemoryExperienceHistoryQuery {
                target_kind: Some("episode".to_string()),
                target_id: Some(episode.id.clone()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        let episode_actions = episode_history
            .items
            .iter()
            .map(|item| item.action.as_str())
            .collect::<Vec<_>>();
        assert!(episode_actions.contains(&"add"));
        assert!(episode_actions.contains(&"archive"));
        assert!(episode_actions.contains(&"restore"));
        assert!(episode_history
            .items
            .iter()
            .all(|item| item.target_kind == "episode"));

        let procedure_history = store
            .list_experience_history_page(&MemoryExperienceHistoryQuery {
                target_kind: Some("procedure".to_string()),
                target_id: Some(procedure.id.clone()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        let procedure_actions = procedure_history
            .items
            .iter()
            .map(|item| item.action.as_str())
            .collect::<Vec<_>>();
        assert!(procedure_actions.contains(&"promote"));
        assert!(procedure_actions.contains(&"archive"));
        assert!(procedure_actions.contains(&"restore"));
        assert!(procedure_history
            .items
            .iter()
            .any(|item| item.content_preview.contains("metadata")));

        let expected_project_scope = MemoryScope::Project {
            id: "proj-1".to_string(),
        };
        let filtered_history = store
            .list_experience_history_page(&MemoryExperienceHistoryQuery {
                scope: Some(expected_project_scope.clone()),
                actions: Some(vec!["promote".to_string()]),
                query: Some("metadata".to_string()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert!(
            filtered_history
                .items
                .iter()
                .any(|item| item.target_kind == "procedure"
                    && item.target_id == procedure.id
                    && item.action == "promote"),
            "experience history query/action/scope filter should find promoted procedure"
        );
        assert!(filtered_history.items.iter().all(|item| {
            item.action == "promote"
                && item.scope == expected_project_scope
                && (item.title_preview.to_lowercase().contains("metadata")
                    || item.content_preview.to_lowercase().contains("metadata")
                    || item.target_id.to_lowercase().contains("metadata"))
        }));

        let invalid_action_filter = store
            .list_experience_history_page(&MemoryExperienceHistoryQuery {
                actions: Some(vec!["not-a-real-action".to_string()]),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            invalid_action_filter.total, 0,
            "unknown action filters should fail closed instead of returning all history"
        );

        let restored_episode = MemoryEpisodeRecord {
            id: "restored-episode".to_string(),
            scope: MemoryScope::Global,
            title: "Restored imported episode".to_string(),
            situation: "Backup carried an old workflow lesson".to_string(),
            actions: vec!["Inspect backup".to_string()],
            outcome: "Imported safely".to_string(),
            lesson: "Preview before apply".to_string(),
            source_session_id: None,
            source_message_ids: Vec::new(),
            success_score: 0.8,
            tags: vec!["backup".to_string()],
            status: "active".to_string(),
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        assert!(store.restore_episode_record(&restored_episode).unwrap());
        let restored_history = store
            .list_experience_history_page(&MemoryExperienceHistoryQuery {
                target_kind: Some("episode".to_string()),
                target_id: Some(restored_episode.id.clone()),
                limit: Some(5),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(restored_history.total, 1);
        assert_eq!(restored_history.items[0].action, "restore_import");
    }
}
