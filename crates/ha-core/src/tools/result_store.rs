//! Bounded model-facing ResultStore readers.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

use crate::session::{
    ModelResultTextRead, ResultTextReadDirection, DEFAULT_RESULT_READ_BYTES, MAX_RESULT_READ_BYTES,
};
use crate::tool_defs::ToolExecContext;

const MAX_RESULT_READ_BYTES_PER_TURN: usize = 200 * 1024;
const RESULT_READ_BUDGET_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_TRACKED_TURN_BUDGETS: usize = 4_096;

static READ_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();
static READ_BUDGETS: OnceLock<Mutex<HashMap<(String, String), ReadBudget>>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct ReadBudget {
    used: usize,
    touched_at: Instant,
}

fn result_store_db(ctx: &ToolExecContext) -> Result<std::sync::Arc<crate::session::SessionDB>> {
    ctx.session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| anyhow!("ResultStore is unavailable"))
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{key} is required"))
}

pub(crate) async fn tool_result_meta(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("tool_result_meta requires a session"))?
        .to_string();
    let result_id = required_string(args, "result_id")?.to_string();
    let db = result_store_db(ctx)?;
    let access = db
        .run(move |db| db.get_model_result_metadata(&session_id, &result_id))
        .await?;
    Ok(serde_json::to_string(&access)?)
}

pub(crate) async fn tool_result_read(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("tool_result_read requires a session"))?
        .to_string();
    if !ctx
        .turn_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(anyhow!("tool_result_read requires a durable turn"));
    }
    let result_id = required_string(args, "result_id")?.to_string();
    let cursor = args
        .get("cursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    let direction = match args
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("forward")
    {
        "forward" => ResultTextReadDirection::Forward,
        "backward" => ResultTextReadDirection::Backward,
        _ => return Err(anyhow!("direction must be forward or backward")),
    };
    let requested = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_RESULT_READ_BYTES)
        .clamp(4, MAX_RESULT_READ_BYTES);
    let max_bytes = reserve_turn_read_budget(ctx, requested)?;

    let _permit = READ_SEMAPHORE
        .get_or_init(|| Semaphore::new(8))
        .acquire()
        .await
        .map_err(|_| anyhow!("ResultStore reader is shutting down"))?;
    let db = result_store_db(ctx)?;
    let read_session_id = session_id.clone();
    let read_result_id = result_id.clone();
    let read = db
        .run(move |db| {
            db.read_authorized_result_text_page(
                &read_session_id,
                &read_result_id,
                cursor.as_deref(),
                Some(max_bytes),
                direction,
            )
        })
        .await?;

    match read {
        ModelResultTextRead::Denied(denial) => Ok(serde_json::to_string(&json!({
            "status": "denied",
            "reason": denial,
        }))?),
        ModelResultTextRead::Authorized(page) => {
            if ctx.metadata_sink.is_some() {
                ctx.emit_metadata(json!({
                    "kind": "result_read_view",
                    "sourceResultId": page.result_id,
                    "startByte": page.start_byte,
                    "endByte": page.end_byte,
                    "direction": direction.as_str(),
                }))
                .await;
            }
            let safe_source = escape_xml_attr(&page.result_id);
            let safe_text = escape_xml_text(&page.text);
            let continuation = page
                .next_cursor
                .as_deref()
                .map(|cursor| format!(" next_cursor=\"{}\"", escape_xml_attr(cursor)))
                .unwrap_or_default();
            Ok(format!(
                "<untrusted_external_data source=\"tool_result:{}\" start_byte=\"{}\" end_byte=\"{}\" total_bytes=\"{}\"{}>\n{}\n</untrusted_external_data>",
                safe_source,
                page.start_byte,
                page.end_byte,
                page.total_bytes,
                continuation,
                safe_text,
            ))
        }
    }
}

fn reserve_turn_read_budget(ctx: &ToolExecContext, requested: usize) -> Result<usize> {
    let (Some(session_id), Some(turn_id)) = (ctx.session_id.as_ref(), ctx.turn_id.as_ref()) else {
        return Err(anyhow!(
            "tool_result_read requires a session and durable turn"
        ));
    };
    let now = Instant::now();
    let budgets = READ_BUDGETS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut budgets = budgets
        .lock()
        .map_err(|_| anyhow!("ResultStore read budget lock poisoned"))?;
    if budgets.len() >= MAX_TRACKED_TURN_BUDGETS {
        budgets.retain(|_, budget| now.duration_since(budget.touched_at) <= RESULT_READ_BUDGET_TTL);
    }
    let budget = budgets
        .entry((session_id.clone(), turn_id.clone()))
        .or_insert(ReadBudget {
            used: 0,
            touched_at: now,
        });
    let remaining = MAX_RESULT_READ_BYTES_PER_TURN.saturating_sub(budget.used);
    if remaining < 4 {
        return Err(anyhow!(
            "tool_result_read reached the per-turn read budget; continue in the next turn"
        ));
    }
    let reserved = requested.min(remaining);
    budget.used = budget.used.saturating_add(reserved);
    budget.touched_at = now;
    Ok(reserved)
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

fn escape_xml_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;")
}
