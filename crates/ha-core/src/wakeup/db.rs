//! SQLite persistence for agent self-scheduled wakeups (R10).
//!
//! A wakeup is a one-shot timer the agent arms via `schedule_wakeup`: at
//! `fire_at` a `<wakeup>` message is injected back into the originating session
//! (reusing the shared injection pipeline) so the agent runs a fresh turn to
//! continue work. This table is the **durable** backing so unfired wakeups
//! survive a restart; the live timers themselves are process-local (see
//! `mod.rs`). Incognito wakeups are never persisted here (close-and-burn).
//!
//! Like `background_jobs.db`, this is a rebuildable/transient cache: project policy
//! is "no migration — drop and rebuild" on a stale schema.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// One scheduled wakeup row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wakeup {
    pub id: String,
    pub session_id: String,
    pub agent_id: String,
    pub note: Option<String>,
    /// Unix seconds (UTC) at which the wakeup should fire.
    pub fire_at: i64,
    pub created_at: i64,
}

pub struct WakeupDB {
    conn: Mutex<Connection>,
}

impl WakeupDB {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open wakeups DB at {}", db_path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Rebuildable cache: a stale-schema probe failure means the table is
        // absent (DROP is a no-op) or from an older shape (DROP clears it).
        if conn.prepare("SELECT fire_at FROM wakeups LIMIT 0").is_err() {
            conn.execute_batch("DROP TABLE IF EXISTS wakeups;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS wakeups (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                note TEXT,
                fire_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                fired INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_wakeups_session ON wakeups(session_id);
            CREATE INDEX IF NOT EXISTS idx_wakeups_pending ON wakeups(fired, fire_at);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn insert(&self, w: &Wakeup) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO wakeups (id, session_id, agent_id, note, fire_at, created_at, fired)
             VALUES (?1,?2,?3,?4,?5,?6,0)",
            params![
                w.id,
                w.session_id,
                w.agent_id,
                w.note,
                w.fire_at,
                w.created_at,
            ],
        )?;
        Ok(())
    }

    /// All unfired wakeups, oldest fire_at first (for ordered restart replay).
    pub fn list_pending(&self) -> Result<Vec<Wakeup>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_id, note, fire_at, created_at
             FROM wakeups WHERE fired = 0 ORDER BY fire_at ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Wakeup {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    note: row.get(3)?,
                    fire_at: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_pending(&self, id: &str) -> Result<Option<Wakeup>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_id, note, fire_at, created_at
             FROM wakeups WHERE id = ?1 AND fired = 0",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(Wakeup {
            id: row.get(0)?,
            session_id: row.get(1)?,
            agent_id: row.get(2)?,
            note: row.get(3)?,
            fire_at: row.get(4)?,
            created_at: row.get(5)?,
        }))
    }

    /// Reassign every unfired wakeup from one Agent to another and return the
    /// exact rows changed so a surrounding lifecycle transaction can perform
    /// conditional compensation if a later step fails.
    pub fn reassign_pending_agent(&self, old: &str, replacement: &str) -> Result<Vec<Wakeup>> {
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT id, session_id, agent_id, note, fire_at, created_at
                 FROM wakeups WHERE fired = 0 AND agent_id = ?1 ORDER BY fire_at ASC",
            )?;
            let rows = stmt.query_map(params![old], |row| {
                Ok(Wakeup {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    agent_id: row.get(2)?,
                    note: row.get(3)?,
                    fire_at: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        tx.execute(
            "UPDATE wakeups SET agent_id = ?1 WHERE fired = 0 AND agent_id = ?2",
            params![replacement, old],
        )?;
        tx.commit()?;
        Ok(rows)
    }

    /// Restore only rows that still contain the lifecycle rewrite value. This
    /// avoids clobbering an unrelated concurrent cancellation or reassignment.
    pub fn restore_reassigned_agent(&self, rows: &[Wakeup], expected_current: &str) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction()?;
        for row in rows {
            tx.execute(
                "UPDATE wakeups SET agent_id = ?1
                 WHERE id = ?2 AND fired = 0 AND agent_id = ?3",
                params![row.agent_id, row.id, expected_current],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete a wakeup row (delivered, or being cancelled). Delivered wakeups
    /// are transient — deleting on delivery both prevents a restart re-arming an
    /// already-fired wakeup (the row is gone, so `list_pending` won't see it) and
    /// auto-GCs the table. Idempotent: deleting a missing id is a no-op.
    pub fn delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute("DELETE FROM wakeups WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete every wakeup for a session (session delete / incognito burn).
    /// Returns the number of rows removed.
    pub fn delete_for_session(&self, session_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "DELETE FROM wakeups WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> WakeupDB {
        let mut path = std::env::temp_dir();
        path.push(format!("ha-wakeup-db-{}.db", uuid::Uuid::new_v4().simple()));
        WakeupDB::open(&path).expect("open")
    }

    fn mk(id: &str, session: &str, fire_at: i64) -> Wakeup {
        Wakeup {
            id: id.into(),
            session_id: session.into(),
            agent_id: "ha-main".into(),
            note: Some("continue".into()),
            fire_at,
            created_at: 100,
        }
    }

    #[test]
    fn insert_list_and_delete() {
        let db = temp_db();
        db.insert(&mk("w1", "s1", 200)).unwrap();
        db.insert(&mk("w2", "s1", 150)).unwrap();

        // Ordered by fire_at ASC.
        let pending = db.list_pending().unwrap();
        assert_eq!(
            pending.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["w2", "w1"]
        );

        // delete (delivery / cancel) removes it from pending; idempotent.
        db.delete("w2").unwrap();
        db.delete("w2").unwrap(); // missing id is a no-op
        let pending = db.list_pending().unwrap();
        assert_eq!(
            pending.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["w1"]
        );
    }

    #[test]
    fn delete_for_session_clears_all() {
        let db = temp_db();
        db.insert(&mk("w1", "s1", 200)).unwrap();
        db.insert(&mk("w2", "s1", 250)).unwrap();
        db.insert(&mk("w3", "s2", 300)).unwrap();

        assert_eq!(db.delete_for_session("s1").unwrap(), 2);
        let pending = db.list_pending().unwrap();
        assert_eq!(
            pending.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["w3"]
        );
    }

    #[test]
    fn reassign_and_restore_pending_agent_are_exact() {
        let db = temp_db();
        let mut old = mk("w1", "s1", 200);
        old.agent_id = "old".into();
        let mut other = mk("w2", "s2", 250);
        other.agent_id = "other".into();
        db.insert(&old).unwrap();
        db.insert(&other).unwrap();

        let changed = db.reassign_pending_agent("old", "replacement").unwrap();
        assert_eq!(changed, vec![old.clone()]);
        let pending = db.list_pending().unwrap();
        assert_eq!(pending[0].agent_id, "replacement");
        assert_eq!(pending[1].agent_id, "other");

        db.restore_reassigned_agent(&changed, "replacement")
            .unwrap();
        let pending = db.list_pending().unwrap();
        assert_eq!(pending[0].agent_id, "old");
        assert_eq!(pending[1].agent_id, "other");
    }
}
