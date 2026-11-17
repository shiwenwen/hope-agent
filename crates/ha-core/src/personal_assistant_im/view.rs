use super::store::{
    command_source_current, eligible_session, grant, load, read_command, session_stop_epoch,
};
use super::*;
use anyhow::Result;
use rusqlite::{params, Connection as SqlConnection, OptionalExtension};

use crate::session::SessionDB;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantImTarget {
    pub session_id: String,
    pub title: Option<String>,
    pub query_status: bool,
    pub delegate: bool,
    pub stop: bool,
    pub grant_revision: i64,
}

/// A projection built from current permission intersected with the authority
/// present when the platform input was accepted. It contains no home history.
pub struct AssistantImContext {
    pub request_id: String,
    pub view_session_id: String,
    pub home_session_id: String,
    pub text: String,
    pub targets: Vec<AssistantImTarget>,
}

pub(super) fn migrate(conn: &SqlConnection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_im_views (
            session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            home_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            connection_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            scope_key TEXT NOT NULL,
            UNIQUE(connection_id, generation, scope_key)
         );
         CREATE TABLE IF NOT EXISTS assistant_im_command_views (
            request_id TEXT PRIMARY KEY REFERENCES assistant_im_commands(request_id) ON DELETE CASCADE,
            view_session_id TEXT NOT NULL REFERENCES assistant_im_views(session_id) ON DELETE CASCADE
         );
         CREATE TRIGGER IF NOT EXISTS assistant_im_view_scope
         BEFORE UPDATE OF kind,incognito,project_id,parent_session_id,forked_from_session_id ON sessions
         WHEN OLD.kind = 'personal_assistant_im' AND
              (NEW.kind <> 'personal_assistant_im' OR NEW.incognito <> 0
               OR NEW.project_id IS NOT NULL OR NEW.parent_session_id IS NOT NULL
               OR NEW.forked_from_session_id IS NOT NULL)
         BEGIN SELECT RAISE(ABORT,'private assistant IM scope is fixed'); END;
         CREATE TRIGGER IF NOT EXISTS assistant_im_memory_update
         BEFORE UPDATE ON session_memory_policy
         WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id = NEW.session_id AND s.kind = 'personal_assistant_im')
              AND (NEW.use_memories <> 'deny' OR NEW.contribute_to_memories <> 'deny')
         BEGIN SELECT RAISE(ABORT,'private assistant IM memory is disabled'); END;
         CREATE TRIGGER IF NOT EXISTS assistant_im_memory_insert
         BEFORE INSERT ON session_memory_policy
         WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id = NEW.session_id AND s.kind = 'personal_assistant_im')
              AND (NEW.use_memories <> 'deny' OR NEW.contribute_to_memories <> 'deny')
         BEGIN SELECT RAISE(ABORT,'private assistant IM memory is disabled'); END;
         CREATE TRIGGER IF NOT EXISTS assistant_im_memory_delete
         BEFORE DELETE ON session_memory_policy
         WHEN EXISTS(SELECT 1 FROM sessions s WHERE s.id = OLD.session_id AND s.kind = 'personal_assistant_im')
         BEGIN SELECT RAISE(ABORT,'private assistant IM memory is disabled'); END;",
    )?;
    Ok(())
}

fn targets_for_command(
    conn: &SqlConnection,
    state: &Connection,
    command_id: &str,
    now: i64,
    allow_completed: bool,
) -> Result<(String, Vec<AssistantImTarget>)> {
    let accepted: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_im_commands
         WHERE request_id = ?1 AND connection_id = ?2 AND generation = ?3
           AND (state IN ('pending','running') OR (?4 AND state IN ('reply_pending','completed','failed')
             AND EXISTS (SELECT 1 FROM assistant_im_command_views cv
               JOIN chat_turns t ON t.id = cv.request_id AND t.session_id = cv.view_session_id
               JOIN messages m ON m.id = t.user_message_id AND m.session_id = t.session_id
               WHERE cv.request_id = ?1 AND t.source = 'channel' AND m.source = 'channel'
                 AND m.role = 'user' AND t.status IN ('completed','failed','interrupted','cancelled')))))",
        params![
            command_id,
            state.view.connection_id,
            state.view.generation,
            allow_completed
        ],
        |row| row.get(0),
    )?;
    let command = read_command(conn, command_id)?.ok_or(AssistantImError::Unavailable)?;
    if !accepted
        || state.view.state != PairingState::Active
        || !command_source_current(conn, &command)?
    {
        return Err(AssistantImError::PermissionDenied.into());
    }
    let mut targets = Vec::new();
    for (session_id, revision) in command.grant_revisions {
        let Some(permission) = grant(conn, &state.view.connection_id, &session_id)? else {
            return Err(AssistantImError::PermissionDenied.into());
        };
        if permission.revision != revision
            || permission.expires_at <= now
            || !eligible_session(conn, &session_id)?
            || (!permission.query_status && !permission.delegate && !permission.stop)
            || command.target_stop_epochs.get(&session_id).copied()
                != Some(u64::try_from(session_stop_epoch(conn, &session_id)?)?)
        {
            return Err(AssistantImError::PermissionDenied.into());
        }
        let title: Option<String> = conn.query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            [&session_id],
            |row| row.get(0),
        )?;
        let title = title.map(|title| crate::truncate_utf8(&title, 160).to_owned());
        targets.push(AssistantImTarget {
            session_id,
            title,
            query_status: permission.query_status,
            delegate: permission.delegate,
            stop: permission.stop,
            grant_revision: revision,
        });
    }
    Ok((command.text, targets))
}

