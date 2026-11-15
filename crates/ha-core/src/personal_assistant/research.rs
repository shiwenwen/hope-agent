//! Versioned owner-shared local files. Bodies never enter Home history/receipts.
use crate::session::SessionDB;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

mod budget;
mod queries;
mod revocation;
mod runs;
mod runtime;
mod sources;
pub use budget::reserve_model_attempt;
pub(crate) use budget::{reserve_operation, settle_operation};
pub(crate) use queries::{input_set_current, validate_input_set};
pub use queries::{ResearchActivity, ResearchQueryReceipt};
pub(crate) use revocation::ensure_channel_guards;
pub(crate) use runtime::SYSTEM_PROMPT;
pub use runtime::{coordinator_registered, process_pending_research, register_coordinator};
pub(crate) use sources::refresh_execution_sources;
pub use sources::{create_source, refresh_owner_sources, scan_sources};

pub const SOURCE_BYTES: usize = 65_536;
pub const MAX_SOURCES: i64 = 4;
pub const SCAN_SECONDS: i64 = 1_800;
pub const REVIEW_SECONDS: i64 = 7_200;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantSourceInput {
    pub request_key: String,
    pub session_id: String,
    pub relative_path: String,
    pub focus: String,
    pub duration_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSource {
    pub id: String,
    pub session_id: String,
    pub relative_path: String,
    pub focus: String,
    pub revision: i64,
    pub version: Option<String>,
    pub state: String,
    pub expires_at: i64,
    pub last_observed_at: Option<i64>,
    pub gap_since: Option<i64>,
    pub error: bool,
    pub suspended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceReference {
    pub source_id: String,
    pub revision: i64,
    pub version: String,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchFindingInput {
    pub topic_key: String,
    pub fact: String,
    pub inference: String,
    pub uncertainty: String,
    pub prepared: String,
    pub references: Vec<SourceReference>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantFinding {
    pub id: String,
    pub revision: i64,
    pub fact: String,
    pub inference: String,
    pub uncertainty: String,
    pub prepared: String,
    pub references: Vec<SourceReference>,
    pub created_at: String,
    pub viewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResearchInput {
    pub source_id: String,
    pub revision: i64,
    pub version: String,
    pub focus: String,
    pub text: String,
    pub previous_text: Option<String>,
    pub previous_version: Option<String>,
}

/// No deserializer/public constructor. Authority is reloaded at admission.
#[derive(Clone)]
pub(crate) struct ResearchProof {
    pub(crate) run_id: String,
}

pub struct PendingResearch {
    pub run_id: String,
    pub session_id: String,
}

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS assistant_sources(
        id TEXT PRIMARY KEY,
        home_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        relative_path TEXT NOT NULL,
        workspace_key TEXT NOT NULL,
        focus TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1,
        state TEXT NOT NULL CHECK(state IN('active','revoked')),
        expires_at INTEGER NOT NULL,
        version TEXT,
        body TEXT,
        previous_body TEXT,
        previous_version TEXT,
        global_stop_epoch INTEGER NOT NULL,
        home_stop_epoch INTEGER NOT NULL,
        next_scan_at INTEGER NOT NULL,
        next_review_at INTEGER NOT NULL,
        last_observed_at INTEGER,
        gap_since INTEGER,
        error INTEGER NOT NULL DEFAULT 0,
        UNIQUE(session_id,relative_path)
    );
    CREATE TABLE IF NOT EXISTS assistant_source_grants(
        request_key TEXT PRIMARY KEY,
        spec_hash TEXT NOT NULL,
        source_id TEXT NOT NULL,
        source_revision INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS assistant_source_gaps(
        source_id TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        ended_at INTEGER,
        PRIMARY KEY(source_id,started_at)
    );
    CREATE TABLE IF NOT EXISTS assistant_scope_changes(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        revision INTEGER NOT NULL,
        invalidation_revision INTEGER NOT NULL DEFAULT 0
    );
    INSERT OR IGNORE INTO assistant_scope_changes VALUES(1,0,0);
    CREATE TABLE IF NOT EXISTS assistant_source_events(
        id TEXT PRIMARY KEY,
        source_id TEXT NOT NULL REFERENCES assistant_sources(id) ON DELETE CASCADE,
        revision INTEGER NOT NULL,
        version TEXT NOT NULL,
        kind TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        run_id TEXT
    );
    CREATE TABLE IF NOT EXISTS assistant_research_runs(
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL UNIQUE,
        home_session_id TEXT NOT NULL,
        execution_id TEXT UNIQUE,
        kind TEXT NOT NULL DEFAULT 'discovery',
        query_id TEXT,
        state TEXT NOT NULL,
        instance_id TEXT NOT NULL,
        policy_revision INTEGER NOT NULL,
        global_stop_epoch INTEGER NOT NULL,
        home_stop_epoch INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        ended_at INTEGER,
        question TEXT,
        prior_finding TEXT,
        outcome_reason TEXT,
        reported_json TEXT,
        models_used INTEGER NOT NULL DEFAULT 0,
        tools_used INTEGER NOT NULL DEFAULT 0,
        active_seconds INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS assistant_research_inputs(
        run_id TEXT NOT NULL REFERENCES assistant_research_runs(id) ON DELETE CASCADE,
        source_id TEXT NOT NULL,
        revision INTEGER NOT NULL,
        version TEXT NOT NULL,
        focus TEXT NOT NULL,
        body TEXT,
        previous_body TEXT,
        previous_version TEXT,
        PRIMARY KEY(run_id,source_id)
    );
    CREATE TABLE IF NOT EXISTS assistant_research_usage(
        id TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        attempted_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS assistant_research_usage_time ON assistant_research_usage(attempted_at);
    CREATE TABLE IF NOT EXISTS assistant_research_reservations(
        token TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        execution_id TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        reserved_seconds INTEGER NOT NULL,
        charged_seconds INTEGER NOT NULL DEFAULT 0,
        settled_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS assistant_findings(
        id TEXT PRIMARY KEY,
        semantic_key TEXT NOT NULL UNIQUE,
        run_id TEXT NOT NULL REFERENCES assistant_research_runs(id) ON DELETE CASCADE,
        revision INTEGER NOT NULL,
        content_hash TEXT NOT NULL,
        body_json TEXT NOT NULL,
        state TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        viewed_at TEXT
    );
    CREATE VIEW IF NOT EXISTS assistant_visible_messages AS
        SELECT m.* FROM messages m WHERE NOT EXISTS(SELECT 1 FROM sessions private WHERE private.id=m.session_id AND private.kind='personal_assistant_research') AND NOT EXISTS(
            SELECT 1 FROM assistant_research_runs r WHERE r.session_id=m.session_id);
    CREATE TRIGGER IF NOT EXISTS assistant_research_fixed_scope
        BEFORE UPDATE OF kind,incognito,project_id,parent_session_id,forked_from_session_id ON sessions
        WHEN OLD.kind='personal_assistant_research' AND (
            NEW.kind<>'personal_assistant_research' OR NEW.incognito<>0 OR NEW.project_id IS NOT NULL
            OR NEW.parent_session_id IS NOT NULL OR NEW.forked_from_session_id IS NOT NULL)
        BEGIN SELECT RAISE(ABORT,'research context scope is fixed'); END;
    CREATE TRIGGER IF NOT EXISTS assistant_research_no_fork_insert BEFORE INSERT ON sessions
        WHEN EXISTS(SELECT 1 FROM assistant_research_runs r WHERE r.session_id IN(NEW.parent_session_id,NEW.forked_from_session_id))
        BEGIN SELECT RAISE(ABORT,'research context cannot be forked'); END;
    CREATE TRIGGER IF NOT EXISTS assistant_research_no_fork_update BEFORE UPDATE OF parent_session_id,forked_from_session_id ON sessions
        WHEN EXISTS(SELECT 1 FROM assistant_research_runs r WHERE r.session_id IN(NEW.parent_session_id,NEW.forked_from_session_id))
        BEGIN SELECT RAISE(ABORT,'research context cannot be forked'); END;
    CREATE TRIGGER IF NOT EXISTS assistant_research_memory_insert BEFORE INSERT ON session_memory_policy
        WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id=NEW.session_id AND s.kind='personal_assistant_research')
        AND (NEW.use_memories<>'deny' OR NEW.contribute_to_memories<>'deny')
        BEGIN SELECT RAISE(ABORT,'research memory is disabled'); END;
    CREATE TRIGGER IF NOT EXISTS assistant_research_memory_update BEFORE UPDATE ON session_memory_policy
        WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id=NEW.session_id AND s.kind='personal_assistant_research')
        AND (NEW.use_memories<>'deny' OR NEW.contribute_to_memories<>'deny')
        BEGIN SELECT RAISE(ABORT,'research memory is disabled'); END;
    CREATE TRIGGER IF NOT EXISTS assistant_research_memory_delete BEFORE DELETE ON session_memory_policy
        WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id=OLD.session_id AND s.kind='personal_assistant_research')
        BEGIN SELECT RAISE(ABORT,'research memory is disabled'); END;")?;
    queries::migrate_queries(conn)?;
    revocation::migrate_revocation(conn)?;
    Ok(())
}

pub(crate) fn instance_id() -> &'static str {
    static INSTANCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

pub(crate) fn run_current(conn: &Connection, run: &str, now: i64) -> Result<bool> {
    let live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_runs r JOIN sessions s ON s.id=r.session_id
        JOIN sessions home ON home.id=r.home_session_id AND home.kind='personal_assistant' AND home.incognito=0
        WHERE r.id=?1 AND r.state IN('pending','running','reported') AND r.instance_id=?2 AND s.kind='personal_assistant_research'
        AND s.incognito=0 AND r.global_stop_epoch=(SELECT epoch FROM runtime_control_epochs WHERE key='global_stop')
        AND r.home_stop_epoch=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=home.id)
        AND NOT EXISTS(SELECT 1 FROM session_autonomy_pauses WHERE session_id=home.id AND resumed_at IS NULL)
        AND EXISTS(SELECT 1 FROM assistant_policy_gate gate WHERE gate.singleton=1 AND gate.failed=0
            AND gate.requested_revision=gate.applied_revision AND r.policy_revision=gate.applied_revision
            AND json_extract(gate.applied_json,'$.paused')=0
            AND (r.kind='owner_query' OR json_extract(gate.applied_json,'$.researchEnabled')=1)))",
        params![run,instance_id()],|r|r.get(0))?;
    if !live {
        return Ok(false);
    };
    input_set_current(conn, run, now)
}

pub(crate) fn validate_run(conn: &Connection, run: &str, now: i64) -> Result<()> {
    if !run_current(conn, run, now)? {
        bail!("Research authority or source version is no longer current")
    }
    Ok(())
}

pub(crate) fn validate_execution(
    conn: &Connection,
    execution: &str,
    now: i64,
) -> Result<Option<String>> {
    let run: Option<String> = conn
        .query_row(
            "SELECT id FROM assistant_research_runs WHERE execution_id=?1",
            [execution],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(run) = &run {
        validate_run(conn, run, now)?;
    }
    Ok(run)
}

fn source_current(
    conn: &Connection,
    source: &str,
    revision: i64,
    version: &str,
    now: i64,
) -> Result<bool> {
    let session:Option<String>=conn.query_row("SELECT src.session_id FROM assistant_sources src JOIN sessions home
        ON home.id=src.home_session_id AND home.kind='personal_assistant' AND home.incognito=0
        WHERE src.id=?1 AND src.revision=?2 AND src.version=?3 AND src.state='active' AND src.expires_at>?4 AND src.error=0",
        params![source,revision,version,now],|r|r.get(0)).optional()?;
    let Some(session) = session else {
        return Ok(false);
    };
    scope_available(super::validate_target(conn, &session))
}

fn scope_available(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.downcast_ref::<super::AssistantError>().is_some() => Ok(false),
        Err(error) => Err(error),
    }
}

fn source_autonomy_current(conn: &Connection, source: &str, now: i64) -> Result<bool> {
    let session:Option<String>=conn.query_row("SELECT src.session_id FROM assistant_sources src JOIN sessions home
        ON home.id=src.home_session_id AND home.kind='personal_assistant' AND home.incognito=0
        WHERE src.id=?1 AND src.state='active' AND src.expires_at>?2
        AND src.global_stop_epoch=(SELECT epoch FROM runtime_control_epochs WHERE key='global_stop')
        AND src.home_stop_epoch=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=src.home_session_id)
        AND NOT EXISTS(SELECT 1 FROM session_autonomy_pauses WHERE session_id=src.home_session_id AND resumed_at IS NULL)
        AND EXISTS(SELECT 1 FROM assistant_policy_gate WHERE singleton=1 AND json_extract(requested_json,'$.paused')=0 AND json_extract(requested_json,'$.researchEnabled')=1)",
        params![source,now],|r|r.get(0)).optional()?;
    let Some(session) = session else {
        return Ok(false);
    };
    scope_available(super::validate_target(conn, &session))
}

pub(super) fn changed(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE assistant_scope_changes SET revision=revision+1 WHERE singleton=1",
        [],
    )?;
    Ok(())
}

