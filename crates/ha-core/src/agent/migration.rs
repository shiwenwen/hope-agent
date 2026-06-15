//! One-shot startup migration: rename the legacy default agent id
//! `"default"` → [`crate::agent_loader::DEFAULT_AGENT_ID`] (currently
//! `"ha-main"`) everywhere it's stored.
//!
//! Touches:
//! - on-disk dirs: `agents/default/`, `default-home/`, `plans/default/`
//! - `agents/*/agent.json`: `subagents.allowedAgents` / `deniedAgents`
//!   array entries (per-agent delegation allow/deny lists)
//! - `sessions.db` (also hosts `projects` / channel tables): `sessions`,
//!   `team_members`, `teams.lead_agent_id`, `subagent_runs.parent_agent_id`,
//!   `subagent_runs.child_agent_id`, `projects.default_agent_id`
//! - `cron.db`: `cron_jobs.payload_json` (rewrites the embedded `agent_id`
//!   inside each `AgentTurn` payload)
//! - `logs.db`: `logs.agent_id`
//! - `memory.db`: `memories.scope_agent_id` for `scope_type='agent'` rows
//! - `background_jobs.db` (best-effort, only when the file already exists)
//! - `canvas/canvas.db` (best-effort, only when the file already exists)
//! - global config (`config.json`): `default_agent_id`,
//!   `recap.analysis_agent`, `channels.default_agent_id`,
//!   `channels.accounts[*].agent_id`, plus per-account
//!   `security.groups[*].agent_id` / `groups[*].topics[*].agent_id` /
//!   `channels[*].agent_id`.
//!
//! Idempotent: a sentinel (`<root>/.agent-id-renamed`) records completion
//! so subsequent startups short-circuit; each step is also independently
//! idempotent (WHERE clauses become no-ops after the first run, dir
//! renames check existence first), so a crash mid-migration leaves the
//! next run able to resume. **The sentinel is only written when the disk
//! rename completes cleanly** — when both `default/` and `ha-main/` exist
//! (manual user setup), the migration aborts without touching DBs / config
//! and re-prompts on next startup.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::globals::{CRON_DB, LOG_DB, SESSION_DB};
use crate::paths;

/// Legacy hard-coded agent id we are migrating away from.
const OLD_DEFAULT_ID: &str = "default";

/// Run the full migration. Safe to call on every startup — re-runs are a
/// no-op once the sentinel is present. Pulls live DB handles from the
/// `globals::*` registries (`SESSION_DB` / `CRON_DB` / `LOG_DB` are
/// initialised earlier in `init_runtime`); opens fresh connections for
/// `background_jobs.db` / `canvas/canvas.db` / `memory.db`, which are either
/// lazy-initialised elsewhere (canvas has no global registry) or not yet
/// stored at this point in the runtime lifecycle (the background jobs DB is
/// set later); for memory we'd need to reach through an `Arc<dyn
/// MemoryBackend>` trait object which doesn't expose the raw connection.
///
/// **Order contract**: callers MUST run this before `ensure_default_agent()`
/// (which would otherwise pre-create an empty `agents/<DEFAULT_AGENT_ID>/`
/// template, making the rename refuse and orphan the user's legacy data).
/// Both desktop (`src-tauri/src/lib.rs`) and server (`src-tauri/src/main.rs`)
/// run `init_runtime` before `ensure_default_agent` for this reason.
pub fn migrate_default_agent_id_to_ha_main() -> Result<()> {
    let sentinel = paths::root_dir()?.join(".agent-id-renamed");
    if sentinel.exists() {
        return Ok(());
    }

    // Defensive: tests / dev environments that override the constant to the
    // legacy literal would otherwise keep rewriting their own data.
    if DEFAULT_AGENT_ID == OLD_DEFAULT_ID {
        return Ok(());
    }

    // Disk dir rename is the only step that can hit a true blocking conflict
    // (both legacy and new dir present, with possibly distinct user data).
    // When that happens, abort everything — DB / config rewrite would orphan
    // the legacy `agents/default/` data behind a sentinel that says "done".
    if !rename_disk_dirs()? {
        app_warn!(
            "agent",
            "migration",
            "aborting migration: directory conflict between legacy 'default' \
             and new '{}' agent dirs. Resolve manually (merge or delete one); \
             rerun the app to retry.",
            DEFAULT_AGENT_ID
        );
        return Ok(());
    }

    if let Some(db) = SESSION_DB.get() {
        update_session_db(db)?;
    }
    if let Some(db) = CRON_DB.get() {
        update_cron_db(db)?;
    }
    if let Some(db) = LOG_DB.get() {
        update_log_db(db)?;
    }
    update_external_db_if_present(
        "background_jobs.db",
        paths::background_jobs_db_path()?,
        "background_jobs",
    )?;
    update_external_db_if_present("canvas.db", paths::canvas_db_path()?, "canvas_projects")?;
    update_memory_db_if_present()?;

    update_agent_configs()?;
    update_config_in_place()?;

    std::fs::write(&sentinel, b"")
        .with_context(|| format!("write migration sentinel {}", sentinel.display()))?;
    app_info!(
        "agent",
        "migration",
        "renamed default agent id '{}' → '{}' (sentinel: {})",
        OLD_DEFAULT_ID,
        DEFAULT_AGENT_ID,
        sentinel.display()
    );
    Ok(())
}

