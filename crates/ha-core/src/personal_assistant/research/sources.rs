use super::*;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum ReadAuthority {
    Owner,
    Autonomous,
    ResearchRun(String),
    Attention(String),
}

fn read_authorized(
    conn: &Connection,
    source: &str,
    revision: i64,
    authority: &ReadAuthority,
    now: i64,
) -> Result<bool> {
    let current:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_sources WHERE id=?1 AND revision=?2 AND state='active' AND expires_at>?3)",params![source,revision,now],|r|r.get(0))?;
    if !current {
        return Ok(false);
    }
    match authority {
        ReadAuthority::Owner=>Ok(true),
        ReadAuthority::Autonomous=>source_autonomy_current(conn,source,now),
        ReadAuthority::ResearchRun(run)=>Ok(run_current(conn,run,now)? && conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_inputs WHERE run_id=?1 AND source_id=?2 AND revision=?3)",params![run,source,revision],|r|r.get::<_,bool>(0))?),
        ReadAuthority::Attention(key)=>crate::personal_assistant::attention::read_scope_current(conn,key,source,revision,now),
    }
}

const COLUMNS: &str = "id,session_id,relative_path,focus,revision,version,state,expires_at,last_observed_at,gap_since,error";

fn row_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantSource> {
    Ok(AssistantSource {
        id: r.get(0)?,
        session_id: r.get(1)?,
        relative_path: r.get(2)?,
        focus: r.get(3)?,
        revision: r.get(4)?,
        version: r.get(5)?,
        state: r.get(6)?,
        expires_at: r.get(7)?,
        last_observed_at: r.get(8)?,
        gap_since: r.get(9)?,
        error: r.get(10)?,
        suspended: false,
        im_summary_allowed: false,
        im_disclosure_revision: 0,
    })
}

pub(super) fn eligible(conn: &Connection, session: &str) -> Result<()> {
    super::super::validate_target(conn, session)?;
    let evaluated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_turns WHERE session_id=?1 AND source='eval')
        OR EXISTS(SELECT 1 FROM chat_stream_runs WHERE session_id=?1 AND source='eval')",
        [session],
        |r| r.get(0),
    )?;
    if evaluated || crate::eval_context::context_for_session(session).is_some() {
        bail!("Persistent file observation requires durable evaluation provenance");
    }
    Ok(())
}

fn spec_hash(input: &AssistantSourceInput) -> Result<String> {
    if uuid::Uuid::parse_str(&input.request_key).is_err()
        || input.focus.trim().is_empty()
        || input.focus.len() > 2_000
        || input.relative_path.len() > 4_096
        || !(1..=30).contains(&input.duration_days)
    {
        bail!("Invalid source request, focus or expiry");
    }
    let path = std::path::Path::new(&input.relative_path);
    if input.relative_path.is_empty()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("Choose a relative file inside the selected workspace");
    }
    Ok(blake3::hash(
        serde_json::to_string(&(
            &input.session_id,
            &input.relative_path,
            input.focus.trim(),
            input.duration_days,
        ))?
        .as_bytes(),
    )
    .to_hex()
    .to_string())
}

