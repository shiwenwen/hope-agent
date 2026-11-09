use super::*;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::{params, Connection as SqlConnection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

use crate::session::SessionDB;

pub(crate) fn migrate(conn: &SqlConnection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS im_bot_purposes (
            platform TEXT NOT NULL,
            subject_id INTEGER NOT NULL,
            purpose TEXT NOT NULL,
            account_id TEXT,
            committed INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY(platform, subject_id)
         );
         CREATE TABLE IF NOT EXISTS assistant_im_connection (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            state_json TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_im_grants (
            connection_id TEXT NOT NULL,
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            grant_json TEXT NOT NULL,
            PRIMARY KEY(connection_id, session_id)
         );
         CREATE TABLE IF NOT EXISTS assistant_im_received (
            connection_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            cursor_epoch INTEGER NOT NULL,
            update_id INTEGER NOT NULL,
            received_at INTEGER NOT NULL,
            PRIMARY KEY(connection_id, generation, cursor_epoch, update_id)
         );
         CREATE TABLE IF NOT EXISTS assistant_im_commands (
            request_id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            text TEXT NOT NULL,
            grant_revisions TEXT NOT NULL,
            global_stop_epoch INTEGER NOT NULL,
            home_stop_epoch INTEGER NOT NULL,
            target_stop_epochs TEXT NOT NULL,
            source_view_id TEXT,
            view_stop_epoch INTEGER NOT NULL,
            state TEXT NOT NULL,
            created_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS assistant_im_deliveries (
            id TEXT PRIMARY KEY,
            connection_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            request_id TEXT NOT NULL,
            session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
            grant_revision INTEGER NOT NULL,
            grant_revisions TEXT,
            origin_command_id TEXT,
            completion_notice INTEGER NOT NULL,
            text TEXT NOT NULL,
            state TEXT NOT NULL,
            message_id INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            UNIQUE(connection_id, generation, request_id)
         );",
    )?;
    super::notification_policy::migrate(conn)?;
    super::pairing_delivery::migrate(conn)?;
    super::view::migrate(conn)?;
    Ok(())
}

pub(super) fn load(conn: &SqlConnection) -> Result<Option<Connection>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT state_json FROM assistant_im_connection WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| serde_json::from_str(&json).context("read assistant IM state"))
        .transpose()
}

fn save(conn: &SqlConnection, state: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO assistant_im_connection(singleton, state_json) VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET state_json = excluded.state_json",
        [serde_json::to_string(state)?],
    )?;
    Ok(())
}

pub(super) fn check_lease(state: &Connection, lease: &PollLease) -> Result<()> {
    if state.view.connection_id != lease.connection_id
        || state.view.bot_id != lease.bot_id
        || state.view.generation != lease.generation
        || state.credential_ref != lease.credential_ref
        || state.cursor_epoch != lease.cursor_epoch
    {
        return Err(AssistantImError::StaleGeneration.into());
    }
    Ok(())
}

pub(super) fn lease(state: &Connection) -> PollLease {
    PollLease {
        connection_id: state.view.connection_id.clone(),
        bot_id: state.view.bot_id,
        generation: state.view.generation,
        credential_ref: state.credential_ref.clone(),
        cursor: state.cursor,
        cursor_epoch: state.cursor_epoch,
    }
}

fn nonce() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>())
}

pub(super) fn hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

pub(super) fn current_global_epoch(conn: &SqlConnection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT epoch FROM runtime_control_epochs WHERE key = 'global_stop'",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn session_stop_epoch(conn: &SqlConnection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM session_autonomy_pauses WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?)
}

pub(super) fn current_home_epoch(conn: &SqlConnection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM session_autonomy_pauses p JOIN sessions s ON s.id = p.session_id
         WHERE s.kind = 'personal_assistant'",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn read_command(
    conn: &SqlConnection,
    request_id: &str,
) -> Result<Option<ReceivedCommand>> {
    let row = conn.query_row(
        "SELECT connection_id,generation,text,grant_revisions,global_stop_epoch,home_stop_epoch,target_stop_epochs,source_view_id,view_stop_epoch
         FROM assistant_im_commands WHERE request_id = ?1",
        [request_id],
        |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
            row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?, row.get::<_, i64>(8)?,
        )),
    ).optional()?;
    let Some((
        connection_id,
        generation,
        text,
        revisions,
        global,
        home,
        target_epochs,
        source_view_id,
        view_stop_epoch,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(ReceivedCommand {
        request_id: request_id.to_owned(),
        connection_id,
        generation,
        text,
        grant_revisions: serde_json::from_str(&revisions)?,
        global_stop_epoch: u64::try_from(global).map_err(|_| AssistantImError::Unavailable)?,
        home_stop_epoch: u64::try_from(home).map_err(|_| AssistantImError::Unavailable)?,
        target_stop_epochs: serde_json::from_str(&target_epochs)?,
        source_view_id,
        view_stop_epoch: u64::try_from(view_stop_epoch)
            .map_err(|_| AssistantImError::Unavailable)?,
    }))
}