impl SessionDB {
    pub fn prepare_assistant_im_context(
        &self,
        command_id: &str,
        now: i64,
    ) -> Result<AssistantImContext> {
        let home = self.ensure_personal_assistant_home()?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        let (text, targets) = targets_for_command(&tx, &state, command_id, now, false)?;
        let scope: Vec<_> = targets
            .iter()
            .map(|target| (&target.session_id, target.grant_revision))
            .collect();
        let scope_key = serde_json::to_string(&scope)?;
        let existing: Option<String> = tx.query_row(
            "SELECT v.session_id FROM assistant_im_views v JOIN sessions s ON s.id = v.session_id
             WHERE v.connection_id = ?1 AND v.generation = ?2 AND v.scope_key = ?3
               AND s.kind = 'personal_assistant_im' AND s.incognito = 0",
            params![state.view.connection_id, state.view.generation, scope_key],
            |row| row.get(0),
        ).optional()?;
        let session_id = match existing {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let timestamp = chrono::Utc::now().to_rfc3339();
                tx.execute(
                    "INSERT INTO sessions(id,title,agent_id,created_at,updated_at,kind)
                     VALUES (?1,'Hope · Telegram',?2,?3,?3,'personal_assistant_im')",
                    params![id, crate::agent_loader::DEFAULT_AGENT_ID, timestamp],
                )?;
                tx.execute(
                    "INSERT INTO personal_assistant_private_sessions(session_id) VALUES (?1)",
                    [&id],
                )?;
                tx.execute(
                    "INSERT INTO session_memory_policy(session_id,use_memories,contribute_to_memories,updated_at)
                     VALUES (?1,'deny','deny',?2)",
                    params![id, timestamp],
                )?;
                tx.execute(
                    "INSERT INTO assistant_im_views(session_id,home_session_id,connection_id,generation,scope_key)
                     VALUES (?1,?2,?3,?4,?5)",
                    params![id, home.id, state.view.connection_id, state.view.generation, scope_key],
                )?;
                id
            }
        };
        let command = read_command(&tx, command_id)?.ok_or(AssistantImError::Unavailable)?;
        if session_stop_epoch(&tx, &session_id)? != i64::try_from(command.view_stop_epoch)? {
            return Err(AssistantImError::StaleGeneration.into());
        }
        tx.execute(
            "UPDATE assistant_im_commands SET source_view_id = ?1 WHERE request_id = ?2 AND source_view_id IS NULL",
            params![session_id,command_id],
        )?;
        let previous_view: Option<String> = tx
            .query_row(
                "SELECT view_session_id FROM assistant_im_command_views WHERE request_id = ?1",
                [command_id],
                |row| row.get(0),
            )
            .optional()?;
        if previous_view
            .as_ref()
            .is_some_and(|previous| previous != &session_id)
        {
            return Err(AssistantImError::StaleGeneration.into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO assistant_im_command_views(request_id,view_session_id) VALUES (?1,?2)",
            params![command_id, session_id],
        )?;
        tx.commit()?;
        Ok(AssistantImContext {
            request_id: command_id.to_owned(),
            view_session_id: session_id,
            home_session_id: home.id,
            text,
            targets,
        })
    }

