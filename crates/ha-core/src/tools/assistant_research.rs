//! Body-free Home references/query and the sealed private report tool.
use crate::tool_defs::ToolExecContext;
use anyhow::{anyhow, bail, Result};
use serde_json::Value;

pub(super) async fn query(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito || ctx.turn_provenance != crate::tool_defs::ToolTurnProvenance::ForegroundUser
    {
        bail!("A live owner Hope turn is required");
    }
    if !crate::personal_assistant::research::coordinator_registered() {
        bail!("The assistant research coordinator is unavailable");
    }
    let db = ctx
        .session_db
        .as_ref()
        .ok_or_else(|| anyhow!("Database unavailable"))?
        .0
        .clone();
    let turn = ctx
        .turn_id
        .clone()
        .ok_or_else(|| anyhow!("Owner turn identity is required"))?;
    let epoch = ctx
        .turn_admitted_global_stop_epoch
        .ok_or_else(|| anyhow!("Owner Stop receipt is required"))?;
    let owner_turn = turn.clone();
    db.run(move |db| {
        let conn = db.read_conn()?;
        crate::personal_assistant::source_turn(&conn, &owner_turn, epoch).map(|_| ())
    })
    .await?;
    crate::personal_assistant::research::refresh_owner_sources(db.clone()).await?;
    let action = args.get("action").and_then(Value::as_str).unwrap_or("list");
    if action == "list" {
        let refs = db
            .run(move |db| {
                let conn = db.read_conn()?;
                crate::personal_assistant::source_turn(&conn, &turn, epoch)?;
                drop(conn);
                db.assistant_finding_references_for_model(chrono::Utc::now().timestamp())
            })
            .await?;
        return Ok(serde_json::json!({"references":refs,"instruction":"Research bodies stay in private source-scoped updates. To ask about a reference, queue an owner question with its exact ID/revision; the answer returns as a new update, never ordinary Home history."}).to_string());
    }
    if action != "query" {
        bail!("Expected action=list or query");
    }
    let id = args
        .get("finding_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("A finding reference is required"))?
        .to_owned();
    let revision = args
        .get("revision")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("A finding version is required"))?;
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("A current owner question is required"))?
        .to_owned();
    let call = ctx
        .tool_call_id
        .clone()
        .ok_or_else(|| anyhow!("A live tool call identity is required"))?;
    let query = db
        .run(move |db| {
            db.request_assistant_research_from_turn(
                &id,
                revision,
                &question,
                &turn,
                &call,
                epoch,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?;
    Ok(serde_json::json!({"status":query.state,"requestId":query.id,"reason":query.reason,"delivery":"source_scoped_update","instruction":"Do not claim the research is complete or invent the source contents. Its result returns through a current-source update."}).to_string())
}

pub(super) async fn report(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito {
        bail!("Research is unavailable in incognito");
    }
    let db = ctx
        .session_db
        .as_ref()
        .ok_or_else(|| anyhow!("Database unavailable"))?
        .0
        .clone();
    let execution = ctx
        .persistence_run_id
        .clone()
        .ok_or_else(|| anyhow!("An exact research execution is required"))?;
    let findings: Vec<crate::personal_assistant::research::ResearchFindingInput> =
        serde_json::from_value(
            args.get("findings")
                .cloned()
                .ok_or_else(|| anyhow!("A findings array is required"))?,
        )?;
    // Fresh source checks occur before this durable proposal is recorded.
    // Publication still waits for standard stream commit and reconciliation.
    crate::personal_assistant::research::refresh_execution_sources(db.clone(), execution.clone())
        .await?;
    db.run(move |db| db.report_assistant_research(&execution, findings))
        .await?;
    Ok(serde_json::json!({"status":"recorded","instruction":"End this turn. The host will verify current sources and the committed execution before publishing an update."}).to_string())
}