pub(super) fn command_source_current(
    conn: &SqlConnection,
    command: &ReceivedCommand,
) -> Result<bool> {
    let paused: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_autonomy_pauses p JOIN sessions s ON s.id = p.session_id
         WHERE s.kind = 'personal_assistant' AND p.resumed_at IS NULL)",
        [], |row| row.get(0),
    )?;
    let view_current = match &command.source_view_id {
        Some(session_id) => {
            let view_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_im_views v
                 JOIN sessions s ON s.id = v.session_id
                 JOIN sessions h ON h.id = v.home_session_id
                 WHERE v.session_id = ?1 AND s.kind = 'personal_assistant_im'
                   AND s.archived_at IS NULL AND h.kind = 'personal_assistant'
                   AND h.archived_at IS NULL AND v.connection_id = ?2 AND v.generation = ?3)",
                params![session_id, command.connection_id, command.generation],
                |row| row.get(0),
            )?;
            view_exists
                && session_stop_epoch(conn, session_id)? == i64::try_from(command.view_stop_epoch)?
        }
        None => true,
    };
    Ok(!paused
        && view_current
        && current_global_epoch(conn)? == i64::try_from(command.global_stop_epoch)?
        && current_home_epoch(conn)? == i64::try_from(command.home_stop_epoch)?)
}