fn invalidate_changed(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE assistant_scope_changes SET revision=revision+1,invalidation_revision=invalidation_revision+1 WHERE singleton=1",[])?;
    Ok(())
}

pub fn emit_research_hint(invalidate: bool) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "personal_assistant:research_changed",
            serde_json::json!({"invalidate":invalidate}),
        );
    }
}

pub(crate) fn quarantine_source(conn: &Connection, source: &str, now: i64) -> Result<()> {
    invalidate_changed(conn)?;
    conn.execute("UPDATE assistant_research_runs SET state='invalidated',ended_at=?2,reported_json=NULL,question=NULL,prior_finding=NULL,outcome_reason='source_changed' WHERE id IN(
        SELECT run_id FROM assistant_research_inputs WHERE source_id=?1)",params![source,now])?;
    conn.execute("UPDATE sessions SET context_json=NULL,context_revision=context_revision+1,context_run_id=NULL WHERE id IN(
        SELECT r.session_id FROM assistant_research_runs r JOIN assistant_research_inputs i ON i.run_id=r.id WHERE i.source_id=?1)",[source])?;
    conn.execute(
        "UPDATE assistant_research_inputs SET body=NULL,previous_body=NULL WHERE run_id IN(
        SELECT run_id FROM assistant_research_inputs WHERE source_id=?1)",
        [source],
    )?;
    conn.execute(
        "UPDATE assistant_findings SET state='invalidated',body_json='{}' WHERE run_id IN(
        SELECT run_id FROM assistant_research_inputs WHERE source_id=?1)",
        [source],
    )?;
    conn.execute("UPDATE assistant_research_queries SET state='cancelled',reason='source_changed',question='',prior_finding=NULL WHERE
        EXISTS(SELECT 1 FROM json_each(source_set_json) ref WHERE json_extract(ref.value,'$.sourceId')=?1)", [source])?;
    Ok(())
}