/// Rename the three legacy disk dirs. Returns `false` when any rename had to
/// be skipped because both the legacy and the new path are populated — caller
/// must abort the rest of the migration in that case so DB / config don't
/// silently re-point sessions to a fresh-template `ha-main` while the user's
/// real data sits in `default/`.
fn rename_disk_dirs() -> Result<bool> {
    let mut all_clear = true;
    all_clear &= rename_dir_if_present(
        &paths::agent_dir(OLD_DEFAULT_ID)?,
        &paths::agent_dir(DEFAULT_AGENT_ID)?,
    )?;
    all_clear &= rename_dir_if_present(
        &paths::agent_home_dir(OLD_DEFAULT_ID)?,
        &paths::agent_home_dir(DEFAULT_AGENT_ID)?,
    )?;
    let plans = paths::plans_dir()?;
    all_clear &= rename_dir_if_present(&plans.join(OLD_DEFAULT_ID), &plans.join(DEFAULT_AGENT_ID))?;
    Ok(all_clear)
}

/// Rename `from` → `to`. Returns:
/// - `Ok(true)` if rename succeeded or `from` was absent (nothing to do).
/// - `Ok(false)` if both `from` and `to` exist (refused — caller decides
///   whether to abort).
fn rename_dir_if_present(from: &Path, to: &Path) -> Result<bool> {
    if !from.exists() {
        return Ok(true);
    }
    if to.exists() {
        app_warn!(
            "agent",
            "migration",
            "skipping rename: both {} and {} exist; resolve manually",
            from.display(),
            to.display()
        );
        return Ok(false);
    }
    if let Some(parent) = to.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::rename(from, to)
        .with_context(|| format!("rename {} → {}", from.display(), to.display()))?;
    app_info!(
        "agent",
        "migration",
        "renamed {} → {}",
        from.display(),
        to.display()
    );
    Ok(true)
}

fn update_session_db(session_db: &crate::session::SessionDB) -> Result<()> {
    let conn = session_db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let mut total: usize = 0;
    total += update_table(&conn, "sessions", "agent_id")?;
    total += update_table(&conn, "team_members", "agent_id")?;
    total += update_table(&conn, "teams", "lead_agent_id")?;
    total += update_table(&conn, "subagent_runs", "parent_agent_id")?;
    total += update_table(&conn, "subagent_runs", "child_agent_id")?;
    total += update_table(&conn, "projects", "default_agent_id")?;
    log_rewrite("sessions.db", total);
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn update_table(conn: &Connection, table: &str, column: &str) -> Result<usize> {
    // Table may not exist yet on a fresh DB that's never seen the relevant
    // feature (e.g. `projects` if the user has never created one through
    // older builds). Treat "no such table" as zero rows.
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let sql = format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2");
    Ok(conn.execute(&sql, params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID])?)
}

