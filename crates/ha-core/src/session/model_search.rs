//! Model history access is derived from the bound execution context. Search
//! candidates and their later context windows use the same source policy.
use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::tool_defs::{ToolExecContext, ToolTurnProvenance};

#[derive(Clone)]
pub(crate) struct ModelHistorySource {
    session_id: Option<String>,
    turn_id: Option<String>,
    foreground: bool,
    incognito: bool,
    stop_epoch: Option<u64>,
    global_epoch: Option<u64>,
}

pub(super) const NONPRIVATE_SEARCH_SQL: &str =
    "s.kind NOT IN ('personal_assistant','personal_assistant_im') AND NOT EXISTS
     (SELECT 1 FROM personal_assistant_private_sessions private WHERE private.session_id=s.id)";

impl ModelHistorySource {
    pub(crate) fn from_context(ctx: &ToolExecContext) -> Self {
        Self {
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            foreground: ctx.turn_provenance == ToolTurnProvenance::ForegroundUser,
            incognito: ctx.incognito,
            stop_epoch: ctx.turn_admitted_stop_epoch,
            global_epoch: ctx.turn_admitted_global_stop_epoch,
        }
    }

    fn is_owner_home(&self, conn: &Connection, target: Option<&str>) -> Result<bool> {
        let Some(source) = self.session_id.as_deref() else {
            if self.incognito {
                bail!("An incognito search requires its bound session");
            }
            return Ok(false);
        };
        let (kind, incognito): (String, bool) = conn
            .query_row(
                "SELECT kind,incognito FROM sessions WHERE id=?1",
                [source],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("Search source is unavailable"))?;
        if kind == "personal_assistant_im" {
            bail!("Restricted IM views cannot search conversation history");
        }
        if (self.incognito || incognito) && target != Some(source) {
            bail!("Incognito sources may search only their current session");
        }
        if kind != "personal_assistant" {
            return Ok(false);
        }
        let (Some(turn), Some(epoch), Some(stop_epoch)) =
            (self.turn_id.as_deref(), self.global_epoch, self.stop_epoch)
        else {
            bail!("Private assistant search requires a live owner turn");
        };
        if !self.foreground {
            bail!("Private assistant search requires owner input");
        }
        // Read authority is separate from dispatch eligibility: a fresh owner
        // turn may inspect a paused conversation without resuming controllers.
        let live: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN messages m
                 ON m.id=t.user_message_id AND m.session_id=t.session_id
             WHERE t.id=?1 AND t.session_id=?2 AND t.status='running'
               AND t.source IN ('desktop','http') AND m.role='user'
               AND COALESCE(m.source,'desktop') IN ('desktop','http'))",
            rusqlite::params![turn, source],
            |row| row.get(0),
        )?;
        let global: i64 = conn.query_row(
            "SELECT epoch FROM runtime_control_epochs WHERE key='global_stop'",
            [],
            |row| row.get(0),
        )?;
        let current: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=?1",
            [source],
            |row| row.get(0),
        )?;
        if !live || u64::try_from(global)? != epoch || u64::try_from(current)? != stop_epoch {
            bail!("Private assistant search was superseded by Stop");
        }
        Ok(true)
    }

    /// No public or model-supplied allow-private boolean exists. The only
    /// exceptions are a validated owner home turn or an explicitly scoped
    /// lookup of the bound current session.
    pub(super) fn private_filter(
        &self,
        conn: &Connection,
        target: Option<&str>,
    ) -> Result<Option<&'static str>> {
        if self.is_owner_home(conn, target)?
            || target.is_some_and(|target| self.session_id.as_deref() == Some(target))
        {
            Ok(None)
        } else {
            Ok(Some(NONPRIVATE_SEARCH_SQL))
        }
    }

    pub(super) fn may_read_window(
        &self,
        conn: &Connection,
        target: &str,
        global: bool,
    ) -> Result<bool> {
        let private_filter = self.private_filter(conn, (!global).then_some(target))?;
        let current = self.session_id.as_deref() == Some(target);
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=?1
             AND NOT EXISTS(SELECT 1 FROM session_import_sources i WHERE i.session_id=s.id)
             AND (s.incognito=0 OR ?2)
             {} {})",
            if global {
                "AND s.archived_at IS NULL AND s.is_cron=0 AND s.parent_session_id IS NULL
                 AND NOT EXISTS(SELECT 1 FROM channel_conversations c WHERE c.session_id=s.id)
                 AND s.kind NOT IN ('side','knowledge','design','eval_fixture','personal_assistant','personal_assistant_im')"
            } else {
                ""
            },
            private_filter
                .map(|filter| format!("AND ({filter})"))
                .unwrap_or_default(),
        );
        Ok(
            conn.query_row(&sql, rusqlite::params![target, current && !global], |row| {
                row.get(0)
            })?,
        )
    }
}

pub(crate) struct ModelSearchWindow {
    pub hit: super::SessionSearchResult,
    pub messages: Vec<super::SessionMessage>,
    pub total: u32,
    pub has_more_before: bool,
    pub has_more_after: bool,
}