impl SessionDB {
    pub(crate) fn assistant_research_context(&self, session: &str) -> Result<bool> {
        let conn = self.read_conn()?;
        is_research_context(&conn, session)
    }
}

pub(crate) fn is_research_context(conn: &Connection, session: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_research_runs WHERE session_id=?1)
        OR EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND kind='personal_assistant_research')",
        [session],
        |r| r.get(0),
    )?)
}

pub(crate) fn check_context_write(
    conn: &Connection,
    session: &str,
    execution: Option<&str>,
) -> Result<()> {
    if !is_research_context(conn, session)? {
        return Ok(());
    }
    let execution =
        execution.ok_or_else(|| anyhow!("A sealed current research execution is required"))?;
    let run = validate_execution(conn, execution, chrono::Utc::now().timestamp())?
        .ok_or_else(|| anyhow!("Research execution identity is missing"))?;
    let same: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_research_runs WHERE id=?1 AND session_id=?2)",
        params![run, session],
        |r| r.get(0),
    )?;
    if !same {
        bail!("Research execution does not own this context")
    }
    Ok(())
}

pub(crate) fn read_shared_file(
    db: &SessionDB,
    session: &str,
    relative: &str,
    expected_key: Option<&str>,
) -> Result<(String, String)> {
    let path = std::path::Path::new(relative);
    if path.is_absolute()
        || relative.is_empty()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Choose a relative file inside the selected workspace");
    }
    {
        let conn = db.read_conn()?;
        sources::eligible(&conn, session)?;
    }
    let meta = db
        .get_session(session)?
        .ok_or_else(|| anyhow!("Shared workspace is unavailable"))?;
    let root = crate::session::effective_working_dir_for_meta(&meta)
        .ok_or_else(|| anyhow!("Choose a workspace before sharing a source"))?;
    let scope = crate::filesystem::WorkspaceScope::from_resolved_root(root)?;
    let workspace_key = blake3::hash(scope.root().to_string_lossy().as_bytes())
        .to_hex()
        .to_string();
    if expected_key.is_some_and(|expected| expected != workspace_key) {
        bail!(sources::WorkspaceChanged)
    }
    scope.resolve_existing(relative)?;
    let mut reader =
        crate::platform::open_file_beneath(scope.root(), std::path::Path::new(relative))?;
    if !reader.metadata()?.is_file() {
        bail!("The selected source must be a regular file");
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > SOURCE_BYTES {
        bail!("Shared sources are limited to 64 KiB of UTF-8 text");
    }
    let text = String::from_utf8(bytes)?;
    if text.contains('\0') {
        bail!("The selected source is not a text file");
    }
    Ok((workspace_key, text))
}

/// A deny fence based on durable kernel identity, never a caller's tool scope.
pub(crate) fn guard_execution(
    conn: &Connection,
    execution: Option<&str>,
    session: Option<&str>,
) -> Result<bool> {
    let run = execution
        .map(|id| {
            conn.query_row(
                "SELECT id,session_id,state FROM assistant_research_runs WHERE execution_id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
        })
        .transpose()?
        .flatten();
    if let Some((run, owner, state)) = run {
        if session != Some(owner.as_str()) || state != "running" {
            bail!("A current sealed research context is required")
        }
        validate_run(conn, &run, chrono::Utc::now().timestamp())?;
        return Ok(true);
    }
    if let Some(session) = session {
        if is_research_context(conn, session)? {
            bail!("Private research requires its exact execution identity")
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
