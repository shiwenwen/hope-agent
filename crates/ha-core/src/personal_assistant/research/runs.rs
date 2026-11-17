use super::queries::{keys_current, keys_for_run, SourceKey};
use super::*;

fn input_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ResearchInput> {
    Ok(ResearchInput {
        source_id: r.get(0)?,
        revision: r.get(1)?,
        version: r.get(2)?,
        focus: r.get(3)?,
        text: r.get(4)?,
        previous_text: r.get(5)?,
        previous_version: r.get(6)?,
    })
}

fn source_inputs(conn: &Connection, keys: &[SourceKey]) -> Result<Vec<ResearchInput>> {
    let mut result = Vec::new();
    for key in keys {
        result.push(conn.query_row("SELECT id,revision,version,focus,body,previous_body,previous_version FROM assistant_sources WHERE id=?1 AND revision=?2 AND version=?3",
            params![key.source_id,key.revision,key.version],input_row)?);
    }
    Ok(result)
}

fn query_current(
    conn: &Connection,
    home: &str,
    global: i64,
    epoch: i64,
    keys: &[SourceKey],
    now: i64,
) -> Result<bool> {
    let eligible:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=?1 AND s.kind='personal_assistant' AND s.incognito=0
        AND ?2=(SELECT epoch FROM runtime_control_epochs WHERE key='global_stop')
        AND ?3=(SELECT COUNT(*) FROM session_autonomy_pauses WHERE session_id=s.id)
        AND NOT EXISTS(SELECT 1 FROM session_autonomy_pauses WHERE session_id=s.id AND resumed_at IS NULL))",
        params![home,global,epoch],|r|r.get(0))?;
    Ok(eligible && keys_current(conn, keys, now)?)
}