fn receipt(conn: &Connection, input: &AssistantSourceInput) -> Result<Option<AssistantSource>> {
    let hash = spec_hash(input)?;
    let old:Option<(String,String,i64)>=conn.query_row("SELECT spec_hash,source_id,source_revision FROM assistant_source_grants WHERE request_key=?1",
        [&input.request_key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((original, id, revision)) = old else {
        return Ok(None);
    };
    if original != hash {
        bail!("Source request identity was reused differently")
    }
    let source:Option<AssistantSource>=conn.query_row(&format!("SELECT {COLUMNS} FROM assistant_sources WHERE id=?1 AND revision=?2 AND state='active' AND expires_at>?3"),
        params![id,revision,chrono::Utc::now().timestamp()],row_source).optional()?;
    Ok(Some(source.ok_or_else(|| {
        anyhow!("This source receipt was superseded or revoked")
    })?))
}

#[derive(Clone)]
pub(crate) struct ReadTarget {
    source: AssistantSource,
    workspace_key: String,
}

impl SessionDB {
    pub fn list_assistant_sources(&self) -> Result<Vec<AssistantSource>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM assistant_sources WHERE state='active' AND expires_at>?1
             UNION ALL SELECT {COLUMNS} FROM (SELECT {COLUMNS} FROM assistant_sources
                WHERE state<>'active' OR expires_at<=?1 ORDER BY id LIMIT 64)
             ORDER BY id"
        ))?;
        let mut sources = stmt
            .query_map([chrono::Utc::now().timestamp()], row_source)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for source in &mut sources {
            (source.im_summary_allowed, source.im_disclosure_revision) =
                crate::personal_assistant::attention::source_disclosure_status(
                    &conn,
                    &source.id,
                    source.revision,
                    chrono::Utc::now().timestamp(),
                )?;
            let paused:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_policy_gate WHERE singleton=1 AND json_extract(requested_json,'$.paused')=1)",[],|r|r.get(0))?;
            source.suspended = source.state == "active"
                && (paused
                    || !source_autonomy_current(
                        &conn,
                        &source.id,
                        chrono::Utc::now().timestamp(),
                    )?);
        }
        Ok(sources)
    }

    pub(crate) fn preflight_assistant_source(
        &self,
        input: &AssistantSourceInput,
    ) -> Result<Option<AssistantSource>> {
        let conn = self.read_conn()?;
        eligible(&conn, &input.session_id)?;
        receipt(&conn, input)
    }

    pub(crate) fn save_assistant_source(
        &self,
        input: AssistantSourceInput,
        workspace_key: String,
        text: String,
        now: i64,
    ) -> Result<AssistantSource> {
        let hash = spec_hash(&input)?;
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        eligible(&tx, &input.session_id)?;
        if let Some(source) = receipt(&tx, &input)? {
            return Ok(source);
        };
        let home: String = tx.query_row(
            "SELECT id FROM sessions WHERE kind='personal_assistant' AND incognito=0",
            [],
            |r| r.get(0),
        )?;
        let epoch: i64 = tx.query_row(
            "SELECT epoch FROM runtime_control_epochs WHERE key='global_stop'",
            [],
            |r| r.get(0),
        )?;
        super::super::check_stop(&tx, &home, u64::try_from(epoch)?)?;
        let home_epoch = super::super::stop_epoch(&tx, &home)?;
        let old:Option<(String,i64,bool)>=tx.query_row("SELECT id,revision,state='active' AND expires_at>?3 FROM assistant_sources WHERE session_id=?1 AND relative_path=?2",
            params![input.session_id,input.relative_path,now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM assistant_sources WHERE state='active' AND expires_at>?1",
            [now],
            |r| r.get(0),
        )?;
        if !old.as_ref().is_some_and(|(_, _, active)| *active) && count >= MAX_SOURCES {
            bail!("Up to four local files may be shared")
        }
        let (id, revision) = old
            .map(|(id, revision, _)| (id, revision + 1))
            .unwrap_or_else(|| (uuid::Uuid::new_v4().to_string(), 1));
        quarantine_source(&tx, &id, now)?;
        let version = blake3::hash(text.as_bytes()).to_hex().to_string();
        tx.execute("INSERT INTO assistant_sources(id,home_session_id,session_id,relative_path,workspace_key,focus,revision,state,expires_at,
            version,body,global_stop_epoch,home_stop_epoch,next_scan_at,next_review_at,last_observed_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?9,?10,?11,?12,?13,?14,?15)
            ON CONFLICT(id) DO UPDATE SET home_session_id=excluded.home_session_id,workspace_key=excluded.workspace_key,
            focus=excluded.focus,revision=excluded.revision,state='active',expires_at=excluded.expires_at,version=excluded.version,
            body=excluded.body,previous_body=NULL,previous_version=NULL,global_stop_epoch=excluded.global_stop_epoch,home_stop_epoch=excluded.home_stop_epoch,
            next_scan_at=excluded.next_scan_at,next_review_at=excluded.next_review_at,last_observed_at=excluded.last_observed_at,gap_since=NULL,error=0",
            params![id,home,input.session_id,input.relative_path,workspace_key,input.focus.trim(),revision,now+i64::from(input.duration_days)*86_400,
                version,text,epoch,home_epoch,now+SCAN_SECONDS,now+REVIEW_SECONDS,now])?;
        tx.execute("INSERT INTO assistant_source_grants(request_key,spec_hash,source_id,source_revision,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![input.request_key,hash,id,revision,now])?;
        tx.execute(
            "DELETE FROM assistant_source_events WHERE source_id=?1 AND run_id IS NULL",
            [&id],
        )?;
        let source = tx.query_row(
            &format!("SELECT {COLUMNS} FROM assistant_sources WHERE id=?1"),
            [&id],
            row_source,
        )?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(true);
        Ok(source)
    }

    pub fn revoke_assistant_source(&self, id: &str, now: i64) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE assistant_sources SET state='revoked',revision=revision+1,body=NULL,previous_body=NULL,previous_version=NULL WHERE id=?1 AND state='active'",[id])?;
        quarantine_source(&tx, id, now)?;
        tx.execute(
            "DELETE FROM assistant_source_events WHERE source_id=?1 AND run_id IS NULL",
            [id],
        )?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(true);
        Ok(())
    }

    pub(crate) fn sources_due_for_scan(&self, now: i64) -> Result<Vec<ReadTarget>> {
        let conn = self.read_conn()?;
        let mut stmt=conn.prepare(&format!("SELECT {COLUMNS},workspace_key FROM assistant_sources WHERE state='active' AND (next_scan_at<=?1 OR expires_at<=?1) ORDER BY id LIMIT 4"))?;
        let rows = stmt
            .query_map([now], |r| {
                Ok(ReadTarget {
                    source: row_source(r)?,
                    workspace_key: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub(crate) fn source_targets_for_owner_read(&self) -> Result<Vec<ReadTarget>> {
        let conn = self.read_conn()?;
        let mut stmt=conn.prepare(&format!("SELECT {COLUMNS},workspace_key FROM assistant_sources WHERE state='active' AND expires_at>?1 ORDER BY id LIMIT 4"))?;
        let rows = stmt
            .query_map([chrono::Utc::now().timestamp()], |r| {
                Ok(ReadTarget {
                    source: row_source(r)?,
                    workspace_key: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub(crate) fn observe_assistant_source(
        &self,
        source: AssistantSource,
        read: Option<(String, String)>,
        now: i64,
        authority: ReadAuthority,
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot:Option<(String,i64,String,Option<String>,i64,i64,bool,Option<i64>)>=tx.query_row(
            "SELECT state,revision,workspace_key,version,expires_at,next_review_at,error,last_observed_at FROM assistant_sources WHERE id=?1",[&source.id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let Some((
            state,
            revision,
            workspace,
            old_version,
            expires,
            review,
            old_error,
            last_observed,
        )) = snapshot
        else {
            return Ok(());
        };
        if state != "active"
            || revision != source.revision
            || !read_authorized(&tx, &source.id, revision, &authority, now)?
        {
            return Ok(());
        }
        let meaningful;
        if expires <= now || !scope_available(eligible(&tx, &source.session_id))? {
            meaningful = true;
            tx.execute("UPDATE assistant_sources SET state='revoked',revision=revision+1,body=NULL,previous_body=NULL,previous_version=NULL WHERE id=?1",[&source.id])?;
            quarantine_source(&tx, &source.id, now)?;
        } else if let Some((_, text)) = read.filter(|(key, _)| key == &workspace) {
            let version = blake3::hash(text.as_bytes()).to_hex().to_string();
            if let Some(previous) =
                last_observed.filter(|previous| now.saturating_sub(*previous) > SCAN_SECONDS * 2)
            {
                tx.execute("INSERT OR IGNORE INTO assistant_source_gaps(source_id,started_at,ended_at) VALUES(?1,?2,?3)",params![source.id,previous,now])?;
            }
            let baseline = old_version.is_none();
            let material = old_version.as_deref() != Some(version.as_str());
            meaningful = material || old_error;
            if material && !baseline {
                quarantine_source(&tx, &source.id, now)?
            }
            tx.execute("UPDATE assistant_sources SET previous_body=CASE WHEN version<>?2 THEN body ELSE previous_body END,
                previous_version=CASE WHEN version<>?2 THEN version ELSE previous_version END,body=?3,version=?2,last_observed_at=?4,
                next_scan_at=?5,gap_since=NULL,error=0 WHERE id=?1",params![source.id,version,text,now,now+SCAN_SECONDS])?;
            tx.execute("UPDATE assistant_source_gaps SET ended_at=?2 WHERE source_id=?1 AND ended_at IS NULL",params![source.id,now])?;
            if baseline {
                tx.execute(
                    "UPDATE assistant_sources SET next_review_at=?2 WHERE id=?1",
                    params![source.id, now + REVIEW_SECONDS],
                )?;
            } else if material || review <= now {
                let kind = if material { "change" } else { "review" };
                // Each actual observation is a new event, even when its body
                // returns to an earlier content hash. The transaction's version
                // comparison already coalesces simultaneous identical reads.
                let event = uuid::Uuid::new_v4().to_string();
                tx.execute("INSERT OR IGNORE INTO assistant_source_events(id,source_id,revision,version,kind,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![event,source.id,revision,version,kind,now])?;
                tx.execute(
                    "UPDATE assistant_sources SET next_review_at=?2 WHERE id=?1",
                    params![source.id, now + REVIEW_SECONDS],
                )?;
            }
        } else {
            meaningful = !old_error;
            if meaningful {
                quarantine_source(&tx, &source.id, now)?
            }
            let gap = source.gap_since.or(source.last_observed_at).unwrap_or(now);
            tx.execute("UPDATE assistant_sources SET error=1,gap_since=COALESCE(gap_since,?2),next_scan_at=?3 WHERE id=?1",params![source.id,gap,now+SCAN_SECONDS])?;
            tx.execute(
                "INSERT OR IGNORE INTO assistant_source_gaps(source_id,started_at) VALUES(?1,?2)",
                params![source.id, gap],
            )?;
        }
        if meaningful {
            changed(&tx)?
        }
        tx.commit()?;
        drop(conn);
        if meaningful {
            emit_research_hint(true)
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ReadCancelled;
impl std::fmt::Display for ReadCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Source read authority changed")
    }
}
impl std::error::Error for ReadCancelled {}

#[derive(Debug)]
pub(super) struct WorkspaceChanged;
impl std::fmt::Display for WorkspaceChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared workspace identity changed; share the file again")
    }
}
impl std::error::Error for WorkspaceChanged {}

fn reader_slots() -> &'static Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
}

async fn bounded_read(
    db: Arc<SessionDB>,
    session: String,
    relative: String,
    key: Option<String>,
    grant: Option<(String, i64, ReadAuthority)>,
) -> Result<(String, String)> {
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader_slots().clone().acquire_owned(),
    )
    .await
    .map_err(|_| anyhow!("A source read is still in progress"))?
    .map_err(|_| anyhow!("Source reader is unavailable"))?;
    let read = crate::blocking::run_blocking(move || {
        let _permit = permit;
        if let Some((source, revision, authority)) = grant {
            let conn = db.read_conn()?;
            if !read_authorized(
                &conn,
                &source,
                revision,
                &authority,
                chrono::Utc::now().timestamp(),
            )? {
                return Err(anyhow!(ReadCancelled));
            }
        }
        read_shared_file(&db, &session, &relative, key.as_deref())
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), read)
        .await
        .map_err(|_| anyhow!("Source read could not be verified in time"))?
}

pub async fn create_source(
    db: Arc<SessionDB>,
    mut input: AssistantSourceInput,
) -> Result<AssistantSource> {
    spec_hash(&input)?;
    input.relative_path = std::path::Path::new(&input.relative_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    let check = input.clone();
    if let Some(source) = db
        .run(move |db| db.preflight_assistant_source(&check))
        .await?
    {
        return Ok(source);
    }
    db.run(|db| db.ensure_personal_assistant_home()).await?;
    let (key, text) = bounded_read(
        db.clone(),
        input.session_id.clone(),
        input.relative_path.clone(),
        None,
        None,
    )
    .await?;
    db.run(move |db| db.save_assistant_source(input, key, text, chrono::Utc::now().timestamp()))
        .await
}

async fn observe_target(
    db: Arc<SessionDB>,
    target: ReadTarget,
    authority: ReadAuthority,
) -> Result<()> {
    let source = target.source;
    let check = source.clone();
    let now = chrono::Utc::now().timestamp();
    let preflight = authority.clone();
    let permitted = db
        .run(move |db| {
            let conn = db.read_conn()?;
            if !read_authorized(&conn, &check.id, check.revision, &preflight, now)? {
                return Ok(false);
            }
            scope_available(eligible(&conn, &check.session_id))
        })
        .await?;
    if !permitted || source.expires_at <= now {
        let id = source.id;
        if source.expires_at <= now || matches!(authority, ReadAuthority::Owner) {
            db.run(move |db| db.revoke_assistant_source(&id, now))
                .await?;
        }
        return Ok(());
    }
    let read = bounded_read(
        db.clone(),
        source.session_id.clone(),
        source.relative_path.clone(),
        Some(target.workspace_key),
        Some((source.id.clone(), source.revision, authority.clone())),
    )
    .await;
    if read
        .as_ref()
        .err()
        .is_some_and(|error| error.downcast_ref::<WorkspaceChanged>().is_some())
    {
        let id = source.id;
        db.run(move |db| db.revoke_assistant_source(&id, now))
            .await?;
        return Ok(());
    }
    db.run(move |db| {
        db.observe_assistant_source(source, read.ok(), chrono::Utc::now().timestamp(), authority)
    })
    .await
}

pub async fn refresh_owner_sources(db: Arc<SessionDB>) -> Result<()> {
    for target in db.run(|db| db.source_targets_for_owner_read()).await? {
        observe_target(db.clone(), target, ReadAuthority::Owner).await?;
    }
    Ok(())
}

pub async fn scan_sources(db: Arc<SessionDB>) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let policy = db
        .run(|db| {
            let conn = db.read_conn()?;
            super::super::policy::current(&conn)
        })
        .await?;
    // Expiry still closes scope while discovery is disabled or paused.
    let targets = db.run(move |db| db.sources_due_for_scan(now)).await?;
    for target in targets {
        if target.source.expires_at <= now {
            let id = target.source.id;
            db.run(move |db| db.revoke_assistant_source(&id, now))
                .await?;
        } else if policy
            .as_ref()
            .is_some_and(|p| p.settings.research_enabled && !p.settings.paused)
        {
            observe_target(db.clone(), target, ReadAuthority::Autonomous).await?;
        }
    }
    Ok(())
}

/// Background reads retain their own sealed run, including owner-query receipts.
pub(crate) async fn refresh_research_sources(db: Arc<SessionDB>, run: String) -> Result<()> {
    let identity = run.clone();
    let targets=db.run(move|db| ->Result<Vec<ReadTarget>> {
        let conn=db.read_conn()?;
        validate_run(&conn,&identity,chrono::Utc::now().timestamp())?;
        let mut statement=conn.prepare(&format!("SELECT {COLUMNS},workspace_key FROM assistant_sources WHERE id IN(SELECT source_id FROM assistant_research_inputs WHERE run_id=?1) ORDER BY id LIMIT 4"))?;
        let rows=statement.query_map([identity],|r|Ok(ReadTarget{source:row_source(r)?,workspace_key:r.get(11)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    for target in targets {
        observe_target(db.clone(), target, ReadAuthority::ResearchRun(run.clone())).await?;
    }
    Ok(())
}

pub(crate) async fn refresh_execution_sources(db: Arc<SessionDB>, execution: String) -> Result<()> {
    let run = db
        .run(move |db| -> Result<Option<String>> {
            let conn = db.read_conn()?;
            Ok(conn
                .query_row(
                    "SELECT id FROM assistant_research_runs WHERE execution_id=?1",
                    [execution],
                    |r| r.get(0),
                )
                .optional()?)
        })
        .await?;
    if let Some(run) = run {
        refresh_research_sources(db, run).await?;
    }
    Ok(())
}

/// A notification has its own sealed read context after research has finished.
/// It cannot borrow the owner's UI permission or a completed research lease.
pub(crate) async fn refresh_attention_sources(db: Arc<SessionDB>, candidate: String) -> Result<()> {
    let receipt = candidate.clone();
    let targets=db.run(move|db| ->Result<Vec<ReadTarget>> {
        let conn=db.read_conn()?;
        let set:Option<String>=conn.query_row("SELECT source_set_json FROM assistant_attention_candidates WHERE logical_key=?1",[&receipt],|row|row.get(0)).optional()?.flatten();
        let Some(set)=set else {return Ok(Vec::new())};
        let keys:Vec<super::queries::SourceKey>=serde_json::from_str(&set)?;
        if keys.is_empty() || keys.len()>4 {bail!("A current source snapshot is required");}
        let mut targets=Vec::new();
        for key in keys {
            if !crate::personal_assistant::attention::read_scope_current(&conn,&receipt,&key.source_id,key.revision,chrono::Utc::now().timestamp())? {return Ok(Vec::new());}
            let target=conn.query_row(&format!("SELECT {COLUMNS},workspace_key FROM assistant_sources WHERE id=?1 AND revision=?2 AND version=?3"),params![key.source_id,key.revision,key.version],|row|Ok(ReadTarget{source:row_source(row)?,workspace_key:row.get(11)?})).optional()?;
            let Some(target)=target else {return Ok(Vec::new())};targets.push(target);
        }
        Ok(targets)
    }).await?;
    for target in targets {
        observe_target(
            db.clone(),
            target,
            ReadAuthority::Attention(candidate.clone()),
        )
        .await?;
    }
    Ok(())
}