fn claim_bot(
    conn: &SqlConnection,
    bot_id: i64,
    purpose: BotPurpose,
    account_id: &str,
    committed: bool,
) -> Result<()> {
    if bot_id <= 0 || account_id.is_empty() || account_id.len() > 128 {
        return Err(AssistantImError::InvalidInput.into());
    }
    conn.execute(
        "INSERT INTO im_bot_purposes(platform, subject_id, purpose, account_id,committed)
         VALUES ('telegram', ?1, ?2, ?3, ?4)
         ON CONFLICT(platform, subject_id) DO NOTHING",
        params![bot_id, purpose.as_str(), account_id, committed],
    )?;
    let (stored_purpose, stored_account): (String, Option<String>) = conn.query_row(
        "SELECT purpose, account_id FROM im_bot_purposes
         WHERE platform = 'telegram' AND subject_id = ?1",
        [bot_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if stored_purpose != purpose.as_str()
        || stored_account.is_some_and(|stored| stored != account_id)
    {
        return Err(AssistantImError::IdentityInUse.into());
    }
    conn.execute(
        "UPDATE im_bot_purposes SET account_id = ?1,committed = MAX(committed,?3)
         WHERE platform = 'telegram' AND subject_id = ?2",
        params![account_id, bot_id, committed],
    )?;
    Ok(())
}

pub(super) fn invalidate(conn: &SqlConnection, connection_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE assistant_im_activations SET state = 'cancelled', callback_data = ''
         WHERE connection_id = ?1 AND state = 'pending'",
        [connection_id],
    )?;
    conn.execute(
        "DELETE FROM assistant_im_grants WHERE connection_id = ?1",
        [connection_id],
    )?;
    conn.execute(
        "UPDATE assistant_im_commands SET state = 'cancelled', text = ''
         WHERE connection_id = ?1 AND state = 'pending'",
        [connection_id],
    )?;
    conn.execute(
        "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
         WHERE connection_id = ?1 AND state = 'pending'",
        [connection_id],
    )?;
    Ok(())
}

fn reset_pairing(state: &mut Connection) {
    state.view.generation += 1;
    state.view.state = PairingState::Unpaired;
    state.view.user_id = None;
    state.view.chat_id = None;
    state.view.notifications_paused = true;
    state.pair_hash = None;
    state.activation_hash = None;
    state.pair_attempts = 0;
}

pub(super) fn eligible_session(conn: &SqlConnection, session_id: &str) -> Result<bool> {
    match crate::personal_assistant::validate_target(conn, session_id) {
        Ok(()) => Ok(true),
        Err(error)
            if error
                .downcast_ref::<crate::personal_assistant::AssistantError>()
                .is_some() =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn grant(
    conn: &SqlConnection,
    connection_id: &str,
    session_id: &str,
) -> Result<Option<SessionGrant>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT grant_json FROM assistant_im_grants
             WHERE connection_id = ?1 AND session_id = ?2",
            params![connection_id, session_id],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| serde_json::from_str(&json).context("read assistant IM grant"))
        .transpose()
}

impl SessionDB {
    pub fn claim_assistant_im_command(
        &self,
        request_id: &str,
        _now: i64,
    ) -> Result<Option<ReceivedCommand>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(state) = load(&tx)? else {
            return Ok(None);
        };
        let Some(command) = read_command(&tx, request_id)? else {
            return Ok(None);
        };
        if state.view.state != PairingState::Active
            || state.view.connection_id != command.connection_id
            || state.view.generation != command.generation
            || !command_source_current(&tx, &command)?
        {
            tx.execute(
                "UPDATE assistant_im_commands SET state = 'cancelled',text = ''
                 WHERE request_id = ?1 AND state = 'pending'",
                [request_id],
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let changed = tx.execute(
            "UPDATE assistant_im_commands SET state = 'running'
             WHERE request_id = ?1 AND state = 'pending'",
            [request_id],
        )?;
        tx.commit()?;
        Ok((changed == 1).then_some(command))
    }

    pub fn settle_assistant_im_command(
        &self,
        request_id: &str,
        terminal: CommandTerminal,
    ) -> Result<()> {
        let state = match terminal {
            CommandTerminal::Completed => "completed",
            CommandTerminal::Failed => "failed",
            CommandTerminal::Unknown => "unknown",
        };
        self.with_conn_internal(|conn| {
            conn.execute(
                "UPDATE assistant_im_commands SET state = ?1,text = ''
                 WHERE request_id = ?2 AND state IN ('pending','running','reply_pending')",
                params![state, request_id],
            )?;
            Ok(())
        })
    }

    /// Run once after the shared turn recovery. Work that has a durable turn
    /// is never re-submitted merely because its transport did not see a reply.
    pub fn recover_assistant_im_commands(&self) -> Result<usize> {
        self.with_conn_internal(|conn| {
            Ok(conn.execute(
                "UPDATE assistant_im_commands SET
                 state = CASE
                   WHEN NOT EXISTS(SELECT 1 FROM chat_turns t WHERE t.id = request_id) THEN 'pending'
                   WHEN EXISTS(SELECT 1 FROM chat_turns t WHERE t.id = request_id AND t.status IN ('completed','failed','interrupted','cancelled')) THEN 'reply_pending'
                   ELSE 'unknown' END,
                 text = CASE WHEN EXISTS(SELECT 1 FROM chat_turns t WHERE t.id = request_id) THEN '' ELSE text END
                 WHERE state = 'running'",
                [],
            )?)
        })
    }
    /// Called after platform getMe, before menu changes, polling or webhooks.
    pub fn claim_telegram_bot(
        &self,
        bot_id: i64,
        purpose: BotPurpose,
        account_id: &str,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim_bot(&tx, bot_id, purpose, account_id, true)?;
        tx.commit()?;
        Ok(())
    }

    /// Release an activity claim only after stopping its consumer. The
    /// platform identity's product-purpose tombstone is deliberately retained.
    pub fn release_telegram_bot(&self, bot_id: i64, account_id: &str) -> Result<()> {
        self.with_conn_internal(|conn| {
            conn.execute(
                "UPDATE im_bot_purposes SET account_id = NULL
                 WHERE platform = 'telegram' AND subject_id = ?1 AND account_id = ?2",
                params![bot_id, account_id],
            )?;
            Ok(())
        })
    }

    pub fn reserve_telegram_bot(&self, bot_id: i64, account_id: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim_bot(&tx, bot_id, BotPurpose::Ordinary, account_id, false)?;
        tx.commit()?;
        Ok(())
    }

    /// Caller holds the process-shared Telegram configuration guard and passes
    /// a freshly reloaded authoritative config, never the ambient cache.
    /// Present accounts remain provisional until their current credentials are
    /// resolved against Telegram's canonical identity endpoint.
    pub fn reconcile_telegram_bot_reservations(
        &self,
        configured_account_ids: &[String],
    ) -> Result<Vec<(i64, String)>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let reservations = {
            let mut statement = tx.prepare(
                "SELECT subject_id,account_id FROM im_bot_purposes
                 WHERE platform='telegram' AND purpose='ordinary' AND committed=0",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut retained = Vec::new();
        for (bot_id, account_id) in reservations {
            if let Some(account_id) = account_id.filter(|id| configured_account_ids.contains(id)) {
                retained.push((bot_id, account_id));
            } else {
                tx.execute(
                    "DELETE FROM im_bot_purposes WHERE platform='telegram' AND subject_id=?1 AND committed=0",
                    [bot_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(retained)
    }

    /// Commit only after canonical identity verification and config publication.
    /// A token replacement releases the old active account binding, retaining
    /// its committed ordinary-purpose tombstone. Unpublished stale reservations
    /// for the same account never became a product identity and can be removed.
    pub fn commit_ordinary_telegram_bot(&self, bot_id: i64, account_id: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        claim_bot(&tx, bot_id, BotPurpose::Ordinary, account_id, true)?;
        tx.execute(
            "DELETE FROM im_bot_purposes WHERE platform='telegram' AND purpose='ordinary'
             AND account_id=?1 AND subject_id<>?2 AND committed=0",
            params![account_id, bot_id],
        )?;
        tx.execute(
            "UPDATE im_bot_purposes SET account_id=NULL
             WHERE platform='telegram' AND purpose='ordinary'
             AND account_id=?1 AND subject_id<>?2 AND committed=1",
            params![account_id, bot_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn discard_telegram_bot_reservation(&self, bot_id: i64, account_id: &str) -> Result<()> {
        self.with_conn_internal(|conn| {
            conn.execute(
                "DELETE FROM im_bot_purposes WHERE platform = 'telegram' AND subject_id = ?1
                 AND account_id = ?2 AND committed = 0",
                params![bot_id, account_id],
            )?;
            Ok(())
        })
    }

    pub fn release_telegram_bot_account(&self, account_id: &str) -> Result<()> {
        self.with_conn_internal(|conn| {
            conn.execute(
                "UPDATE im_bot_purposes SET account_id = NULL WHERE platform = 'telegram' AND account_id = ?1",
                [account_id],
            )?;
            Ok(())
        })
    }

    pub fn assistant_im_connection(&self) -> Result<Option<ConnectionView>> {
        let conn = self.read_conn()?;
        Ok(load(&conn)?.map(|state| state.view))
    }

    pub fn assistant_im_runtime_snapshot(&self) -> Result<Option<PollLease>> {
        let conn = self.read_conn()?;
        Ok(load(&conn)?.as_ref().map(lease))
    }

    /// The caller publishes an immutable secure credential file first. This
    /// commit is the sole activation point; a failed commit leaves it unused.
    pub fn save_assistant_im_connection(
        &self,
        bot_id: i64,
        bot_username: &str,
        credential_ref: &str,
        now: i64,
    ) -> Result<ConnectionView> {
        uuid::Uuid::parse_str(credential_ref).map_err(|_| AssistantImError::InvalidInput)?;
        if bot_username.is_empty()
            || bot_username.len() > 64
            || !bot_username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous = load(&tx)?;
        let connection_id = previous
            .as_ref()
            .map(|state| state.view.connection_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        claim_bot(
            &tx,
            bot_id,
            BotPurpose::PersonalAssistant,
            &connection_id,
            true,
        )?;
        if let Some(previous) = &previous {
            invalidate(&tx, &connection_id)?;
            if previous.view.bot_id != bot_id {
                tx.execute(
                    "UPDATE im_bot_purposes SET account_id = NULL
                     WHERE platform = 'telegram' AND subject_id = ?1 AND account_id = ?2",
                    params![previous.view.bot_id, connection_id],
                )?;
            }
        }
        let state = Connection {
            view: ConnectionView {
                connection_id,
                bot_id,
                bot_username: bot_username.to_owned(),
                generation: previous.as_ref().map_or(1, |s| s.view.generation + 1),
                state: PairingState::Unpaired,
                user_id: None,
                chat_id: None,
                notifications_paused: true,
            },
            credential_ref: credential_ref.to_owned(),
            pair_hash: None,
            pair_expires_at: 0,
            pair_attempts: 0,
            activation_hash: None,
            activation_expires_at: 0,
            cursor: 0,
            cursor_epoch: previous.as_ref().map_or(0, |s| s.cursor_epoch + 1),
            last_update_at: now,
            last_poll_at: now,
            last_expiry_event_at: 0,
        };
        save(&tx, &state)?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(state.view)
    }

    pub fn begin_assistant_im_pairing(&self, now: i64) -> Result<PairingCode> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let mut state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        invalidate(&tx, &state.view.connection_id)?;
        reset_pairing(&mut state);
        let code = nonce();
        state.view.state = PairingState::AwaitingCode;
        state.pair_hash = Some(hash(&code));
        state.pair_expires_at = now + 600;
        save(&tx, &state)?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(PairingCode {
            connection_id: state.view.connection_id,
            generation: state.view.generation,
            code,
            expires_at: now + 600,
        })
    }

    /// Owner confirmation must echo the candidate and generation displayed in
    /// the UI, so a racing re-pair cannot confirm a different remote person.
    pub fn confirm_assistant_im_pairing(
        &self,
        generation: i64,
        user_id: i64,
        chat_id: i64,
        now: i64,
    ) -> Result<ActivationPrompt> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let mut state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        if state.view.generation != generation
            || state.view.state != PairingState::AwaitingConfirmation
            || state.view.user_id != Some(user_id)
            || state.view.chat_id != Some(chat_id)
            || state.pair_expires_at <= now
        {
            return Err(AssistantImError::StaleGeneration.into());
        }
        let activation = nonce();
        state.activation_hash = Some(hash(&activation));
        state.activation_expires_at = now + 600;
        state.view.state = PairingState::AwaitingActivation;
        save(&tx, &state)?;
        tx.execute(
            "INSERT INTO assistant_im_activations
             (id,connection_id,generation,user_id,chat_id,callback_data,state,created_at,expires_at)
             VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?8)",
            params![
                uuid::Uuid::new_v4().to_string(),
                state.view.connection_id,
                generation,
                user_id,
                chat_id,
                format!("{ACTIVATE_PREFIX}{activation}"),
                now,
                state.activation_expires_at
            ],
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(ActivationPrompt {
            lease: lease(&state),
            chat_id,
            callback_data: format!("{ACTIVATE_PREFIX}{activation}"),
        })
    }

    pub fn revoke_assistant_im_pairing(&self) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        if let Some(mut state) = load(&tx)? {
            invalidate(&tx, &state.view.connection_id)?;
            reset_pairing(&mut state);
            save(&tx, &state)?;
        }
        tx.commit()?;
        drop(conn);
        changed();
        Ok(())
    }

    pub fn remove_assistant_im_connection(&self) -> Result<Option<String>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let state = load(&tx)?;
        if let Some(state) = &state {
            invalidate(&tx, &state.view.connection_id)?;
            tx.execute(
                "UPDATE im_bot_purposes SET account_id = NULL
                 WHERE platform = 'telegram' AND subject_id = ?1 AND account_id = ?2",
                params![state.view.bot_id, state.view.connection_id],
            )?;
            tx.execute("DELETE FROM assistant_im_connection", [])?;
        }
        tx.commit()?;
        drop(conn);
        changed();
        Ok(state.map(|state| state.credential_ref))
    }

    pub fn assistant_im_poll_lease(&self, now: i64) -> Result<Option<PollLease>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let Some(mut state) = load(&tx)? else {
            return Ok(None);
        };
        let mut pairing_reset = false;
        // Telegram may randomize update_id after a week without updates. Reset
        // before that window, without treating a lower ID as already consumed.
        if now.saturating_sub(state.last_update_at) >= 6 * 86_400 {
            state.cursor = 0;
            state.cursor_epoch += 1;
            state.last_update_at = now;
            if now.saturating_sub(state.last_poll_at) >= 86_400 {
                invalidate(&tx, &state.view.connection_id)?;
                reset_pairing(&mut state);
                pairing_reset = true;
            }
            save(&tx, &state)?;
        }
        tx.commit()?;
        drop(conn);
        if pairing_reset {
            changed();
        }
        Ok(Some(lease(&state)))
    }

    pub fn expire_assistant_im_state(&self, now: i64) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let Some(mut state) = load(&tx)? else {
            return Ok(());
        };
        let pairing_expired = (matches!(
            state.view.state,
            PairingState::AwaitingCode | PairingState::AwaitingConfirmation
        ) && state.pair_expires_at <= now)
            || (state.view.state == PairingState::AwaitingActivation
                && state.activation_expires_at <= now);
        if pairing_expired {
            invalidate(&tx, &state.view.connection_id)?;
            reset_pairing(&mut state);
        }
        let grant_expired: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_im_grants WHERE connection_id = ?1
             AND json_extract(grant_json,'$.queryStatus') = 1
             AND json_extract(grant_json,'$.expiresAt') <= ?2
             AND json_extract(grant_json,'$.expiresAt') > ?3)",
            params![state.view.connection_id, now, state.last_expiry_event_at],
            |row| row.get(0),
        )?;
        let expired_activation = tx.execute(
            "UPDATE assistant_im_activations SET state = 'cancelled',callback_data = ''
             WHERE state = 'pending' AND expires_at <= ?1",
            [now],
        )? > 0;
        let expired_delivery = tx.execute(
            "UPDATE assistant_im_deliveries SET state = 'cancelled',text = ''
             WHERE state = 'pending' AND expires_at <= ?1",
            [now],
        )? > 0;
        if grant_expired {
            state.last_expiry_event_at = now;
        }
        if pairing_expired || grant_expired {
            save(&tx, &state)?;
        }
        tx.commit()?;
        drop(conn);
        if pairing_expired || grant_expired || expired_delivery || expired_activation {
            changed();
        }
        Ok(())
    }

    pub fn assistant_im_poll_completed(&self, expected: &PollLease, now: i64) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let mut state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        check_lease(&state, expected)?;
        state.last_poll_at = now;
        save(&tx, &state)?;
        tx.commit()?;
        Ok(())
    }

    /// Persist consumption and accepted work together, before acknowledging an
    /// offset to Telegram. Pairing material is hashed and never enters a turn.
    pub fn receive_assistant_im_update(
        &self,
        expected: &PollLease,
        input: TelegramInput,
        now: i64,
    ) -> Result<IngressOutcome> {
        if input.update_id < 0 || input.update_id >= i64::from(i32::MAX) {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let mut state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        check_lease(&state, expected)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO assistant_im_received
             (connection_id, generation, cursor_epoch, update_id, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                expected.connection_id,
                expected.generation,
                expected.cursor_epoch,
                input.update_id,
                now
            ],
        )?;
        if inserted == 0 {
            return Ok(IngressOutcome::Duplicate);
        }
        let outcome = consume_input(&tx, &mut state, input, now)?;
        state.last_update_at = now;
        state.last_poll_at = now;
        save(&tx, &state)?;
        tx.execute(
            "DELETE FROM assistant_im_received WHERE received_at < ?1",
            [now - 8 * 86_400],
        )?;
        tx.commit()?;
        drop(conn);
        if outcome != IngressOutcome::Ignored {
            changed();
        }
        Ok(outcome)
    }

    pub fn assistant_im_grants(&self, _now: i64) -> Result<Vec<SessionGrant>> {
        let conn = self.read_conn()?;
        let Some(state) = load(&conn)? else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare(
            "SELECT grant_json FROM assistant_im_grants WHERE connection_id = ?1 ORDER BY session_id",
        )?;
        let rows = stmt.query_map([state.view.connection_id], |row| row.get::<_, String>(0))?;
        let mut grants = Vec::new();
        for row in rows {
            let grant: SessionGrant = serde_json::from_str(&row?)?;
            if eligible_session(&conn, &grant.session_id)? {
                grants.push(grant);
            }
        }
        Ok(grants)
    }

    /// Owner-only CAS. IM commands may revoke, but cannot call this to widen
    /// targets, operations, duration or disclosure.
    pub fn set_assistant_im_grant(
        &self,
        mut input: SessionGrant,
        now: i64,
    ) -> Result<SessionGrant> {
        if input.expires_at <= now
            || input.expires_at > now + 366 * 86_400
            || ((!input.query_status) && (input.delegate || input.stop || input.completion_notice))
        {
            return Err(AssistantImError::InvalidInput.into());
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        if state.view.state != PairingState::Active || !eligible_session(&tx, &input.session_id)? {
            return Err(AssistantImError::PermissionDenied.into());
        }
        let previous = grant(&tx, &state.view.connection_id, &input.session_id)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM assistant_im_grants WHERE connection_id = ?1
             AND json_extract(grant_json, '$.queryStatus') = 1
             AND json_extract(grant_json, '$.expiresAt') > ?2",
            params![state.view.connection_id, now],
            |row| row.get(0),
        )?;
        if previous.is_none() && count >= 64 {
            return Err(AssistantImError::InvalidInput.into());
        }
        if previous.as_ref().map_or(0, |grant| grant.revision) != input.revision {
            return Err(AssistantImError::StaleGeneration.into());
        }
        input.revision += 1;
        tx.execute(
            "INSERT INTO assistant_im_grants(connection_id, session_id, grant_json)
             VALUES (?1, ?2, ?3) ON CONFLICT(connection_id, session_id)
             DO UPDATE SET grant_json = excluded.grant_json",
            params![
                state.view.connection_id,
                input.session_id,
                serde_json::to_string(&input)?
            ],
        )?;
        tx.execute(
            "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
             WHERE connection_id = ?1 AND state = 'pending' AND
             (session_id = ?2 OR EXISTS(SELECT 1 FROM json_each(grant_revisions) j
              WHERE json_extract(j.value,'$[0]') = ?2))",
            params![state.view.connection_id, input.session_id],
        )?;
        tx.commit()?;
        drop(conn);
        changed();
        Ok(input)
    }

    pub fn revoke_assistant_im_grant(&self, session_id: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        if let Some(state) = load(&tx)? {
            if let Some(mut permission) = grant(&tx, &state.view.connection_id, session_id)? {
                permission.revision += 1;
                permission.query_status = false;
                permission.delegate = false;
                permission.stop = false;
                permission.completion_notice = false;
                tx.execute(
                    "UPDATE assistant_im_grants SET grant_json = ?1
                     WHERE connection_id = ?2 AND session_id = ?3",
                    params![
                        serde_json::to_string(&permission)?,
                        state.view.connection_id,
                        session_id
                    ],
                )?;
            }
            tx.execute(
                "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
                 WHERE connection_id = ?1 AND state = 'pending' AND
                 (session_id = ?2 OR EXISTS(SELECT 1 FROM json_each(grant_revisions) j
                  WHERE json_extract(j.value,'$[0]') = ?2))",
                params![state.view.connection_id, session_id],
            )?;
        }
        tx.commit()?;
        drop(conn);
        changed();
        Ok(())
    }

    pub fn assistant_im_pending_commands(&self) -> Result<Vec<ReceivedCommand>> {
        let conn = self.read_conn()?;
        let Some(state) = load(&conn)? else {
            return Ok(Vec::new());
        };
        if state.view.state != PairingState::Active {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT request_id, text, grant_revisions, global_stop_epoch, home_stop_epoch, target_stop_epochs, source_view_id, view_stop_epoch FROM assistant_im_commands
             WHERE connection_id = ?1 AND generation = ?2 AND state = 'pending'
             ORDER BY created_at, rowid LIMIT 10",
        )?;
        let commands = stmt
            .query_map(
                params![state.view.connection_id, state.view.generation],
                |row| {
                    Ok(ReceivedCommand {
                        request_id: row.get(0)?,
                        connection_id: state.view.connection_id.clone(),
                        generation: state.view.generation,
                        text: row.get(1)?,
                        grant_revisions: serde_json::from_str(&row.get::<_, String>(2)?).map_err(
                            |error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    2,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            },
                        )?,
                        global_stop_epoch: row.get::<_, i64>(3)?.max(0) as u64,
                        home_stop_epoch: row.get::<_, i64>(4)?.max(0) as u64,
                        target_stop_epochs: serde_json::from_str(&row.get::<_, String>(5)?)
                            .map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    5,
                                    rusqlite::types::Type::Text,
                                    Box::new(error),
                                )
                            })?,
                        source_view_id: row.get(6)?,
                        view_stop_epoch: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(commands)
    }

    pub fn assistant_im_commands_awaiting_reply(&self) -> Result<Vec<ReceivedCommand>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT request_id FROM assistant_im_commands WHERE state = 'reply_pending' ORDER BY created_at LIMIT 10",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut commands = Vec::new();
        for id in ids {
            if let Some(command) = read_command(&conn, &id)? {
                commands.push(command);
            }
        }
        Ok(commands)
    }

    pub fn complete_assistant_im_command(&self, request_id: &str) -> Result<()> {
        self.with_conn_internal(|conn| {
            conn.execute(
                "UPDATE assistant_im_commands SET state = 'completed', text = ''
                 WHERE request_id = ?1 AND state IN ('pending','running')",
                [request_id],
            )?;
            Ok(())
        })
    }

    pub fn set_assistant_im_notifications_paused(&self, paused: bool) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AssistantImError::Unavailable)?;
        let tx = conn.transaction()?;
        let mut state = load(&tx)?.ok_or(AssistantImError::Unavailable)?;
        if state.view.state != PairingState::Active {
            return Err(AssistantImError::Unavailable.into());
        }
        state.view.notifications_paused = paused;
        save(&tx, &state)?;
        if paused {
            tx.execute(
                "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
                 WHERE connection_id = ?1 AND completion_notice = 1 AND state = 'pending'",
                [&state.view.connection_id],
            )?;
        }
        tx.commit()?;
        drop(conn);
        changed();
        Ok(())
    }
}

