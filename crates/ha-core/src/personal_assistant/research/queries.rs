use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceKey {
    pub source_id: String,
    pub revision: i64,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchQueryReceipt {
    pub id: String,
    pub state: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchActivity {
    pub id: String,
    pub finding_id: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub created_at: i64,
}

pub(crate) fn migrate_queries(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_research_queries(
        id TEXT PRIMARY KEY,origin_key TEXT NOT NULL UNIQUE,
        home_session_id TEXT NOT NULL,finding_id TEXT,finding_revision INTEGER,
        source_run_id TEXT,source_set_json TEXT NOT NULL,prior_finding TEXT,
        question TEXT NOT NULL,spec_hash TEXT NOT NULL,state TEXT NOT NULL,reason TEXT,
        global_stop_epoch INTEGER NOT NULL,home_stop_epoch INTEGER NOT NULL,
        created_at INTEGER NOT NULL,run_id TEXT
    );",
    )?;
    Ok(())
}

pub(crate) fn keys_for_run(conn: &Connection, run: &str) -> Result<Vec<SourceKey>> {
    let mut stmt=conn.prepare("SELECT source_id,revision,version FROM assistant_research_inputs WHERE run_id=?1 ORDER BY source_id")?;
    let keys = stmt
        .query_map([run], |r| {
            Ok(SourceKey {
                source_id: r.get(0)?,
                revision: r.get(1)?,
                version: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(keys)
}

pub(crate) fn keys_current(conn: &Connection, keys: &[SourceKey], now: i64) -> Result<bool> {
    if keys.is_empty() || keys.len() > 4 {
        return Ok(false);
    }
    for key in keys {
        if !source_current(conn, &key.source_id, key.revision, &key.version, now)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn input_set_current(conn: &Connection, run: &str, now: i64) -> Result<bool> {
    let keys = keys_for_run(conn, run)?;
    if !keys_current(conn, &keys, now)? {
        return Ok(false);
    }
    Ok(!conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_research_inputs WHERE run_id=?1 AND body IS NULL)",
        [run],
        |r| r.get::<_, bool>(0),
    )?)
}

pub(crate) fn validate_input_set(conn: &Connection, run: &str, now: i64) -> Result<()> {
    if !input_set_current(conn, run, now)? {
        bail!("Research source is no longer current")
    }
    Ok(())
}

fn question_hash(question: &str, finding: Option<(&str, i64)>) -> Result<String> {
    Ok(
        blake3::hash(serde_json::to_string(&(question.trim(), finding))?.as_bytes())
            .to_hex()
            .to_string(),
    )
}

fn receipt(
    conn: &Connection,
    key: &str,
    question: &str,
    finding: Option<(&str, i64)>,
) -> Result<Option<ResearchQueryReceipt>> {
    let old: Option<(String, String, String, Option<String>)> = conn
        .query_row(
            "SELECT id,spec_hash,state,reason FROM assistant_research_queries WHERE origin_key=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((id, hash, state, reason)) = old else {
        return Ok(None);
    };
    if hash != question_hash(question, finding)? {
        bail!("Question identity was reused differently")
    }
    Ok(Some(ResearchQueryReceipt { id, state, reason }))
}

pub(crate) fn insert_question(
    conn: &Connection,
    key: &str,
    home: &str,
    question: &str,
    finding: Option<(&str, i64)>,
    source_run: Option<&str>,
    keys: &[SourceKey],
    prior: Option<&str>,
    now: i64,
) -> Result<ResearchQueryReceipt> {
    if question.trim().is_empty() || question.len() > 8_000 {
        bail!("A bounded owner question is required")
    }
    if let Some(old) = receipt(conn, key, question, finding)? {
        return Ok(old);
    }
    if !keys_current(conn, keys, now)? {
        bail!("The source version is no longer available")
    }
    let paused:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_policy_gate WHERE singleton=1 AND json_extract(requested_json,'$.paused')=1)",[],|r|r.get(0))?;
    if paused {
        bail!("Resume the assistant before starting another source question")
    }
    let epoch: i64 = conn.query_row(
        "SELECT epoch FROM runtime_control_epochs WHERE key='global_stop'",
        [],
        |r| r.get(0),
    )?;
    super::super::check_stop(conn, home, u64::try_from(epoch)?)?;
    let home_epoch = super::super::stop_epoch(conn, home)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM assistant_research_queries WHERE state IN('pending','running')",
        [],
        |r| r.get(0),
    )?;
    if count >= 8 {
        bail!("Too many pending owner source questions")
    }
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute("INSERT INTO assistant_research_queries(id,origin_key,home_session_id,finding_id,finding_revision,source_run_id,
        source_set_json,prior_finding,question,spec_hash,state,global_stop_epoch,home_stop_epoch,created_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending',?11,?12,?13)",
        params![id,key,home,finding.map(|(id,_)|id),finding.map(|(_,revision)|revision),source_run,
            serde_json::to_string(keys)?,prior,question.trim(),question_hash(question,finding)?,epoch,home_epoch,now])?;
    changed(conn)?;
    Ok(ResearchQueryReceipt {
        id,
        state: "pending".into(),
        reason: None,
    })
}

impl SessionDB {
    pub fn request_assistant_research_question(
        &self,
        id: &str,
        revision: i64,
        question: &str,
        request_key: &str,
        now: i64,
    ) -> Result<ResearchQueryReceipt> {
        if uuid::Uuid::parse_str(request_key).is_err() {
            bail!("A unique owner request identity is required")
        }
        self.insert_finding_question(
            id,
            revision,
            question,
            &format!("owner:{request_key}"),
            now,
            None,
        )
    }

    pub(crate) fn request_assistant_research_from_turn(
        &self,
        id: &str,
        revision: i64,
        question: &str,
        turn: &str,
        call: &str,
        epoch: u64,
        now: i64,
    ) -> Result<ResearchQueryReceipt> {
        if call.is_empty() || call.len() > 256 {
            bail!("A live owner tool call is required")
        }
        self.insert_finding_question(
            id,
            revision,
            question,
            &format!("turn:{turn}:{call}"),
            now,
            Some((turn, epoch)),
        )
    }

    fn insert_finding_question(
        &self,
        id: &str,
        revision: i64,
        question: &str,
        key: &str,
        now: i64,
        turn: Option<(&str, u64)>,
    ) -> Result<ResearchQueryReceipt> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = turn
            .map(|(turn, epoch)| super::super::source_turn(&tx, turn, epoch))
            .transpose()?;
        if let Some(old) = receipt(&tx, key, question, Some((id, revision)))? {
            return Ok(old);
        }
        let finding:Option<(String,String,String)>=tx.query_row("SELECT f.run_id,r.home_session_id,f.body_json FROM assistant_findings f
            JOIN assistant_research_runs r ON r.id=f.run_id WHERE f.id=?1 AND f.revision=?2 AND f.state='active'",params![id,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (run, home, body) =
            finding.ok_or_else(|| anyhow!("The finding is no longer available"))?;
        if owner.as_ref().is_some_and(|(source, _)| source != &home) {
            bail!("Question owner does not match the finding")
        }
        validate_input_set(&tx, &run, now)?;
        let keys = keys_for_run(&tx, &run)?;
        let receipt = insert_question(
            &tx,
            key,
            &home,
            question,
            Some((id, revision)),
            Some(&run),
            &keys,
            Some(&body),
            now,
        )?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(false);
        Ok(receipt)
    }

    pub fn request_assistant_source_review(
        &self,
        source: &str,
        request_key: &str,
        now: i64,
    ) -> Result<ResearchQueryReceipt> {
        if uuid::Uuid::parse_str(request_key).is_err() {
            bail!("A unique owner request identity is required")
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let key = format!("review:{source}:{request_key}");
        let question = "Review the owner's configured focus against this source now.";
        if let Some(old) = receipt(&tx, &key, question, None)? {
            return Ok(old);
        }
        let (home, revision, version): (String, i64, String) = tx.query_row(
            "SELECT home_session_id,revision,version FROM assistant_sources WHERE id=?1",
            [source],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let keys = [SourceKey {
            source_id: source.to_owned(),
            revision,
            version,
        }];
        let receipt = insert_question(&tx, &key, &home, question, None, None, &keys, None, now)?;
        tx.commit()?;
        drop(conn);
        emit_research_hint(false);
        Ok(receipt)
    }

    pub fn list_assistant_research_activity(&self) -> Result<Vec<ResearchActivity>> {
        let conn = self.read_conn()?;
        let mut stmt=conn.prepare("SELECT id,finding_id,state,reason,created_at FROM assistant_research_queries ORDER BY created_at DESC,id LIMIT 64")?;
        let mut rows = stmt
            .query_map([], |r| {
                Ok(ResearchActivity {
                    id: r.get(0)?,
                    finding_id: r.get(1)?,
                    state: r.get(2)?,
                    reason: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.reverse();
        Ok(rows)
    }

    pub(crate) fn assistant_finding_references_for_model(
        &self,
        now: i64,
    ) -> Result<serde_json::Value> {
        let findings = self.list_assistant_findings(now)?;
        Ok(serde_json::json!(findings
            .iter()
            .map(|f| serde_json::json!({"id":f.id,"revision":f.revision,"status":"available"}))
            .collect::<Vec<_>>()))
    }
}
