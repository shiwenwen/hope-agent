use super::*;

/// Returns token + active-time reservation. The caller constructs the common
/// lease; unknown termination deliberately leaves this ledger occupied.
pub(crate) fn reserve_operation(
    db: &SessionDB,
    execution: &str,
    model: bool,
    tool_key: Option<&str>,
) -> Result<Option<(String, u64)>> {
    let now = chrono::Utc::now().timestamp();
    let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = validate_execution(&tx, execution, now)? else {
        return Ok(None);
    };
    let policy = super::super::policy::current(&tx)?
        .ok_or_else(|| anyhow!("Assistant policy is pending"))?;
    if policy.settings.paused {
        bail!("The assistant is paused");
    }
    let (state,models,tools,active):(String,i64,i64,i64)=tx.query_row("SELECT state,models_used,tools_used,active_seconds FROM assistant_research_runs WHERE id=?1",[&run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if state != "running" {
        bail!("Research already handed control back");
    }
    let token = if model {
        uuid::Uuid::new_v4().to_string()
    } else {
        format!(
            "research:{execution}:{}",
            tool_key.ok_or_else(|| anyhow!("Tool identity required"))?
        )
    };
    let previous: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_research_reservations WHERE token=?1)",
        [&token],
        |r| r.get(0),
    )?;
    if previous {
        bail!("Research tool call was already allocated");
    }
    let (held,count):(i64,i64)=tx.query_row("SELECT COALESCE(SUM(reserved_seconds),0),COUNT(*) FROM assistant_research_reservations WHERE run_id=?1 AND settled_at IS NULL",[&run],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count > 0 {
        bail!("A research operation has no observed termination yet");
    }
    let available = 90 - active - held;
    if available <= 0 || (model && models >= 2) || (!model && tools >= 8) {
        bail!("Research allowance exhausted");
    }
    if !model {
        tx.execute(
            "UPDATE assistant_research_runs SET tools_used=tools_used+1 WHERE id=?1",
            [&run],
        )?;
    }
    let seconds = if model { available } else { available.min(10) };
    tx.execute("INSERT INTO assistant_research_reservations(token,run_id,execution_id,started_at,reserved_seconds) VALUES(?1,?2,?3,?4,?5)",
        params![token,run,execution,now,seconds])?;
    tx.commit()?;
    Ok(Some((token, u64::try_from(seconds)?)))
}

pub(crate) fn settle_operation(conn: &Connection, token: &str, seconds: u64) -> Result<bool> {
    let row:Option<(String,i64,Option<i64>)>=conn.query_row("SELECT run_id,reserved_seconds,settled_at FROM assistant_research_reservations WHERE token=?1",[token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((run, reserved, settled)) = row else {
        return Ok(false);
    };
    if settled.is_none() {
        let charged = i64::try_from(seconds).unwrap_or(i64::MAX).min(reserved);
        conn.execute("UPDATE assistant_research_reservations SET charged_seconds=?2,settled_at=?3 WHERE token=?1 AND settled_at IS NULL",params![token,charged,chrono::Utc::now().timestamp()])?;
        conn.execute(
            "UPDATE assistant_research_runs SET active_seconds=active_seconds+?2 WHERE id=?1",
            params![run, charged],
        )?;
    }
    Ok(true)
}

/// Count actual prepared-provider dispatches, including repair/fallback attempts.
/// This records an attempt allocation, never estimated token usage.
pub async fn reserve_model_attempt(db: std::sync::Arc<SessionDB>, execution: String) -> Result<()> {
    super::refresh_execution_sources(db.clone(), execution.clone()).await?;
    db.run(move |db| {
        let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(run) = validate_execution(&tx, &execution, chrono::Utc::now().timestamp())? else {
            return Ok(());
        };
        let models: i64 = tx.query_row(
            "SELECT models_used FROM assistant_research_runs WHERE id=?1 AND state='running'",
            [&run],
            |r| r.get(0),
        )?;
        let now = chrono::Utc::now().timestamp();
        let owner: i64 = tx.query_row(
            "SELECT COUNT(*) FROM assistant_research_usage WHERE attempted_at>=?1",
            [now - 86_400],
            |r| r.get(0),
        )?;
        if models >= 2 || owner >= 8 {
            bail!("Research model attempt allowance exhausted")
        }
        tx.execute(
            "INSERT INTO assistant_research_usage(id,run_id,attempted_at) VALUES(?1,?2,?3)",
            params![uuid::Uuid::new_v4().to_string(), run, now],
        )?;
        tx.execute(
            "UPDATE assistant_research_runs SET models_used=models_used+1 WHERE id=?1",
            [&run],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