fn consume_input(
    tx: &Transaction<'_>,
    state: &mut Connection,
    input: TelegramInput,
    now: i64,
) -> Result<IngressOutcome> {
    state.cursor = state.cursor.max(input.update_id + 1);
    if input.user_id <= 0 || input.chat_id <= 0 || input.user_id != input.chat_id {
        return Ok(IngressOutcome::Ignored);
    }
    match input.kind {
        TelegramInputKind::PairCode(code) => {
            if state.view.state != PairingState::AwaitingCode
                || state.pair_expires_at <= now
                || state.pair_attempts >= 5
            {
                return Ok(IngressOutcome::Ignored);
            }
            state.pair_attempts += 1;
            if state.pair_hash.as_deref() != Some(hash(&code).as_str()) {
                return Ok(IngressOutcome::Ignored);
            }
            state.pair_hash = None;
            state.view.user_id = Some(input.user_id);
            state.view.chat_id = Some(input.chat_id);
            state.view.state = PairingState::AwaitingConfirmation;
            Ok(IngressOutcome::PairingCandidate)
        }
        TelegramInputKind::Activate(token) => {
            if state.view.state != PairingState::AwaitingActivation
                || state.activation_expires_at <= now
                || state.view.user_id != Some(input.user_id)
                || state.view.chat_id != Some(input.chat_id)
                || state.activation_hash.as_deref() != Some(hash(&token).as_str())
            {
                return Ok(IngressOutcome::Ignored);
            }
            state.activation_hash = None;
            state.view.state = PairingState::Active;
            Ok(IngressOutcome::Activated)
        }
        TelegramInputKind::Message(text) => {
            if state.view.state != PairingState::Active
                || state.view.user_id != Some(input.user_id)
                || state.view.chat_id != Some(input.chat_id)
                || text.is_empty()
                || text.len() > MAX_MESSAGE_BYTES
                || !input.sent_at.is_some_and(|sent_at| {
                    sent_at >= now.saturating_sub(300) && sent_at <= now.saturating_add(30)
                })
            {
                return Ok(IngressOutcome::Ignored);
            }
            if text.trim() == "/ha_unpair" {
                invalidate(tx, &state.view.connection_id)?;
                reset_pairing(state);
                return Ok(IngressOutcome::Revoked);
            }
            if text.trim() == "/ha_pause" {
                state.view.notifications_paused = true;
                tx.execute(
                    "UPDATE assistant_im_deliveries SET state = 'cancelled', text = ''
                     WHERE connection_id = ?1 AND completion_notice = 1 AND state = 'pending'",
                    [&state.view.connection_id],
                )?;
                return Ok(IngressOutcome::Paused);
            }
            let pending: i64 = tx.query_row(
                "SELECT count(*) FROM assistant_im_commands WHERE state IN ('pending','running')",
                [],
                |row| row.get(0),
            )?;
            if pending >= 64 {
                return Ok(IngressOutcome::Ignored);
            }
            let mut stmt =
                tx.prepare("SELECT grant_json FROM assistant_im_grants WHERE connection_id = ?1")?;
            let rows =
                stmt.query_map([&state.view.connection_id], |row| row.get::<_, String>(0))?;
            let mut revisions = std::collections::BTreeMap::new();
            let mut target_epochs = std::collections::BTreeMap::new();
            for row in rows {
                let permission: SessionGrant = serde_json::from_str(&row?)?;
                if permission.query_status
                    && permission.expires_at > now
                    && eligible_session(tx, &permission.session_id)?
                {
                    target_epochs.insert(
                        permission.session_id.clone(),
                        session_stop_epoch(tx, &permission.session_id)?,
                    );
                    revisions.insert(permission.session_id, permission.revision);
                }
            }
            let scope: Vec<_> = revisions.iter().collect();
            let source_view_id: Option<String> = tx.query_row(
                "SELECT session_id FROM assistant_im_views WHERE connection_id = ?1 AND generation = ?2 AND scope_key = ?3",
                params![state.view.connection_id,state.view.generation,serde_json::to_string(&scope)?],
                |row| row.get(0),
            ).optional()?;
            let view_epoch = match &source_view_id {
                Some(id) => session_stop_epoch(tx, id)?,
                None => 0,
            };
            tx.execute(
                "INSERT INTO assistant_im_commands(request_id, connection_id, generation, text, grant_revisions,
                 global_stop_epoch, home_stop_epoch, target_stop_epochs, source_view_id, view_stop_epoch, state, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
                params![uuid::Uuid::new_v4().to_string(), state.view.connection_id,
                    state.view.generation, text, serde_json::to_string(&revisions)?,
                    current_global_epoch(tx)?, current_home_epoch(tx)?, serde_json::to_string(&target_epochs)?, source_view_id, view_epoch, now],
            )?;
            Ok(IngressOutcome::Queued)
        }
        TelegramInputKind::Ignore => Ok(IngressOutcome::Ignored),
    }
}
