//! Projections for the sealed paired-IM context. No conversation bodies cross
//! this tool boundary; every lookup is intersected with the inbound authority.
use crate::{session::SessionDB, tools::ToolExecContext};
use anyhow::{anyhow, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) fn restricted(ctx: &ToolExecContext) -> bool {
    ctx.tool_scope == Some(crate::tool_defs::ToolScope::PersonalAssistantIm)
}

fn source(ctx: &ToolExecContext) -> Result<(Arc<SessionDB>, String, String)> {
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .ok_or_else(|| anyhow!("Restricted IM database unavailable"))?;
    Ok((
        db,
        ctx.session_id
            .clone()
            .ok_or_else(|| anyhow!("Missing IM view"))?,
        ctx.turn_id
            .clone()
            .ok_or_else(|| anyhow!("Missing IM turn"))?,
    ))
}

fn targets(db: &SessionDB, view: &str, turn_id: &str) -> Result<Vec<super::AssistantImTarget>> {
    let turn = db
        .get_chat_turn(turn_id)?
        .ok_or_else(|| anyhow!("IM turn unavailable"))?;
    if turn.session_id != view || turn.source != "channel" || turn.status.as_str() != "running" {
        anyhow::bail!("IM turn is no longer active");
    }
    db.assistant_im_turn_targets(view, turn_id, chrono::Utc::now().timestamp())
}

pub(crate) async fn list(ctx: &ToolExecContext) -> Result<String> {
    let (db, view, turn) = source(ctx)?;
    let rows = db.run(move |db| targets(db, &view, &turn)).await?;
    Ok(serde_json::to_string(&rows)?)
}

pub(crate) async fn status(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let target = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing session_id"))?
        .to_owned();
    let (db, view, turn) = source(ctx)?;
    let projection = db.run(move |db| -> Result<Value> {
        let permission = targets(db, &view, &turn)?.into_iter()
            .find(|item| item.session_id == target && item.query_status)
            .ok_or_else(|| anyhow!("Session status is unavailable to this IM view"))?;
        let conn = db.read_conn()?;
        let state: Option<String> = conn.query_row(
            "SELECT status FROM chat_turns WHERE session_id=?1 ORDER BY started_at DESC,id DESC LIMIT 1",
            [&target], |row| row.get(0),
        ).optional()?;
        let mut stmt = conn.prepare("SELECT r.id, CASE WHEN t.status IN ('completed','failed','interrupted') THEN t.status ELSE r.status END
            FROM personal_assistant_requests r LEFT JOIN chat_turns t ON t.id=r.turn_id AND t.session_id=r.target_session_id
            WHERE r.target_session_id=?1 ORDER BY r.created_at DESC LIMIT 20")?;
        let requests = stmt.query_map([&target], |row| Ok(json!({"id": row.get::<_,String>(0)?, "status": row.get::<_,String>(1)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"sessionId": target, "title": permission.title, "turnStatus": state, "requests": requests}))
    }).await?;
    Ok(serde_json::to_string(&projection)?)
}

pub(crate) async fn send(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if args.get("wait").and_then(Value::as_bool) == Some(true)
        || args
            .get("attachments")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.is_empty())
    {
        anyhow::bail!("IM delegation accepts asynchronous text instructions only");
    }
    let (db, _, turn) = source(ctx)?;
    let input = crate::personal_assistant::AssistantDispatchInput {
        source_turn_id: turn,
        item_key: ctx
            .tool_call_id
            .clone()
            .ok_or_else(|| anyhow!("Missing tool call identity"))?,
        target_session_id: args
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Missing session_id"))?
            .to_owned(),
        instruction: args
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Missing message"))?
            .to_owned(),
        global_stop_epoch: ctx
            .turn_admitted_global_stop_epoch
            .ok_or_else(|| anyhow!("Missing durable Stop admission"))?,
    };
    let request = db
        .run(move |db| db.enqueue_personal_assistant_request(input))
        .await?;
    Ok(
        json!({"id":request.id,"sessionId":request.target_session_id,"status":request.status})
            .to_string(),
    )
}

pub(crate) async fn cancel(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if args.get("kind").and_then(Value::as_str) != Some("assistant_request") {
        anyhow::bail!("Only an exact assistant request can be cancelled through this IM view");
    }
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing request id"))?
        .to_owned();
    let (db, view, turn) = source(ctx)?;
    let outcome = db
        .run(move |db| db.cancel_assistant_im_request(&view, &turn, &id))
        .await?;
    if let Some(turn_id) = outcome.cancel_turn_id.as_deref() {
        crate::chat_engine::active_turn::cancel_current(
            &outcome.request.target_session_id,
            Some(turn_id),
        );
    }
    Ok(json!({"id":outcome.request.id,"sessionId":outcome.request.target_session_id,"status":outcome.request.status}).to_string())
}