impl SessionDB {
    pub(crate) fn claim_assistant_research(&self, now: i64) -> Result<Option<PendingResearch>> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(policy) = super::super::policy::current(&tx)? else {
            return Ok(None);
        };
        if policy.settings.paused {
            return Ok(None);
        };
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_runs WHERE state IN('pending','running','reported'))",[],|r|r.get(0))?;
        if active {
            return Ok(None);
        };
        let owner_busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_stream_runs r JOIN sessions s ON s.id=r.session_id WHERE r.status='running' AND s.kind='personal_assistant')",[],|r|r.get(0))?;
        if owner_busy {
            return Ok(None);
        };
        let used: i64 = tx.query_row(
            "SELECT COUNT(*) FROM assistant_research_usage WHERE attempted_at>=?1",
            [now - 86_400],
            |r| r.get(0),
        )?;
        let questions = {
            let mut statement=tx.prepare("SELECT id,home_session_id,global_stop_epoch,home_stop_epoch,source_set_json,question,prior_finding,created_at
                FROM assistant_research_queries WHERE state='pending' ORDER BY created_at,id LIMIT 8")?;
            let rows = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut selected = None;
        let mut mutated = false;
        for (id, home, global, epoch, json, question, prior, created) in questions {
            let keys: Vec<SourceKey> = serde_json::from_str(&json)?;
            let reason = if created <= now - 86_400 {
                Some("expired")
            } else if !query_current(&tx, &home, global, epoch, &keys, now)?
                || !super::super::attention::replies::query_scope_current(&tx, &id, now)?
            {
                Some("scope_changed")
            } else if used >= 8 {
                Some("budget_exhausted")
            } else {
                None
            };
            if let Some(reason) = reason {
                tx.execute("UPDATE assistant_research_queries SET state='cancelled',reason=?2,question='',prior_finding=NULL WHERE id=?1",params![id,reason])?;
                mutated = true;
                continue;
            }
            selected = Some((home, global, epoch, keys, Some(id), Some(question), prior));
            break;
        }
        if selected.is_none() && policy.settings.research_enabled && used < 8 {
            let mut keys = Vec::new();
            let home: Option<String> = tx
                .query_row(
                    "SELECT id FROM sessions WHERE kind='personal_assistant' AND incognito=0",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(home) = home {
                let candidates = {
                    let mut statement=tx.prepare("SELECT id,revision,version,error=0 AND body IS NOT NULL FROM assistant_sources WHERE home_session_id=?1 AND state='active' AND expires_at>?2 ORDER BY id LIMIT 4")?;
                    let rows = statement
                        .query_map(params![home, now], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, Option<String>>(2)?,
                                r.get::<_, bool>(3)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                };
                let mut event = false;
                let mut gap = false;
                for (source_id, revision, version, healthy) in candidates {
                    if !source_autonomy_current(&tx, &source_id, now)? {
                        continue;
                    }
                    let Some(version) = version.filter(|_| healthy) else {
                        gap = true;
                        break;
                    };
                    if !source_current(&tx, &source_id, revision, &version, now)? {
                        gap = true;
                        break;
                    }
                    event|=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_source_events WHERE source_id=?1 AND revision=?2 AND version=?3 AND run_id IS NULL)",params![source_id,revision,version],|r|r.get::<_,bool>(0))?;
                    keys.push(SourceKey {
                        source_id,
                        revision,
                        version,
                    });
                }
                if event && !gap && !keys.is_empty() {
                    let global = tx.query_row(
                        "SELECT epoch FROM runtime_control_epochs WHERE key='global_stop'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )?;
                    let epoch = super::super::stop_epoch(&tx, &home)?;
                    selected = Some((home, global, epoch, keys, None, None, None));
                }
            }
        }
        let Some((home, global, epoch, keys, query, question, prior)) = selected else {
            if mutated {
                changed(&tx)?
            }
            tx.commit()?;
            drop(conn);
            if mutated {
                emit_research_hint(false)
            }
            return Ok(None);
        };
        let inputs = source_inputs(&tx, &keys)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let session_id = uuid::Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339();
        tx.execute("INSERT INTO sessions(id,title,agent_id,created_at,updated_at,kind) VALUES(?1,'Hope',?2,?3,?3,'personal_assistant_research')",params![session_id,crate::agent_loader::DEFAULT_AGENT_ID,timestamp])?;
        tx.execute(
            "INSERT INTO personal_assistant_private_sessions(session_id) VALUES(?1)",
            [&session_id],
        )?;
        tx.execute("INSERT INTO session_memory_policy(session_id,use_memories,contribute_to_memories,updated_at) VALUES(?1,'deny','deny',?2)",params![session_id,timestamp])?;
        tx.execute("INSERT INTO assistant_research_runs(id,session_id,home_session_id,kind,query_id,state,instance_id,policy_revision,global_stop_epoch,home_stop_epoch,created_at,question,prior_finding)
            VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,?8,?9,?10,?11,?12)",params![run_id,session_id,home,if query.is_some(){"owner_query"}else{"discovery"},query,instance_id(),policy.revision,global,epoch,now,question,prior])?;
        for input in inputs {
            tx.execute("INSERT INTO assistant_research_inputs(run_id,source_id,revision,version,focus,body,previous_body,previous_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![run_id,input.source_id,input.revision,input.version,input.focus,input.text,input.previous_text,input.previous_version])?;
            if query.is_none() {
                tx.execute("UPDATE assistant_source_events SET run_id=?1 WHERE source_id=?2 AND revision=?3 AND version=?4 AND run_id IS NULL",params![run_id,input.source_id,input.revision,input.version])?;
            }
        }
        if let Some(query) = query {
            tx.execute("UPDATE assistant_research_queries SET state='running',run_id=?2 WHERE id=?1 AND state='pending'",params![query,run_id])?;
        }
        changed(&tx)?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(false);
        Ok(Some(PendingResearch { run_id, session_id }))
    }

    pub(crate) fn research_proof(&self, run_id: &str) -> Result<ResearchProof> {
        let conn = self.read_conn()?;
        let valid:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_runs WHERE id=?1 AND state='pending' AND instance_id=?2)",params![run_id,instance_id()],|r|r.get(0))?;
        if !valid {
            bail!("Research claim is not available")
        }
        Ok(ResearchProof {
            run_id: run_id.to_owned(),
        })
    }

    pub(crate) fn admit_research_stream(
        &self,
        proof: &ResearchProof,
        create: &crate::session::CreateStreamRun,
    ) -> Result<(crate::session::StreamRunRegistration, String)> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let policy = super::super::policy::current(&tx)?
            .ok_or_else(|| anyhow!("Assistant policy is pending"))?;
        let kind: String = tx.query_row(
            "SELECT kind FROM assistant_research_runs WHERE id=?1",
            [&proof.run_id],
            |r| r.get(0),
        )?;
        if policy.settings.paused || (kind == "discovery" && !policy.settings.research_enabled) {
            bail!("Research is disabled")
        }
        let won=tx.execute("UPDATE assistant_research_runs SET state='running',execution_id=?2 WHERE id=?1 AND session_id=?3 AND state='pending' AND instance_id=?4 AND policy_revision=?5",params![proof.run_id,create.run_id,create.session_id,instance_id(),policy.revision])?;
        if won != 1 {
            bail!("Research claim changed before admission")
        }
        validate_run(&tx, &proof.run_id, chrono::Utc::now().timestamp())?;
        let inputs = {
            let mut statement=tx.prepare("SELECT source_id,revision,version,focus,body,previous_body,previous_version FROM assistant_research_inputs WHERE run_id=?1 ORDER BY source_id")?;
            let rows = statement
                .query_map([&proof.run_id], input_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let (question, prior): (Option<String>, Option<String>) = tx.query_row(
            "SELECT question,prior_finding FROM assistant_research_runs WHERE id=?1",
            [&proof.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let registration = Self::create_stream_run_in_transaction(&tx, create, None)?;
        let mut snapshots = Vec::new();
        for input in inputs {
            let mut source = serde_json::to_value(&input)?;
            let mut statement=tx.prepare("SELECT started_at,ended_at FROM assistant_source_gaps WHERE source_id=?1 ORDER BY started_at DESC LIMIT 3")?;
            let gaps=statement.query_map([&input.source_id],|r|Ok(serde_json::json!({"since":r.get::<_,i64>(0)?,"until":r.get::<_,Option<i64>>(1)?,"intermediateVersionsMayBeMissing":true})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            source["observationGaps"] = serde_json::json!(gaps);
            snapshots.push(source);
        }
        let data =
            serde_json::json!({"sources":snapshots,"ownerQuestion":question,"priorFinding":prior,"observedAtUtc":chrono::Utc::now().to_rfc3339(),"ownerTimezone":policy.settings.timezone})
                .to_string();
        tx.commit()?;
        Ok((registration, data))
    }

    pub fn report_assistant_research(
        &self,
        execution: &str,
        findings: Vec<ResearchFindingInput>,
    ) -> Result<()> {
        if findings.len() > 8 {
            bail!("Research reports contain at most eight findings")
        }
        let now = chrono::Utc::now().timestamp();
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = validate_execution(&tx, execution, now)?
            .ok_or_else(|| anyhow!("Sealed research execution required"))?;
        let kind: String = tx.query_row(
            "SELECT kind FROM assistant_research_runs WHERE id=?1",
            [&run],
            |r| r.get(0),
        )?;
        if kind == "owner_query" && findings.is_empty() {
            bail!("An owner question requires an evidence-backed answer or a clear uncertainty finding")
        }
        for finding in &findings {
            if let Some(until) = finding.attention_until.as_deref() {
                chrono::DateTime::parse_from_rfc3339(until)
                    .map_err(|_| anyhow!("attentionUntil must be an absolute RFC3339 timestamp"))?;
            }
            if finding.topic_key.is_empty()
                || finding.topic_key.len() > 64
                || !finding
                    .topic_key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                || finding.fact.trim().is_empty()
                || finding.fact.len() > 2_000
                || finding.inference.len() > 4_000
                || finding.uncertainty.len() > 2_000
                || finding.prepared.len() > 12_000
                || finding.references.is_empty()
                || finding.references.len() > 8
            {
                bail!("Invalid bounded research finding")
            }
            for reference in &finding.references {
                let text:Option<String>=tx.query_row("SELECT CASE WHEN version=?4 THEN body WHEN previous_version=?4 THEN previous_body END FROM assistant_research_inputs WHERE run_id=?1 AND source_id=?2 AND revision=?3",params![run,reference.source_id,reference.revision,reference.version],|r|r.get(0)).optional()?.flatten();
                let text = text.ok_or_else(|| {
                    anyhow!("Report references must match an actual supplied input version")
                })?;
                if reference.start_line == 0
                    || reference.end_line < reference.start_line
                    || usize::try_from(reference.end_line)? > text.lines().count().max(1)
                {
                    bail!("Report reference lines are outside the shared input")
                }
            }
        }
        let body = serde_json::to_string(&findings)?;
        if body.len() > 65_536 {
            bail!("Research report is too large")
        }
        let won=tx.execute("UPDATE assistant_research_runs SET state='reported',reported_json=?2 WHERE id=?1 AND state='running'",params![run,body])?;
        if won != 1 {
            bail!("Research already handed control back")
        }
        tx.commit()?;
        Ok(())
    }

    pub fn settle_assistant_research(&self, now: i64) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let terminal = {
            let mut statement=tx.prepare("SELECT r.id,r.reported_json,s.status,r.query_id FROM assistant_research_runs r JOIN chat_stream_runs s ON s.run_id=r.execution_id WHERE r.state IN('running','reported') AND s.status<>'running'")?;
            let rows = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mutated = !terminal.is_empty();
        for (run, body, status, query) in terminal {
            let reason = if status != "committed" {
                Some("execution_unverified")
            } else if !run_current(&tx, &run, now)? {
                Some("scope_changed")
            } else if body.is_none() {
                Some("missing_report")
            } else {
                None
            };
            if let Some(reason) = reason {
                tx.execute("UPDATE assistant_research_runs SET state='unknown',outcome_reason=?3,ended_at=?2,reported_json=NULL,question=NULL,prior_finding=NULL WHERE id=?1",params![run,now,reason])?;
                tx.execute("UPDATE assistant_research_queries SET state='failed',reason=?2,question='',prior_finding=NULL WHERE run_id=?1",params![run,reason])?;
                continue;
            }
            let findings: Vec<ResearchFindingInput> =
                serde_json::from_str(body.as_deref().unwrap_or("[]"))?;
            let scope = serde_json::to_string(
                &keys_for_run(&tx, &run)?
                    .iter()
                    .map(|key| &key.source_id)
                    .collect::<Vec<_>>(),
            )?;
            let timestamp = chrono::Utc::now().to_rfc3339();
            for finding in findings {
                let semantic = blake3::hash(
                    format!(
                        "{scope}:{}:{}",
                        query.as_deref().unwrap_or("discovery"),
                        finding.topic_key
                    )
                    .as_bytes(),
                )
                .to_hex()
                .to_string();
                let json = serde_json::to_string(&finding)?;
                let hash = blake3::hash(
                    serde_json::to_string(&(
                        &finding.fact,
                        &finding.inference,
                        &finding.uncertainty,
                        &finding.prepared,
                    ))?
                    .as_bytes(),
                )
                .to_hex()
                .to_string();
                tx.execute("INSERT INTO assistant_findings(id,semantic_key,run_id,revision,content_hash,body_json,state,created_at,updated_at) VALUES(?1,?2,?3,1,?4,?5,'active',?6,?6)
                    ON CONFLICT(semantic_key) DO UPDATE SET run_id=excluded.run_id,revision=assistant_findings.revision+CASE WHEN content_hash<>excluded.content_hash THEN 1 ELSE 0 END,
                    viewed_at=CASE WHEN content_hash<>excluded.content_hash THEN NULL ELSE viewed_at END,updated_at=CASE WHEN content_hash<>excluded.content_hash THEN excluded.updated_at ELSE updated_at END,
                    content_hash=excluded.content_hash,body_json=excluded.body_json,state='active'",params![uuid::Uuid::new_v4().to_string(),semantic,run,hash,json,timestamp])?;
            }
            tx.execute("UPDATE assistant_research_runs SET state='finished',ended_at=?2,reported_json=NULL,question=NULL,prior_finding=NULL WHERE id=?1",params![run,now])?;
            tx.execute("UPDATE assistant_research_queries SET state='finished',reason=NULL,question='',prior_finding=NULL WHERE run_id=?1",[&run])?;
        }
        if mutated {
            changed(&tx)?
        }
        tx.commit()?;
        drop(conn);
        if mutated {
            emit_research_hint(false)
        }
        Ok(())
    }

    pub fn list_assistant_findings(&self, now: i64) -> Result<Vec<AssistantFinding>> {
        let conn = self.read_conn()?;
        let mut statement=conn.prepare("SELECT id,revision,body_json,updated_at,viewed_at,run_id FROM assistant_findings WHERE state='active' ORDER BY updated_at DESC,id LIMIT 200")?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut findings = Vec::new();
        for (id, revision, body, created_at, viewed_at, run) in rows {
            if !input_set_current(&conn, &run, now)? {
                continue;
            }
            let body: ResearchFindingInput = serde_json::from_str(&body)?;
            findings.push(AssistantFinding {
                id,
                revision,
                fact: body.fact,
                inference: body.inference,
                uncertainty: body.uncertainty,
                prepared: body.prepared,
                references: body.references,
                created_at,
                viewed_at,
            });
        }
        findings.reverse();
        Ok(findings)
    }

    pub fn view_assistant_finding(&self, id: &str, revision: i64, now: i64) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run:Option<String>=tx.query_row("SELECT run_id FROM assistant_findings WHERE id=?1 AND revision=?2 AND state='active'",params![id,revision],|r|r.get(0)).optional()?;
        validate_input_set(
            &tx,
            &run.ok_or_else(|| anyhow!("Finding version is no longer available"))?,
            now,
        )?;
        tx.execute("UPDATE assistant_findings SET viewed_at=COALESCE(viewed_at,?3) WHERE id=?1 AND revision=?2 AND state='active'",params![id,revision,chrono::Utc::now().to_rfc3339()])?;
        changed(&tx)?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(false);
        Ok(())
    }
}