fn update_cron_db(cron_db: &crate::cron::CronDB) -> Result<()> {
    let conn = cron_db.conn.lock().unwrap_or_else(|p| p.into_inner());

    // Read jobs whose payload mentions the legacy id, decode → mutate →
    // re-encode in Rust. SQLite's json_set would be cleaner but we'd have
    // to rely on the JSON1 module being present in every shipped sqlite.
    let mut stmt = conn
        .prepare("SELECT id, payload_json FROM cron_jobs WHERE payload_json LIKE '%\"agent_id%'")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);

    let mut rewritten: usize = 0;
    for (id, payload) in rows {
        let mut value: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if rewrite_agent_id_in_value(&mut value) {
            let new_payload = serde_json::to_string(&value)?;
            conn.execute(
                "UPDATE cron_jobs SET payload_json = ?1 WHERE id = ?2",
                params![new_payload, id],
            )?;
            rewritten += 1;
        }
    }
    if rewritten > 0 {
        app_info!(
            "agent",
            "migration",
            "cron.db: rewrote {} payload(s) agent_id '{}' → '{}'",
            rewritten,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
    Ok(())
}

/// Walk a JSON value, rewriting any `"agent_id": "default"` field to the
/// new id. Returns true if anything changed. Recursion handles nested
/// payload variants (e.g. future cron payload kinds that wrap the agent
/// id deeper).
fn rewrite_agent_id_in_value(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "agent_id" {
                    if let Some(s) = v.as_str() {
                        if s == OLD_DEFAULT_ID {
                            *v = serde_json::Value::String(DEFAULT_AGENT_ID.to_string());
                            changed = true;
                            continue;
                        }
                    }
                }
                if rewrite_agent_id_in_value(v) {
                    changed = true;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                if rewrite_agent_id_in_value(item) {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    changed
}

fn update_log_db(log_db: &crate::logging::LogDB) -> Result<()> {
    let conn = log_db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let n = conn.execute(
        "UPDATE logs SET agent_id = ?1 WHERE agent_id = ?2",
        params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID],
    )?;
    log_rewrite("logs.db", n);
    Ok(())
}

/// Rewrite `agent_id` in the only column of an external SQLite file (one
/// that's not opened eagerly during `init_runtime`). Skips when the file
/// or table doesn't exist; fresh `Connection::open` is intentional —
/// `async_jobs` is set into its global only later in init, and `canvas`
/// has no global registry at all (lazy `CanvasDB::open` per call).
fn update_external_db_if_present(label: &str, path: PathBuf, table: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let conn =
        Connection::open(&path).with_context(|| format!("open {} at {}", label, path.display()))?;
    if !table_exists(&conn, table) {
        return Ok(());
    }
    let sql = format!("UPDATE {table} SET agent_id = ?1 WHERE agent_id = ?2");
    let n = conn.execute(&sql, params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID])?;
    log_rewrite(label, n);
    Ok(())
}

fn log_rewrite(label: &str, n: usize) {
    if n > 0 {
        app_info!(
            "agent",
            "migration",
            "{}: rewrote {} row(s) agent_id '{}' → '{}'",
            label,
            n,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
}

/// Memory DB needs a dedicated helper because its schema diverges from the
/// other agent_id-bearing tables: the column is `scope_agent_id` and the
/// row is only relevant when `scope_type = 'agent'` (project-scoped rows
/// share the schema but use `scope_project_id`).
fn update_memory_db_if_present() -> Result<()> {
    let path = paths::memory_db_path()?;
    if !path.exists() {
        return Ok(());
    }
    let conn =
        Connection::open(&path).with_context(|| format!("open memory db at {}", path.display()))?;
    if !table_exists(&conn, "memories") {
        return Ok(());
    }
    let n = conn.execute(
        "UPDATE memories SET scope_agent_id = ?1 \
         WHERE scope_type = 'agent' AND scope_agent_id = ?2",
        params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID],
    )?;
    log_rewrite("memory.db", n);
    Ok(())
}

/// Walk every `agents/<id>/agent.json` and rewrite legacy `"default"`
/// occurrences inside `subagents.allowedAgents` / `subagents.deniedAgents`.
/// These are user-authored allow/deny lists where the literal `"default"`
/// referenced the main agent — left unrewritten, post-rename delegation
/// would silently match a non-existent id.
fn update_agent_configs() -> Result<()> {
    let agents_dir = paths::agents_dir()?;
    let entries = match std::fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let mut rewritten_files: usize = 0;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let json_path = entry.path().join("agent.json");
        if !json_path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&json_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if rewrite_subagent_allowlists_in_value(&mut value) {
            // Pretty-write to match `save_agent_config` so a subsequent open in
            // the agent editor doesn't reformat the whole file.
            let new_content = serde_json::to_string_pretty(&value)?;
            std::fs::write(&json_path, new_content)
                .with_context(|| format!("write migrated agent.json at {}", json_path.display()))?;
            rewritten_files += 1;
        }
    }
    if rewritten_files > 0 {
        app_info!(
            "agent",
            "migration",
            "agents/*/agent.json: rewrote subagents allow/deny in {} file(s) '{}' → '{}'",
            rewritten_files,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
    Ok(())
}

/// Rewrite `subagents.allowedAgents` / `subagents.deniedAgents` array entries
/// whose value equals the legacy `"default"`. Conservative — touches only
/// these two specific paths, not any other `Vec<String>` in the JSON, so
/// fields that legitimately store `"default"` (theme, etc.) are untouched.
fn rewrite_subagent_allowlists_in_value(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    let Some(subagents) = value.get_mut("subagents").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    for key in ["allowedAgents", "deniedAgents"] {
        let Some(arr) = subagents.get_mut(key).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        for item in arr.iter_mut() {
            if item.as_str() == Some(OLD_DEFAULT_ID) {
                *item = serde_json::Value::String(DEFAULT_AGENT_ID.to_string());
                changed = true;
            }
        }
    }
    changed
}

/// Helper: rewrite `*slot` to the new id when it equals the legacy literal,
/// flipping `*changed` so the caller can decide whether to log.
fn rewrite_legacy_id(slot: &mut Option<String>, changed: &mut bool) {
    if slot.as_deref() == Some(OLD_DEFAULT_ID) {
        *slot = Some(DEFAULT_AGENT_ID.to_string());
        *changed = true;
    }
}

fn update_config_in_place() -> Result<()> {
    crate::config::mutate_config(("agent.id_rename", "migration"), |cfg| {
        let mut changed = false;
        rewrite_legacy_id(&mut cfg.default_agent_id, &mut changed);
        rewrite_legacy_id(&mut cfg.recap.analysis_agent, &mut changed);
        rewrite_legacy_id(&mut cfg.channels.default_agent_id, &mut changed);
        for account in cfg.channels.accounts.iter_mut() {
            rewrite_legacy_id(&mut account.agent_id, &mut changed);
            for group in account.security.groups.values_mut() {
                rewrite_legacy_id(&mut group.agent_id, &mut changed);
                for topic in group.topics.values_mut() {
                    rewrite_legacy_id(&mut topic.agent_id, &mut changed);
                }
            }
            for channel in account.security.channels.values_mut() {
                rewrite_legacy_id(&mut channel.agent_id, &mut changed);
            }
        }
        if changed {
            app_info!(
                "agent",
                "migration",
                "config.json: rewrote agent_id '{}' → '{}'",
                OLD_DEFAULT_ID,
                DEFAULT_AGENT_ID
            );
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_agent_id_handles_nested_objects_and_arrays() {
        let mut value = serde_json::json!({
            "type": "agentTurn",
            "agent_id": OLD_DEFAULT_ID,
            "nested": {
                "agent_id": OLD_DEFAULT_ID,
                "other": 1
            },
            "siblings": [
                { "agent_id": OLD_DEFAULT_ID },
                { "agent_id": "keep-me" }
            ]
        });
        assert!(rewrite_agent_id_in_value(&mut value));
        assert_eq!(value["agent_id"], DEFAULT_AGENT_ID);
        assert_eq!(value["nested"]["agent_id"], DEFAULT_AGENT_ID);
        assert_eq!(value["siblings"][0]["agent_id"], DEFAULT_AGENT_ID);
        assert_eq!(value["siblings"][1]["agent_id"], "keep-me");
    }

    #[test]
    fn rewrite_agent_id_no_op_when_field_missing_or_unrelated() {
        let mut a = serde_json::json!({ "type": "agentTurn", "prompt": "hi" });
        assert!(!rewrite_agent_id_in_value(&mut a));
        let mut b = serde_json::json!({ "agent_id": "coder" });
        assert!(!rewrite_agent_id_in_value(&mut b));
    }

    #[test]
    fn rewrite_legacy_id_only_replaces_old_default() {
        let mut changed = false;
        let mut slot: Option<String> = Some(OLD_DEFAULT_ID.to_string());
        rewrite_legacy_id(&mut slot, &mut changed);
        assert_eq!(slot.as_deref(), Some(DEFAULT_AGENT_ID));
        assert!(changed);

        // None / unrelated / already-renamed are all no-ops.
        for keep in [
            None,
            Some("coder".to_string()),
            Some(DEFAULT_AGENT_ID.to_string()),
        ] {
            let mut c = false;
            let mut s = keep.clone();
            rewrite_legacy_id(&mut s, &mut c);
            assert_eq!(s, keep);
            assert!(!c);
        }
    }

    #[test]
    fn rewrite_subagent_allowlists_replaces_in_both_lists() {
        let mut value = serde_json::json!({
            "name": "Hope",
            "subagents": {
                "allowedAgents": [OLD_DEFAULT_ID, "coder"],
                "deniedAgents": [OLD_DEFAULT_ID],
                "maxConcurrent": 3,
            },
        });
        assert!(rewrite_subagent_allowlists_in_value(&mut value));
        assert_eq!(
            value["subagents"]["allowedAgents"],
            serde_json::json!([DEFAULT_AGENT_ID, "coder"])
        );
        assert_eq!(
            value["subagents"]["deniedAgents"],
            serde_json::json!([DEFAULT_AGENT_ID])
        );
        // Unrelated fields untouched.
        assert_eq!(value["subagents"]["maxConcurrent"], 3);
        assert_eq!(value["name"], "Hope");
    }

    #[test]
    fn rewrite_subagent_allowlists_no_op_when_subagents_missing_or_clean() {
        // `subagents` block missing entirely.
        let mut a = serde_json::json!({ "name": "Hope" });
        assert!(!rewrite_subagent_allowlists_in_value(&mut a));

        // Lists present but no legacy id inside.
        let mut b = serde_json::json!({
            "subagents": {
                "allowedAgents": ["coder", "researcher"],
                "deniedAgents": [],
            },
        });
        assert!(!rewrite_subagent_allowlists_in_value(&mut b));
    }

    #[test]
    fn rewrite_subagent_allowlists_only_touches_allow_deny_paths() {
        // A field that legitimately holds the literal "default" elsewhere
        // in agent.json (e.g. a personality preset id) must NOT be rewritten.
        // Conservative — we only walk `subagents.{allowed,denied}Agents`.
        let mut value = serde_json::json!({
            "personality": { "preset": OLD_DEFAULT_ID },
            "subagents": { "allowedAgents": [] },
        });
        assert!(!rewrite_subagent_allowlists_in_value(&mut value));
        assert_eq!(value["personality"]["preset"], OLD_DEFAULT_ID);
    }
}
