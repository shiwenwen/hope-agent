//! Production lifecycle operations for owner-managed Agents.
//!
//! Deletion is a coordinated, recoverable operation rather than a raw
//! `remove_dir_all`: validate ids, inspect active work, rebind durable routing
//! references, preserve historical rows, then atomically move Agent-owned
//! filesystem data into a timestamped trash directory.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::agent_loader::{self, DEFAULT_AGENT_ID};
use crate::cron::CronPayload;

fn lifecycle_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn active_agent_runs() -> &'static Mutex<HashMap<String, usize>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn deleted_agent_ids() -> &'static Mutex<HashSet<String>> {
    static DELETED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DELETED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Admission guard shared by every execution path.
///
/// Registration and deletion both pass through `lifecycle_lock`, so there is
/// no gap where a run has validated an Agent but deletion cannot see it yet.
pub struct AgentRunGuard {
    agent_id: String,
}

impl Drop for AgentRunGuard {
    fn drop(&mut self) {
        let mut active = active_agent_runs()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(count) = active.get_mut(&self.agent_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&self.agent_id);
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReferenceCounts {
    pub global_config: usize,
    pub projects: usize,
    pub cron_jobs: usize,
    pub pending_wakeups: usize,
    pub other_agent_configs: usize,
    pub historical_sessions: usize,
    pub historical_subagent_runs: usize,
    pub historical_teams: usize,
    pub agent_memories: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActiveWorkCounts {
    pub agent_runs: usize,
    pub foreground_sessions: usize,
    pub subagent_runs: usize,
    pub teams: usize,
    pub cron_runs: usize,
    pub background_jobs: usize,
    pub pending_wakeups: usize,
}

impl AgentActiveWorkCounts {
    pub fn total(&self) -> usize {
        self.agent_runs
            + self.foreground_sessions
            + self.subagent_runs
            + self.teams
            + self.cron_runs
            + self.background_jobs
            + self.pending_wakeups
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeletePreview {
    pub agent_id: String,
    pub agent_name: String,
    pub enabled: bool,
    pub is_main: bool,
    pub references: AgentReferenceCounts,
    pub active_work: AgentActiveWorkCounts,
    pub has_home_dir: bool,
    pub has_plan_dir: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeleteRequest {
    pub id: String,
    pub replacement_agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeleteSummary {
    pub agent_id: String,
    pub replacement_agent_id: String,
    pub trash_path: String,
    pub backup_path: Option<String>,
    pub references: AgentReferenceCounts,
    pub retained_agent_memories: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
struct ProjectDefaultSnapshot {
    project_id: String,
    default_agent_id: Option<String>,
}

#[derive(Clone)]
struct CronPayloadSnapshot {
    job_id: String,
    payload_json: String,
    rewritten_payload_json: String,
}

struct DeleteRollbackSnapshot {
    old_agent_id: String,
    replacement_agent_id: String,
    global_config: crate::config::AppConfig,
    agent_configs: Vec<(String, crate::agent_config::AgentConfig)>,
    project_defaults: Vec<ProjectDefaultSnapshot>,
    cron_payloads: Vec<CronPayloadSnapshot>,
    wakeups: Vec<crate::wakeup::Wakeup>,
}

#[derive(Default)]
struct RewriteProgress {
    global_config: bool,
    agent_configs: bool,
    project_defaults: bool,
    cron_payloads: bool,
    wakeups: bool,
}

impl DeleteRollbackSnapshot {
    fn restore(&self, progress: &RewriteProgress) -> Result<()> {
        let mut errors = Vec::new();
        if progress.global_config {
            if let Err(error) =
                crate::config::mutate_config(("agent.delete.rollback", "lifecycle"), |cfg| {
                    restore_global_agent_references(
                        cfg,
                        &self.global_config,
                        &self.old_agent_id,
                        &self.replacement_agent_id,
                    );
                    Ok(())
                })
            {
                errors.push(format!("global config: {error}"));
            }
        }
        if progress.agent_configs {
            for (id, config) in &self.agent_configs {
                if let Err(error) = agent_loader::save_agent_config_unlocked(id, config) {
                    errors.push(format!("Agent config {id}: {error}"));
                }
            }
        }
        if progress.project_defaults {
            if let Err(error) =
                restore_project_defaults(&self.project_defaults, &self.replacement_agent_id)
            {
                errors.push(format!("Project defaults: {error}"));
            }
        }
        if progress.cron_payloads {
            if let Err(error) = restore_cron_payloads(&self.cron_payloads) {
                errors.push(format!("Cron payloads: {error}"));
            }
        }
        if progress.wakeups {
            if let Some(db) = crate::wakeup::get_wakeup_db() {
                match db.restore_reassigned_agent(&self.wakeups, &self.replacement_agent_id) {
                    Ok(()) => crate::wakeup::update_armed_agent(
                        &self.wakeups,
                        &self.replacement_agent_id,
                        &self.old_agent_id,
                    ),
                    Err(error) => errors.push(format!("Wakeups: {error}")),
                }
            } else if !self.wakeups.is_empty() {
                errors.push("Wakeups: database is unavailable".to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("; "))
        }
    }
}

fn restore_global_agent_references(
    current: &mut crate::config::AppConfig,
    original: &crate::config::AppConfig,
    old: &str,
    replacement: &str,
) {
    restore_slot_from_snapshot(
        &mut current.default_agent_id,
        &original.default_agent_id,
        old,
        replacement,
    );
    restore_slot_from_snapshot(
        &mut current.recap.analysis_agent,
        &original.recap.analysis_agent,
        old,
        replacement,
    );
    restore_slot_from_snapshot(
        &mut current.channels.default_agent_id,
        &original.channels.default_agent_id,
        old,
        replacement,
    );

    if let Some(original_index) = original.agent_order.iter().position(|id| id == old) {
        if !current.agent_order.iter().any(|id| id == old) {
            current.agent_order.insert(
                original_index.min(current.agent_order.len()),
                old.to_string(),
            );
        }
    }

    for original_account in &original.channels.accounts {
        let Some(current_account) = current.channels.find_account_mut(&original_account.id) else {
            continue;
        };
        restore_slot_from_snapshot(
            &mut current_account.agent_id,
            &original_account.agent_id,
            old,
            replacement,
        );
        for (group_id, original_group) in &original_account.security.groups {
            let Some(current_group) = current_account.security.groups.get_mut(group_id) else {
                continue;
            };
            restore_slot_from_snapshot(
                &mut current_group.agent_id,
                &original_group.agent_id,
                old,
                replacement,
            );
            for (topic_id, original_topic) in &original_group.topics {
                let Some(current_topic) = current_group.topics.get_mut(topic_id) else {
                    continue;
                };
                restore_slot_from_snapshot(
                    &mut current_topic.agent_id,
                    &original_topic.agent_id,
                    old,
                    replacement,
                );
            }
        }
        for (channel_id, original_channel) in &original_account.security.channels {
            let Some(current_channel) = current_account.security.channels.get_mut(channel_id)
            else {
                continue;
            };
            restore_slot_from_snapshot(
                &mut current_channel.agent_id,
                &original_channel.agent_id,
                old,
                replacement,
            );
        }
    }
}

fn restore_slot_from_snapshot(
    current: &mut Option<String>,
    original: &Option<String>,
    old: &str,
    replacement: &str,
) {
    if original.as_deref() == Some(old) && current.as_deref() == Some(replacement) {
        *current = Some(old.to_string());
    }
}

pub fn ensure_agent_runnable(id: &str) -> Result<()> {
    crate::paths::validate_agent_id(id)?;
    // `ha-main` is the built-in bootstrap identity and can neither be disabled
    // nor deleted. Fresh installs and hermetic engine tests may legitimately
    // run it before onboarding materializes agents/ha-main/agent.json. Once the
    // file exists, still parse it normally so corruption is never hidden.
    if id == DEFAULT_AGENT_ID && !crate::paths::agent_dir(id)?.join("agent.json").is_file() {
        return Ok(());
    }
    let def = agent_loader::load_agent(id)?;
    if !def.config.enabled {
        anyhow::bail!("Agent '{id}' is disabled");
    }
    Ok(())
}

pub(crate) fn save_agent_config(
    id: &str,
    config: &crate::agent_config::AgentConfig,
    explicit_create: bool,
) -> Result<()> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::paths::validate_agent_id(id)?;
    if id == DEFAULT_AGENT_ID && !config.enabled {
        anyhow::bail!("Cannot disable the main agent");
    }
    if !config.enabled && agent_loader::load_agent(id).is_ok_and(|current| current.config.enabled) {
        ensure_agent_can_disable(id)?;
    }

    let mut deleted = deleted_agent_ids()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if explicit_create {
        // Agent identity is established by agent.json. An otherwise orphaned
        // directory may legitimately contain memory/markdown written by an
        // import or recovered from an interrupted older create operation.
        if crate::paths::agent_dir(id)?.join("agent.json").is_file() {
            anyhow::bail!("Agent '{id}' already exists");
        }
        deleted.remove(id);
    } else if deleted.contains(id) {
        anyhow::bail!("Agent '{id}' was deleted; refusing a stale write");
    }
    drop(deleted);

    agent_loader::save_agent_config_unlocked(id, config)
}

pub(crate) fn save_agent_markdown(id: &str, file: &str, content: &str) -> Result<()> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::paths::validate_agent_id(id)?;
    if deleted_agent_ids()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .contains(id)
    {
        anyhow::bail!("Agent '{id}' was deleted; refusing a stale write");
    }
    if !crate::paths::agent_dir(id)?.join("agent.json").is_file() {
        anyhow::bail!("Agent '{id}' does not exist");
    }
    agent_loader::save_agent_markdown_unlocked(id, file, content)
}

pub fn save_agent_memory_md(id: &str, content: &str) -> Result<()> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::paths::validate_agent_id(id)?;
    if deleted_agent_ids()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .contains(id)
    {
        anyhow::bail!("Agent '{id}' was deleted; refusing a stale write");
    }
    let dir = crate::paths::agent_dir(id)?;
    if !dir.join("agent.json").is_file() {
        anyhow::bail!("Agent '{id}' does not exist");
    }
    crate::platform::write_atomic(&dir.join("memory.md"), content.as_bytes())?;
    Ok(())
}

pub fn begin_agent_run(id: &str) -> Result<AgentRunGuard> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    ensure_agent_runnable(id)?;
    let mut active = active_agent_runs()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    *active.entry(id.to_string()).or_default() += 1;
    Ok(AgentRunGuard {
        agent_id: id.to_string(),
    })
}

pub fn set_agent_enabled(id: &str, enabled: bool) -> Result<()> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::paths::validate_agent_id(id)?;
    if id == DEFAULT_AGENT_ID && !enabled {
        anyhow::bail!("Cannot disable the main agent");
    }
    let mut def = agent_loader::load_agent(id)?;
    if !enabled && def.config.enabled {
        ensure_agent_can_disable(id)?;
    }
    def.config.enabled = enabled;
    agent_loader::save_agent_config_unlocked(id, &def.config)
}

fn ensure_agent_can_disable(id: &str) -> Result<()> {
    let references = collect_reference_counts(id)?;
    let blocking = references.global_config
        + references.projects
        + references.cron_jobs
        + references.pending_wakeups;
    if blocking > 0 {
        anyhow::bail!(
            "Agent '{id}' is still used by live routes (global={}, projects={}, cron={}, wakeups={}); reassign them before disabling",
            references.global_config,
            references.projects,
            references.cron_jobs,
            references.pending_wakeups,
        );
    }
    Ok(())
}

pub fn preview_agent_delete(id: &str) -> Result<AgentDeletePreview> {
    crate::paths::validate_agent_id(id)?;
    let def = agent_loader::load_agent(id)?;
    let references = collect_reference_counts(id)?;
    let active_work = collect_active_work(id)?;
    let is_main = id == DEFAULT_AGENT_ID;
    let mut blockers = Vec::new();
    if is_main {
        blockers.push("main_agent".to_string());
    }
    if active_work.total() > 0 {
        blockers.push("active_work".to_string());
    }
    Ok(AgentDeletePreview {
        agent_id: id.to_string(),
        agent_name: def.config.name,
        enabled: def.config.enabled,
        is_main,
        references,
        active_work,
        has_home_dir: crate::paths::agent_home_dir(id)?.exists(),
        has_plan_dir: crate::paths::plans_dir()?.join(id).exists(),
        blockers,
    })
}

pub fn delete_agent(request: &AgentDeleteRequest) -> Result<AgentDeleteSummary> {
    let _guard = lifecycle_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let id = request.id.trim();
    let replacement = request.replacement_agent_id.trim();
    crate::paths::validate_agent_id(id)?;
    crate::paths::validate_agent_id(replacement)?;
    if id == DEFAULT_AGENT_ID {
        anyhow::bail!("Cannot delete the main agent");
    }
    if id == replacement {
        anyhow::bail!("Replacement agent must differ from the deleted agent");
    }
    ensure_agent_runnable(replacement)
        .with_context(|| format!("Replacement agent '{replacement}' is not available"))?;

    // Re-run the preflight while holding the lifecycle lock; a stale UI
    // preview is never trusted as authorization to race active work.
    let preview = preview_agent_delete(id)?;
    if preview.active_work.total() > 0 {
        anyhow::bail!(
            "Agent '{id}' still has active work (runs={}, foreground={}, subagents={}, teams={}, cron={}, background={}, wakeups={})",
            preview.active_work.agent_runs,
            preview.active_work.foreground_sessions,
            preview.active_work.subagent_runs,
            preview.active_work.teams,
            preview.active_work.cron_runs,
            preview.active_work.background_jobs,
            preview.active_work.pending_wakeups,
        );
    }

    // Resolve every fallible path before the lifecycle gate changes. After the
    // primary rename succeeds, deletion is committed and only
    // warning-producing ancillary moves are allowed to run.
    let agent_dir = crate::paths::agent_dir(id)?;
    let agent_home_dir = crate::paths::agent_home_dir(id)?;
    let agent_plans_dir = crate::paths::plans_dir()?.join(id);

    let backup_path = crate::backup::create_backup().map_err(|error| {
        anyhow::anyhow!("Failed to create the mandatory pre-delete backup: {error}")
    })?;
    let backup_root = Path::new(&backup_path);
    verify_backup_file(
        &crate::paths::agent_dir(id)?.join("agent.json"),
        &backup_root.join("agents").join(id).join("agent.json"),
    )?;
    verify_backup_file_if_present(
        &crate::paths::root_dir()?.join("config.json"),
        &backup_root.join("config.json"),
    )?;
    let backup_path = Some(backup_path);

    // Disable first so newly-resolved work stops selecting the Agent while
    // durable references are being rebound. If a later step fails the Agent
    // directory remains present and a retry is safe.
    let mut def = agent_loader::load_agent(id)?;
    let was_enabled = def.config.enabled;
    def.config.enabled = false;
    agent_loader::save_agent_config_unlocked(id, &def.config)?;

    // Close the admission race: work that passed its own runnable check just
    // before the lifecycle gate flipped must become visible before any files
    // move. Leave the Agent intact and re-enable it when that happens.
    let after_disable = collect_active_work(id)?;
    if after_disable.total() > 0 {
        def.config.enabled = was_enabled;
        agent_loader::save_agent_config_unlocked(id, &def.config)?;
        anyhow::bail!("Agent '{id}' became active while deletion was starting; retry when idle");
    }

    // Capture exact pre-mutation values for every durable store. The full
    // backup remains the disaster-recovery layer; this snapshot provides
    // automatic compensation when any ordinary delete step fails.
    let rollback_snapshot = match capture_delete_rollback(id, replacement) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            def.config.enabled = was_enabled;
            agent_loader::save_agent_config_unlocked(id, &def.config)?;
            return Err(error);
        }
    };
    let mut rewrite_progress = RewriteProgress::default();
    let result = (|| -> Result<AgentDeleteSummary> {
        rewrite_progress.global_config = true;
        rewrite_global_config(id, replacement)?;
        // This step writes multiple files and can fail after a partial prefix,
        // so mark it before entering and compensate from the full snapshot.
        rewrite_progress.agent_configs = true;
        rewrite_agent_allowlists(id, replacement)?;
        rewrite_session_db_references(id, replacement)?;
        rewrite_progress.project_defaults = true;
        rewrite_cron_references(id, replacement)?;
        rewrite_progress.cron_payloads = true;
        rewrite_progress.wakeups = true;
        rewrite_wakeup_references(id, replacement)?;

        let trash_root = create_trash_root(id)?;
        let mut summary = AgentDeleteSummary {
            agent_id: id.to_string(),
            replacement_agent_id: replacement.to_string(),
            trash_path: trash_root.to_string_lossy().to_string(),
            backup_path,
            references: preview.references.clone(),
            retained_agent_memories: preview.references.agent_memories,
            warnings: Vec::new(),
        };
        // Write recovery metadata before the primary atomic move. Once the
        // Agent directory moves, the operation is committed from the user's
        // perspective and only ancillary cleanup may degrade.
        let manifest = serde_json::to_vec_pretty(&summary)?;
        crate::platform::write_atomic(&trash_root.join("manifest.json"), &manifest)?;
        deleted_agent_ids()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(id.to_string());
        if let Err(error) = move_to_trash(&agent_dir, &trash_root.join("agent")) {
            deleted_agent_ids()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(id);
            return Err(error);
        }

        for (label, from, to) in [
            ("home", agent_home_dir.clone(), trash_root.join("home")),
            ("plans", agent_plans_dir.clone(), trash_root.join("plans")),
        ] {
            if let Err(error) = move_to_trash(&from, &to) {
                summary
                    .warnings
                    .push(format!("Could not move Agent {label} to trash: {error}"));
            }
        }
        if !summary.warnings.is_empty() {
            if let Ok(manifest) = serde_json::to_vec_pretty(&summary) {
                let _ = crate::platform::write_atomic(&trash_root.join("manifest.json"), &manifest);
            }
        }
        Ok(summary)
    })();

    match result {
        Ok(summary) => Ok(summary),
        Err(error) => {
            let rollback_error = rollback_snapshot.restore(&rewrite_progress).err();
            if let Ok(mut current) = agent_loader::load_agent(id) {
                current.config.enabled = was_enabled;
                let _ = agent_loader::save_agent_config_unlocked(id, &current.config);
            }
            if agent_dir.exists() {
                deleted_agent_ids()
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .remove(id);
            }
            if let Some(rollback_error) = rollback_error {
                anyhow::bail!("{error}; automatic rollback also failed: {rollback_error}");
            }
            Err(error)
        }
    }
}

fn capture_delete_rollback(id: &str, replacement: &str) -> Result<DeleteRollbackSnapshot> {
    let mut agent_configs = Vec::new();
    for agent_id in agent_loader::list_agent_ids()? {
        if agent_id == id {
            continue;
        }
        let Ok(def) = agent_loader::load_agent(&agent_id) else {
            continue;
        };
        if def
            .config
            .subagents
            .allowed_agents
            .iter()
            .chain(def.config.subagents.denied_agents.iter())
            .any(|value| value == id)
        {
            agent_configs.push((agent_id, def.config));
        }
    }

    let mut project_defaults = Vec::new();
    if let Some(db) = crate::get_session_db() {
        let conn = db.conn.lock().unwrap_or_else(|poison| poison.into_inner());
        if table_exists(&conn, "projects") {
            let mut stmt = conn
                .prepare("SELECT id, default_agent_id FROM projects WHERE default_agent_id=?1")?;
            project_defaults = stmt
                .query_map(params![id], |row| {
                    Ok(ProjectDefaultSnapshot {
                        project_id: row.get(0)?,
                        default_agent_id: row.get(1)?,
                    })
                })?
                .collect::<std::result::Result<_, _>>()?;
        }
    }

    let mut cron_payloads = Vec::new();
    if let Some(db) = crate::get_cron_db() {
        let conn = db.conn.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut stmt = conn.prepare("SELECT id, payload_json FROM cron_jobs")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (job_id, payload_json) = row?;
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&payload_json) else {
                continue;
            };
            if replace_agent_id_fields(&mut value, id, replacement) {
                cron_payloads.push(CronPayloadSnapshot {
                    job_id,
                    payload_json,
                    rewritten_payload_json: serde_json::to_string(&value)?,
                });
            }
        }
    }

    let wakeups = crate::wakeup::get_wakeup_db()
        .map(|db| {
            db.list_pending().map(|rows| {
                rows.into_iter()
                    .filter(|row| row.agent_id == id)
                    .collect::<Vec<_>>()
            })
        })
        .transpose()?
        .unwrap_or_default();

    Ok(DeleteRollbackSnapshot {
        old_agent_id: id.to_string(),
        replacement_agent_id: replacement.to_string(),
        global_config: crate::config::cached_config().as_ref().clone(),
        agent_configs,
        project_defaults,
        cron_payloads,
        wakeups,
    })
}

fn restore_project_defaults(snapshots: &[ProjectDefaultSnapshot], replacement: &str) -> Result<()> {
    if snapshots.is_empty() {
        return Ok(());
    }
    let Some(db) = crate::get_session_db() else {
        anyhow::bail!("Session DB is unavailable");
    };
    let mut conn = db.conn.lock().unwrap_or_else(|poison| poison.into_inner());
    let tx = conn.transaction()?;
    for snapshot in snapshots {
        tx.execute(
            "UPDATE projects SET default_agent_id=?1 WHERE id=?2 AND default_agent_id=?3",
            params![snapshot.default_agent_id, snapshot.project_id, replacement],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn restore_cron_payloads(snapshots: &[CronPayloadSnapshot]) -> Result<()> {
    if snapshots.is_empty() {
        return Ok(());
    }
    let Some(db) = crate::get_cron_db() else {
        anyhow::bail!("Cron DB is unavailable");
    };
    let mut conn = db.conn.lock().unwrap_or_else(|poison| poison.into_inner());
    let tx = conn.transaction()?;
    for snapshot in snapshots {
        tx.execute(
            "UPDATE cron_jobs SET payload_json=?1 WHERE id=?2 AND payload_json=?3",
            params![
                snapshot.payload_json,
                snapshot.job_id,
                snapshot.rewritten_payload_json
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn collect_reference_counts(id: &str) -> Result<AgentReferenceCounts> {
    let cfg = crate::config::cached_config();
    let mut global_config = usize::from(cfg.default_agent_id.as_deref() == Some(id))
        + usize::from(cfg.recap.analysis_agent.as_deref() == Some(id))
        + usize::from(cfg.channels.default_agent_id.as_deref() == Some(id));
    for account in &cfg.channels.accounts {
        global_config += usize::from(account.agent_id.as_deref() == Some(id));
        for group in account.security.groups.values() {
            global_config += usize::from(group.agent_id.as_deref() == Some(id));
            for topic in group.topics.values() {
                global_config += usize::from(topic.agent_id.as_deref() == Some(id));
            }
        }
        for channel in account.security.channels.values() {
            global_config += usize::from(channel.agent_id.as_deref() == Some(id));
        }
    }

    let mut counts = AgentReferenceCounts {
        global_config,
        ..Default::default()
    };
    if let Some(db) = crate::get_session_db() {
        let conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());
        counts.projects = count_column(&conn, "projects", "default_agent_id", id)?;
        counts.historical_sessions = count_column(&conn, "sessions", "agent_id", id)?;
        counts.historical_subagent_runs = count_two_columns(
            &conn,
            "subagent_runs",
            "parent_agent_id",
            "child_agent_id",
            id,
        )?;
        counts.historical_teams = count_two_tables_for_agent(&conn, id)?;
    }
    if let Some(db) = crate::get_cron_db() {
        counts.cron_jobs = db
            .list_jobs()?
            .iter()
            .filter(|job| payload_references_agent(&job.payload, id))
            .count();
    }
    counts.pending_wakeups = crate::wakeup::count_pending_for_agent(id)?;
    counts.other_agent_configs = count_allowlist_references(id)?;
    counts.agent_memories = crate::get_memory_backend()
        .and_then(|backend| {
            backend
                .count(Some(&crate::memory::MemoryScope::Agent { id: id.into() }))
                .ok()
        })
        .unwrap_or(0);
    Ok(counts)
}

fn collect_active_work(id: &str) -> Result<AgentActiveWorkCounts> {
    let mut counts = AgentActiveWorkCounts {
        agent_runs: active_agent_runs()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(id)
            .copied()
            .unwrap_or(0),
        ..Default::default()
    };
    if let Some(db) = crate::get_session_db() {
        counts.subagent_runs = db.count_nonterminal_subagent_runs_for_agent(id)?;
        counts.teams = db.count_active_teams_involving_agent(id)?;
        let active_session_ids: Vec<String> = crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect();
        for session_id in active_session_ids {
            if db
                .get_session(&session_id)?
                .is_some_and(|session| session.agent_id == id)
            {
                counts.foreground_sessions += 1;
            }
        }
    }
    if let Some(db) = crate::get_cron_db() {
        counts.cron_runs = db
            .list_jobs()?
            .iter()
            .filter(|job| job.running_at.is_some() && payload_references_agent(&job.payload, id))
            .count();
    }
    if let Some(db) = crate::async_jobs::get_async_jobs_db() {
        counts.background_jobs = db
            .list_running()?
            .iter()
            .filter(|job| job.agent_id.as_deref() == Some(id))
            .count();
    }
    counts.pending_wakeups = crate::wakeup::count_unpersisted_for_agent(id);
    Ok(counts)
}

fn rewrite_global_config(old: &str, replacement: &str) -> Result<()> {
    crate::config::mutate_config(("agent.delete", "lifecycle"), |cfg| {
        replace_slot(&mut cfg.default_agent_id, old, replacement);
        replace_slot(&mut cfg.recap.analysis_agent, old, replacement);
        replace_slot(&mut cfg.channels.default_agent_id, old, replacement);
        cfg.agent_order.retain(|id| id != old);
        for account in &mut cfg.channels.accounts {
            replace_slot(&mut account.agent_id, old, replacement);
            for group in account.security.groups.values_mut() {
                replace_slot(&mut group.agent_id, old, replacement);
                for topic in group.topics.values_mut() {
                    replace_slot(&mut topic.agent_id, old, replacement);
                }
            }
            for channel in account.security.channels.values_mut() {
                replace_slot(&mut channel.agent_id, old, replacement);
            }
        }
        Ok(())
    })
}

fn rewrite_agent_allowlists(old: &str, replacement: &str) -> Result<()> {
    let ids = agent_loader::list_agent_ids()?;
    for id in ids {
        if id == old {
            continue;
        }
        let Ok(mut def) = agent_loader::load_agent(&id) else {
            continue;
        };
        let changed = rebind_subagent_config(&mut def.config.subagents, old, replacement);
        if changed {
            agent_loader::save_agent_config_unlocked(&id, &def.config)?;
        }
    }
    Ok(())
}

fn rebind_subagent_config(
    config: &mut crate::agent_config::SubagentConfig,
    old: &str,
    replacement: &str,
) -> bool {
    let rebinding_allowed_child = config.allowed_agents.iter().any(|value| value == old);
    let mut changed = replace_vec_value(&mut config.allowed_agents, old, Some(replacement));
    changed |= replace_vec_value(&mut config.denied_agents, old, None);
    // Deny wins over allow at execution time. If the deleted child was
    // explicitly allowed, the replacement must not remain explicitly denied
    // or the durable rebind would be internally contradictory.
    if rebinding_allowed_child {
        changed |= replace_vec_value(&mut config.denied_agents, replacement, None);
    }
    changed
}

fn rewrite_session_db_references(old: &str, replacement: &str) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        return Ok(());
    };
    let mut conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let tx = conn.transaction()?;
    if table_exists(&tx, "projects") {
        tx.execute(
            "UPDATE projects SET default_agent_id=?1 WHERE default_agent_id=?2",
            params![replacement, old],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn rewrite_cron_references(old: &str, replacement: &str) -> Result<()> {
    let Some(db) = crate::get_cron_db() else {
        return Ok(());
    };
    let mut conn = db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let tx = conn.transaction()?;
    let mut stmt = tx.prepare("SELECT id, payload_json FROM cron_jobs")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);
    for (id, payload) in rows {
        let mut value: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if replace_agent_id_fields(&mut value, old, replacement) {
            tx.execute(
                "UPDATE cron_jobs SET payload_json=?1, updated_at=?2 WHERE id=?3",
                params![
                    serde_json::to_string(&value)?,
                    chrono::Utc::now().to_rfc3339(),
                    id
                ],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn rewrite_wakeup_references(old: &str, replacement: &str) -> Result<()> {
    let Some(db) = crate::wakeup::get_wakeup_db() else {
        return Ok(());
    };
    let rows = db.reassign_pending_agent(old, replacement)?;
    crate::wakeup::update_armed_agent(&rows, old, replacement);
    Ok(())
}

fn create_trash_root(id: &str) -> Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = crate::paths::root_dir()?
        .join("trash")
        .join("agents")
        .join(format!("{id}-{stamp}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn move_to_trash(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
        .with_context(|| format!("move {} to trash at {}", from.display(), to.display()))
}

fn verify_backup_file(source: &Path, backup: &Path) -> Result<()> {
    let source_bytes = std::fs::read(source)
        .with_context(|| format!("read pre-delete source {}", source.display()))?;
    let backup_bytes = std::fs::read(backup)
        .with_context(|| format!("verify pre-delete backup {}", backup.display()))?;
    if source_bytes != backup_bytes {
        anyhow::bail!(
            "Pre-delete backup verification failed for {}",
            source.display()
        );
    }
    Ok(())
}

fn verify_backup_file_if_present(source: &Path, backup: &Path) -> Result<()> {
    if source.exists() {
        verify_backup_file(source, backup)?;
    }
    Ok(())
}

fn replace_slot(slot: &mut Option<String>, old: &str, replacement: &str) {
    if slot.as_deref() == Some(old) {
        *slot = Some(replacement.to_string());
    }
}

fn replace_vec_value(values: &mut Vec<String>, old: &str, replacement: Option<&str>) -> bool {
    if !values.iter().any(|value| value == old) {
        return false;
    }
    values.retain(|value| value != old);
    if let Some(replacement) = replacement {
        if !values.iter().any(|value| value == replacement) {
            values.push(replacement.to_string());
        }
    }
    true
}

fn replace_agent_id_fields(value: &mut serde_json::Value, old: &str, replacement: &str) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if (key == "agent_id" || key == "agentId") && value.as_str() == Some(old) {
                    *value = serde_json::Value::String(replacement.to_string());
                    changed = true;
                } else {
                    changed |= replace_agent_id_fields(value, old, replacement);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                changed |= replace_agent_id_fields(item, old, replacement);
            }
        }
        _ => {}
    }
    changed
}

fn payload_references_agent(payload: &CronPayload, id: &str) -> bool {
    match payload {
        CronPayload::AgentTurn { agent_id, .. } | CronPayload::SessionLoop { agent_id, .. } => {
            agent_id.as_deref() == Some(id)
        }
    }
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn count_column(conn: &Connection, table: &str, column: &str, id: &str) -> Result<usize> {
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1");
    let count: i64 = conn.query_row(&sql, params![id], |row| row.get(0))?;
    Ok(count as usize)
}

fn count_two_columns(
    conn: &Connection,
    table: &str,
    first: &str,
    second: &str,
    id: &str,
) -> Result<usize> {
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {first}=?1 OR {second}=?1");
    let count: i64 = conn.query_row(&sql, params![id], |row| row.get(0))?;
    Ok(count as usize)
}

fn count_two_tables_for_agent(conn: &Connection, id: &str) -> Result<usize> {
    Ok(count_column(conn, "teams", "lead_agent_id", id)?
        + count_column(conn, "team_members", "agent_id", id)?)
}

fn count_allowlist_references(id: &str) -> Result<usize> {
    let mut count = 0;
    for agent_id in agent_loader::list_agent_ids()? {
        if agent_id == id {
            continue;
        }
        let Ok(def) = agent_loader::load_agent(&agent_id) else {
            continue;
        };
        count += def
            .config
            .subagents
            .allowed_agents
            .iter()
            .filter(|value| value.as_str() == id)
            .count();
        count += def
            .config
            .subagents
            .denied_agents
            .iter()
            .filter(|value| value.as_str() == id)
            .count();
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_rewrite_handles_both_serde_spellings() {
        let mut value = serde_json::json!({
            "agent_id": "old",
            "nested": [{"agentId": "old"}, {"agent_id": "keep"}]
        });
        assert!(replace_agent_id_fields(&mut value, "old", "new"));
        assert_eq!(value["agent_id"], "new");
        assert_eq!(value["nested"][0]["agentId"], "new");
        assert_eq!(value["nested"][1]["agent_id"], "keep");
    }

    #[test]
    fn restrictive_allowlist_stays_restrictive_after_rebind() {
        let mut values = vec!["old".to_string()];
        assert!(replace_vec_value(&mut values, "old", Some("new")));
        assert_eq!(values, vec!["new"]);
    }

    #[test]
    fn allowed_child_rebind_removes_replacement_from_denylist() {
        let mut config = crate::agent_config::SubagentConfig {
            allowed_agents: vec!["old".to_string()],
            denied_agents: vec!["new".to_string(), "keep-denied".to_string()],
            ..Default::default()
        };

        assert!(rebind_subagent_config(&mut config, "old", "new"));
        assert_eq!(config.allowed_agents, vec!["new"]);
        assert_eq!(config.denied_agents, vec!["keep-denied"]);
    }

    #[test]
    fn routing_rollback_preserves_unrelated_concurrent_config_changes() {
        let mut original = crate::config::AppConfig {
            default_agent_id: Some("old".into()),
            agent_order: vec!["ha-main".into(), "old".into()],
            ..Default::default()
        };
        original.recap.analysis_agent = Some("old".into());
        let mut current = original.clone();
        current.default_agent_id = Some("new".into());
        current.recap.analysis_agent = Some("new".into());
        current.agent_order.retain(|id| id != "old");
        current.temperature = Some(1.25);

        restore_global_agent_references(&mut current, &original, "old", "new");

        assert_eq!(current.default_agent_id.as_deref(), Some("old"));
        assert_eq!(current.recap.analysis_agent.as_deref(), Some("old"));
        assert_eq!(current.agent_order, vec!["ha-main", "old"]);
        assert_eq!(current.temperature, Some(1.25));
    }

    #[test]
    fn disabled_agents_stay_owner_visible_but_leave_runtime_discovery() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let active = crate::agent_config::AgentConfig {
                name: "Active".into(),
                ..Default::default()
            };
            let disabled = crate::agent_config::AgentConfig {
                enabled: false,
                name: "Disabled".into(),
                ..Default::default()
            };
            agent_loader::save_agent_config("active", &active).unwrap();
            agent_loader::save_agent_config("disabled", &disabled).unwrap();

            let runtime_ids: Vec<_> = agent_loader::list_agents()
                .unwrap()
                .into_iter()
                .map(|agent| agent.id)
                .collect();
            let owner_ids: Vec<_> = agent_loader::list_all_agents()
                .unwrap()
                .into_iter()
                .map(|agent| agent.id)
                .collect();

            assert_eq!(runtime_ids, vec!["active"]);
            assert_eq!(owner_ids, vec!["active", "disabled"]);
            assert!(ensure_agent_runnable("disabled").is_err());
        });
    }

    #[test]
    fn run_admission_is_visible_until_guard_drops() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let id = format!("admission-{}", uuid::Uuid::new_v4());
            agent_loader::save_agent_config(&id, &crate::agent_config::AgentConfig::default())
                .unwrap();

            let guard = begin_agent_run(&id).unwrap();
            assert_eq!(collect_active_work(&id).unwrap().agent_runs, 1);
            drop(guard);
            assert_eq!(collect_active_work(&id).unwrap().agent_runs, 0);
        });
    }

    #[test]
    fn full_config_save_cannot_disable_main_agent() {
        let config = crate::agent_config::AgentConfig {
            enabled: false,
            ..Default::default()
        };
        let error = agent_loader::save_agent_config(DEFAULT_AGENT_ID, &config).unwrap_err();
        assert!(error.to_string().contains("Cannot disable the main agent"));
    }

    #[test]
    fn built_in_main_is_runnable_before_config_materialization() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            assert!(!crate::paths::agent_dir(DEFAULT_AGENT_ID)
                .unwrap()
                .join("agent.json")
                .exists());
            ensure_agent_runnable(DEFAULT_AGENT_ID).unwrap();
        });
    }

    #[test]
    fn deletion_tombstone_rejects_stale_write_but_explicit_create_clears_it() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let id = format!("deleted-{}", uuid::Uuid::new_v4());
            let config = crate::agent_config::AgentConfig::default();
            agent_loader::create_agent_config(&id, &config).unwrap();
            std::fs::remove_dir_all(crate::paths::agent_dir(&id).unwrap()).unwrap();
            deleted_agent_ids()
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .insert(id.clone());

            assert!(agent_loader::save_agent_config(&id, &config).is_err());
            agent_loader::create_agent_config(&id, &config).unwrap();
            assert!(crate::paths::agent_dir(&id)
                .unwrap()
                .join("agent.json")
                .is_file());
        });
    }
}