    /// Every tool call and the kernel admission boundary recheck this mapping.
    /// `turn_id` is the immutable inbound request ID chosen before model work.
    pub fn assistant_im_turn_targets(
        &self,
        view_session_id: &str,
        turn_id: &str,
        now: i64,
    ) -> Result<Vec<AssistantImTarget>> {
        let conn = self.read_conn()?;
        let state = load(&conn)?.ok_or(AssistantImError::Unavailable)?;
        let matches: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_im_command_views cv
             JOIN assistant_im_views v ON v.session_id = cv.view_session_id
             WHERE cv.request_id = ?1 AND cv.view_session_id = ?2
               AND v.connection_id = ?3 AND v.generation = ?4)",
            params![
                turn_id,
                view_session_id,
                state.view.connection_id,
                state.view.generation
            ],
            |row| row.get(0),
        )?;
        if !matches {
            return Err(AssistantImError::PermissionDenied.into());
        }
        let (_, targets) = targets_for_command(&conn, &state, turn_id, now, false)?;
        Ok(targets)
    }
}

/// Kernel-only dispatch proof for the existing canonical assistant request
/// ledger. The actual source message remains in the restricted IM view.
pub(crate) fn authorize_source_turn(
    conn: &SqlConnection,
    turn_id: &str,
    target_session_id: &str,
    now: i64,
) -> Result<(String, i64)> {
    let state = load(conn)?.ok_or(AssistantImError::Unavailable)?;
    let source: (String, i64) = conn
        .query_row(
            "SELECT v.home_session_id, t.user_message_id
         FROM assistant_im_command_views cv
         JOIN assistant_im_views v ON v.session_id = cv.view_session_id
         JOIN chat_turns t ON t.id = cv.request_id AND t.session_id = v.session_id
         JOIN messages m ON m.id = t.user_message_id AND m.session_id = v.session_id
         WHERE cv.request_id = ?1 AND v.connection_id = ?2 AND v.generation = ?3
           AND t.status = 'running' AND t.source = 'channel'
           AND m.role = 'user' AND m.source = 'channel'",
            params![turn_id, state.view.connection_id, state.view.generation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(AssistantImError::PermissionDenied)?;
    let (_, targets) = targets_for_command(conn, &state, turn_id, now, false)?;
    if !targets
        .iter()
        .any(|target| target.session_id == target_session_id && target.delegate)
    {
        return Err(AssistantImError::PermissionDenied.into());
    }
    Ok(source)
}

/// Pending target work revalidates its original authority after the source
/// model has finished. Completion of the source turn never widens its grant.
pub(crate) fn revalidate_assistant_im_dispatch(
    conn: &SqlConnection,
    command_id: &str,
    target_session_id: &str,
    now: i64,
) -> Result<()> {
    let state = load(conn)?.ok_or(AssistantImError::Unavailable)?;
    let (_, targets) = targets_for_command(conn, &state, command_id, now, true)?;
    if !targets
        .iter()
        .any(|target| target.session_id == target_session_id && target.delegate)
    {
        return Err(AssistantImError::PermissionDenied.into());
    }
    Ok(())
}

pub(crate) fn validate_admitted_assistant_im_with_conn(
    conn: &SqlConnection,
    view_session_id: &str,
    command_id: &str,
    now: i64,
) -> Result<()> {
    let state = load(conn)?.ok_or(AssistantImError::Unavailable)?;
    let matches: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_im_command_views cv
         JOIN assistant_im_views v ON v.session_id = cv.view_session_id
         WHERE cv.request_id = ?1 AND cv.view_session_id = ?2
           AND v.connection_id = ?3 AND v.generation = ?4)",
        params![
            command_id,
            view_session_id,
            state.view.connection_id,
            state.view.generation
        ],
        |row| row.get(0),
    )?;
    if !matches {
        return Err(AssistantImError::PermissionDenied.into());
    }
    targets_for_command(conn, &state, command_id, now, false)?;
    Ok(())
}

pub(crate) fn authorize_assistant_im_cancel_with_conn(
    conn: &SqlConnection,
    view_session_id: &str,
    command_id: &str,
    target_session_id: &str,
    now: i64,
) -> Result<()> {
    validate_admitted_assistant_im_with_conn(conn, view_session_id, command_id, now)?;
    let running: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN messages m ON m.id = t.user_message_id
         WHERE t.id = ?1 AND t.session_id = ?2 AND t.source = 'channel' AND t.status = 'running'
           AND m.session_id = t.session_id AND m.role = 'user' AND m.source = 'channel')",
        params![command_id, view_session_id],
        |row| row.get(0),
    )?;
    let state = load(conn)?.ok_or(AssistantImError::Unavailable)?;
    let (_, targets) = targets_for_command(conn, &state, command_id, now, false)?;
    if !running
        || !targets
            .iter()
            .any(|target| target.session_id == target_session_id && target.stop)
    {
        return Err(AssistantImError::PermissionDenied.into());
    }
    Ok(())
}
