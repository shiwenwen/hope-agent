//! One-shot startup migration: rename the legacy default agent id
//! `"default"` → [`crate::agent_loader::DEFAULT_AGENT_ID`] (currently
//! `"ha-main"`) everywhere it's stored.
//!
//! Touches:
//! - on-disk dirs: `agents/default/`, `default-home/`, `plans/default/`
//! - `sessions.db` (also hosts `projects` / channel tables): `sessions`,
//!   `team_members`, `teams.lead_agent_id`, `subagent_runs.parent_agent_id`,
//!   `subagent_runs.child_agent_id`, `projects.default_agent_id`
//! - `cron.db`: `cron_jobs.payload_json` (rewrites the embedded `agent_id`
//!   inside each `AgentTurn` payload)
//! - `logs.db`: `logs.agent_id`
//! - `async_jobs.db` (best-effort, only when the file already exists)
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
//! next run able to resume.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::cron::CronDB;
use crate::logging::LogDB;
use crate::paths;
use crate::session::SessionDB;

/// Legacy hard-coded agent id we are migrating away from.
const OLD_DEFAULT_ID: &str = "default";

/// Run the full migration. Safe to call on every startup — re-runs are a
/// no-op once the sentinel is present.
pub fn migrate_default_agent_id_to_ha_main(
    session_db: &SessionDB,
    cron_db: &CronDB,
    log_db: &LogDB,
) -> Result<()> {
    // Defensive: the migration only makes sense when the new id differs
    // from the legacy literal. Tests / dev environments that override the
    // constant would otherwise keep rewriting their own data.
    if DEFAULT_AGENT_ID == OLD_DEFAULT_ID {
        return Ok(());
    }

    let sentinel = paths::root_dir()?.join(".agent-id-renamed");
    if sentinel.exists() {
        return Ok(());
    }

    rename_disk_dirs()?;

    update_session_db(session_db)?;
    update_cron_db(cron_db)?;
    update_log_db(log_db)?;
    update_async_jobs_db_if_present()?;
    update_canvas_db_if_present()?;

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

fn rename_disk_dirs() -> Result<()> {
    rename_dir_if_present(
        &paths::agent_dir(OLD_DEFAULT_ID)?,
        &paths::agent_dir(DEFAULT_AGENT_ID)?,
    )?;
    rename_dir_if_present(
        &paths::agent_home_dir(OLD_DEFAULT_ID)?,
        &paths::agent_home_dir(DEFAULT_AGENT_ID)?,
    )?;
    let plans = paths::plans_dir()?;
    rename_dir_if_present(&plans.join(OLD_DEFAULT_ID), &plans.join(DEFAULT_AGENT_ID))?;
    Ok(())
}

fn rename_dir_if_present(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        return Ok(());
    }
    if to.exists() {
        // User somehow has both — refuse to overwrite their `ha-main` data.
        // Surface a warning so they know to merge / drop one side manually;
        // SQL + config rewrites still proceed, since they're non-destructive.
        app_warn!(
            "agent",
            "migration",
            "skipping rename: both {} and {} exist; resolve manually",
            from.display(),
            to.display()
        );
        return Ok(());
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
    Ok(())
}

fn update_session_db(session_db: &SessionDB) -> Result<()> {
    let conn = session_db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let mut total: usize = 0;
    total += update_table(&conn, "sessions", "agent_id")?;
    total += update_table(&conn, "team_members", "agent_id")?;
    total += update_table(&conn, "teams", "lead_agent_id")?;
    total += update_table(&conn, "subagent_runs", "parent_agent_id")?;
    total += update_table(&conn, "subagent_runs", "child_agent_id")?;
    total += update_table(&conn, "projects", "default_agent_id")?;
    if total > 0 {
        app_info!(
            "agent",
            "migration",
            "sessions.db: rewrote {} row(s) agent_id '{}' → '{}'",
            total,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
    Ok(())
}

fn update_table(conn: &Connection, table: &str, column: &str) -> Result<usize> {
    // Table may not exist yet on a fresh DB that's never seen the relevant
    // feature (e.g. `projects` if the user has never created one through
    // older builds). Treat "no such table" as zero rows.
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .is_ok();
    if !exists {
        return Ok(0);
    }
    let sql = format!("UPDATE {table} SET {column} = ?1 WHERE {column} = ?2");
    Ok(conn.execute(&sql, params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID])?)
}

fn update_cron_db(cron_db: &CronDB) -> Result<()> {
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

fn update_log_db(log_db: &LogDB) -> Result<()> {
    let conn = log_db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let n = conn.execute(
        "UPDATE logs SET agent_id = ?1 WHERE agent_id = ?2",
        params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID],
    )?;
    if n > 0 {
        app_info!(
            "agent",
            "migration",
            "logs.db: rewrote {} row(s) agent_id '{}' → '{}'",
            n,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
    Ok(())
}

fn update_async_jobs_db_if_present() -> Result<()> {
    let path = paths::async_jobs_db_path()?;
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("open async_jobs db at {}", path.display()))?;
    if !table_exists(&conn, "async_tool_jobs") {
        return Ok(());
    }
    let n = conn.execute(
        "UPDATE async_tool_jobs SET agent_id = ?1 WHERE agent_id = ?2",
        params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID],
    )?;
    if n > 0 {
        app_info!(
            "agent",
            "migration",
            "async_jobs.db: rewrote {} row(s) agent_id '{}' → '{}'",
            n,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
    Ok(())
}

fn update_canvas_db_if_present() -> Result<()> {
    let path = paths::canvas_db_path()?;
    if !path.exists() {
        return Ok(());
    }
    let conn =
        Connection::open(&path).with_context(|| format!("open canvas db at {}", path.display()))?;
    if !table_exists(&conn, "canvas_projects") {
        return Ok(());
    }
    let n = conn.execute(
        "UPDATE canvas_projects SET agent_id = ?1 WHERE agent_id = ?2",
        params![DEFAULT_AGENT_ID, OLD_DEFAULT_ID],
    )?;
    if n > 0 {
        app_info!(
            "agent",
            "migration",
            "canvas.db: rewrote {} row(s) agent_id '{}' → '{}'",
            n,
            OLD_DEFAULT_ID,
            DEFAULT_AGENT_ID
        );
    }
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

fn update_config_in_place() -> Result<()> {
    crate::config::mutate_config(("agent.id_rename", "migration"), |cfg| {
        let mut changed = false;

        if cfg.default_agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
            cfg.default_agent_id = Some(DEFAULT_AGENT_ID.to_string());
            changed = true;
        }
        if cfg.recap.analysis_agent.as_deref() == Some(OLD_DEFAULT_ID) {
            cfg.recap.analysis_agent = Some(DEFAULT_AGENT_ID.to_string());
            changed = true;
        }
        if cfg.channels.default_agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
            cfg.channels.default_agent_id = Some(DEFAULT_AGENT_ID.to_string());
            changed = true;
        }
        for account in cfg.channels.accounts.iter_mut() {
            if account.agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
                account.agent_id = Some(DEFAULT_AGENT_ID.to_string());
                changed = true;
            }
            for group in account.security.groups.values_mut() {
                if group.agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
                    group.agent_id = Some(DEFAULT_AGENT_ID.to_string());
                    changed = true;
                }
                for topic in group.topics.values_mut() {
                    if topic.agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
                        topic.agent_id = Some(DEFAULT_AGENT_ID.to_string());
                        changed = true;
                    }
                }
            }
            for channel in account.security.channels.values_mut() {
                if channel.agent_id.as_deref() == Some(OLD_DEFAULT_ID) {
                    channel.agent_id = Some(DEFAULT_AGENT_ID.to_string());
                    changed = true;
                }
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
}
