use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{execution_item, validate_item, AssistantError};
use crate::session::SessionDB;
use crate::tool_defs::ToolExecContext;

tokio::task_local! {
    static EXECUTION:(Arc<SessionDB>,String);
    static BLOCK_CUSTOM_HOOKS:bool;
}

pub(crate) fn custom_hooks_blocked() -> bool {
    BLOCK_CUSTOM_HOOKS
        .try_with(|blocked| *blocked)
        .unwrap_or(false)
}

pub(crate) async fn execution_scope<F: std::future::Future>(
    db: Arc<SessionDB>,
    execution: Option<String>,
    future: F,
) -> F::Output {
    if let Some(execution) = execution {
        let id = execution.clone();
        let blocked = db
            .run(move |db| db.assistant_execution_role(&id))
            .await
            .map(|role| role.is_some())
            .unwrap_or(true);
        BLOCK_CUSTOM_HOOKS
            .scope(blocked, EXECUTION.scope((db, execution), future))
            .await
    } else {
        future.await
    }
}

/// Compaction, vision, side queries and permission judges use this same
/// one-shot port, retaining the exact admitted execution across awaits.
pub(crate) async fn bounded_auxiliary_call<T, F: std::future::Future<Output = Result<T>>>(
    future: F,
) -> Result<T> {
    let owner = EXECUTION.try_with(|e| e.clone()).ok();
    let budget = if let Some((db, execution)) = owner {
        begin_model_budget(db, execution).await?
    } else {
        None
    };
    let result = if let Some(budget) = budget.as_ref() {
        tokio::time::timeout(budget.timeout, future)
            .await
            .map_err(|_| anyhow!("Assistant auxiliary model time allowance exhausted"))?
    } else {
        future.await
    };
    if let Some(budget) = budget {
        budget.finish().await?;
    }
    result
}

/// A reservation is durable before work starts. Cancellation/crash leaves its
/// worst-case charge in place; it never silently refunds unknown execution.
pub struct BudgetLease {
    db: Arc<SessionDB>,
    token: String,
    started: Instant,
    pub timeout: Duration,
    parked_at_start: Duration,
}

impl BudgetLease {
    pub async fn finish(self) -> Result<()> {
        let token = self.token.clone();
        let seconds = self.active_elapsed().as_secs().saturating_add(1);
        self.db.run(move |db| settle(db, &token, seconds)).await?;
        Ok(())
    }

    fn active_elapsed(&self) -> Duration {
        self.started.elapsed().saturating_sub(
            crate::async_jobs::approval_bridge::parked_budget_extension()
                .saturating_sub(self.parked_at_start),
        )
    }
}

// Dropping a live lease provides no termination proof. Its durable worst-case
// reservation stays occupied for recovery/owner verification; only finish()
// settles a handler/provider future whose result was actually observed.

pub async fn check_execution(ctx: &ToolExecContext) -> Result<()> {
    let Some(db) = ctx.session_db.as_ref().map(|d| d.0.clone()) else {
        return Ok(());
    };
    let execution = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone());
    let parent = ctx.session_id.clone();
    db.run(move |db| {
        let conn = db.read_conn()?;
        let bound=execution.as_deref().map(|id|execution_item(&conn,id)).transpose()?.flatten();
        if let Some((item, _, revision)) = bound {
            let session:String=conn.query_row("SELECT session_id FROM chat_stream_runs WHERE run_id=?1",
                [&execution],|r|r.get(0))?;
            if parent.as_deref()!=Some(session.as_str()){bail!(AssistantError::Unavailable)}
            let closed: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts
                WHERE execution_id=?1 AND closed_for_question=1)",
                [&execution],
                |r| r.get(0),
            )?;
            if closed {
                bail!("Assistant execution has handed control to its durable question")
            }
            validate_item(&conn, &item)?;
            let current: i64 = conn.query_row(
                "SELECT revision FROM assistant_items WHERE id=?1",
                [&item],
                |r| r.get(0),
            )?;
            if revision != current {
                bail!(AssistantError::Stopped)
            }
        }else{
            let owned:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts c
                JOIN chat_stream_runs s ON s.run_id=c.execution_id WHERE s.session_id=?1 AND s.status='running')",
                [parent],|r|r.get(0))?;
            if owned{bail!("Assistant work requires its exact admitted persistence identity")}
        }
        Ok(())
    })
    .await
}

pub async fn check_tool_capability(
    ctx: &ToolExecContext,
    name: &str,
    args: &serde_json::Value,
) -> Result<()> {
    check_execution(ctx).await?;
    let Some(db) = ctx.session_db.as_ref() else {
        return Ok(());
    };
    let Some(execution) = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone())
    else {
        return Ok(());
    };
    if db
        .0
        .run(move |db| db.assistant_execution_role(&execution))
        .await?
        .is_none()
    {
        return Ok(());
    }
    let directory = ctx.default_path().to_owned();
    let hooked = crate::blocking::run_blocking(move || {
        !crate::hooks::scopes::resolve_for_cwd(Some(std::path::Path::new(&directory))).is_empty()
    })
    .await;
    if hooked {
        bail!("Custom hooks do not yet retain Hope item provenance; background item execution is blocked")
    }
    if name == "exec"
        && (args.get("background").and_then(|v| v.as_bool()) == Some(true)
            || args.get("yield_ms").is_some())
    {
        bail!("Hope item work uses run_in_background managed jobs; legacy background/yield_ms processes cannot retain its execution allowance")
    }
    if matches!(
        name,
        crate::tool_defs::TOOL_ACP_SPAWN
            | crate::tool_defs::TOOL_TEAM
            | crate::tool_defs::TOOL_MANAGE_CRON
            | crate::tool_defs::TOOL_WORKFLOW
            | crate::tool_defs::TOOL_LOOP_RESCHEDULE
            | crate::tool_defs::TOOL_ENTER_PLAN_MODE
    ) {
        bail!("This independent controller does not accept Hope item provenance; use item-owned tools or subagents")
    }
    Ok(())
}

pub async fn begin_tool_budget(
    ctx: &ToolExecContext,
    name: &str,
    _args: &serde_json::Value,
) -> Result<Option<BudgetLease>> {
    let Some(db) = ctx.session_db.as_ref().map(|d| d.0.clone()) else {
        return Ok(None);
    };
    let Some(execution) = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone())
    else {
        return Ok(None);
    };
    let call = ctx
        .tool_call_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let token = format!("tool:{execution}:{call}:{}", ctx.assistant_retry_attempt);
    let replay_safe = matches!(
        name,
        "read"
            | "ls"
            | "find"
            | "grep"
            | "web_search"
            | "web_fetch"
            | "session_status"
            | "sessions_history"
    );
    let waiting = Instant::now();
    let lease = loop {
        match begin(
            db.clone(),
            execution.clone(),
            token.clone(),
            "tool",
            if name == crate::tool_defs::TOOL_ASSISTANT_REPORT {
                1
            } else {
                900
            },
            replay_safe,
        )
        .await
        {
            Err(error)
                if error.downcast_ref::<AssistantError>() == Some(&AssistantError::Busy)
                    && waiting.elapsed() < Duration::from_secs(30) =>
            {
                if ctx
                    .cancellation_token
                    .as_ref()
                    .is_some_and(|c| c.is_cancelled())
                {
                    bail!(AssistantError::Stopped)
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            result => break result?,
        }
    };
    Ok(lease)
}

pub async fn begin_model_budget(
    db: Arc<SessionDB>,
    execution: String,
) -> Result<Option<BudgetLease>> {
    let token = format!("model:{execution}:{}", uuid::Uuid::new_v4());
    let waiting = Instant::now();
    loop {
        match begin(
            db.clone(),
            execution.clone(),
            token.clone(),
            "model",
            90,
            true,
        )
        .await
        {
            Err(error)
                if error.downcast_ref::<AssistantError>() == Some(&AssistantError::Busy)
                    && waiting.elapsed() < Duration::from_secs(30) =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            result => return result,
        }
    }
}

async fn begin(
    db: Arc<SessionDB>,
    execution: String,
    token: String,
    kind: &'static str,
    max_seconds: i64,
    replay_safe: bool,
) -> Result<Option<BudgetLease>> {
    let reservation = token.clone();
    let timeout = db
        .run(move |db| {
            let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let Some((item, root, revision)) = execution_item(&tx, &execution)? else {
                return Ok(None);
            };
            let closed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_execution_contexts
            WHERE execution_id=?1 AND closed_for_question=1)",
                [&execution],
                |r| r.get(0),
            )?;
            if closed {
                bail!("Assistant execution has handed control to its durable question")
            }
            validate_item(&tx, &item)?;
            let waiting_for_owner: bool = tx.query_row(
                "SELECT reason='needs_input' FROM assistant_items
            WHERE id=?1",
                [&item],
                |r| r.get::<_, Option<bool>>(0).map(|v| v.unwrap_or(false)),
            )?;
            let subtask: bool = tx.query_row(
                "SELECT is_subtask FROM assistant_execution_contexts WHERE execution_id=?1",
                [&execution],
                |r| r.get(0),
            )?;
            if (waiting_for_owner || super::needs_owner_input(&tx, &item)?) && !subtask {
                bail!("Assistant item is waiting for its durable owner answer")
            }
            let (current, used_tools, tool_limit, used_seconds, seconds_limit): (
                i64,
                i64,
                i64,
                i64,
                i64,
            ) = tx.query_row(
                "SELECT revision,tools_used,tool_limit,active_seconds,active_seconds_limit
             FROM assistant_items WHERE id=?1",
                [&item],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
            if revision != current {
                bail!(AssistantError::Stopped)
            }
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_budget_reservations
            WHERE token=?1)",
                [&reservation],
                |r| r.get(0),
            )?;
            if exists {
                bail!("Assistant operation already reserved; refusing duplicate execution")
            }
            let (reserved,parallel):(i64,i64)=tx.query_row(
            "SELECT COALESCE(SUM(reserved_seconds),0),COUNT(*) FROM assistant_budget_reservations
             WHERE item_id=?1 AND settled_at IS NULL",[&item],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let now = chrono::Utc::now().timestamp();
            let (_, owner_tool_limit, owner_seconds_limit) =
                super::owner_limits_with_conn(&tx, now)?;
            let owner: i64 = tx.query_row(
                "SELECT COALESCE(SUM(COALESCE(charged_seconds,reserved_seconds)),0)
            FROM assistant_budget_reservations WHERE started_at>=?1",
                [now - 86_400],
                |r| r.get(0),
            )?;
            let tools: i64 = tx.query_row(
                "SELECT COUNT(*) FROM assistant_budget_reservations
            WHERE started_at>=?1 AND kind='tool'",
                [now - 86_400],
                |r| r.get(0),
            )?;
            if parallel >= 2 {
                bail!(AssistantError::Busy)
            }
            let available =
                (seconds_limit - used_seconds - reserved).min(owner_seconds_limit - owner);
            if available <= 0
                || (kind == "tool" && (used_tools >= tool_limit || tools >= owner_tool_limit))
            {
                tx.execute(
                    "UPDATE assistant_items SET state='waiting',reason='budget_exhausted',
                updated_at=?2 WHERE id=?1",
                    params![item, chrono::Utc::now().to_rfc3339()],
                )?;
                tx.commit()?;
                bail!("Assistant cumulative execution budget exhausted; owner action required");
            }
            let seconds = available.min(max_seconds);
            tx.execute(
                "INSERT INTO assistant_budget_reservations(token,item_id,root_turn_id,kind,
            started_at,reserved_seconds,execution_id,replay_safe) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    reservation,
                    item,
                    root,
                    kind,
                    now,
                    seconds,
                    execution,
                    replay_safe
                ],
            )?;
            if kind == "tool" {
                tx.execute(
                    "UPDATE assistant_items SET tools_used=tools_used+1 WHERE id=?1",
                    [&item],
                )?;
                tx.execute(
                    "UPDATE assistant_item_runs SET tools_used=tools_used+1 WHERE turn_id=?1",
                    [&root],
                )?;
            }
            tx.commit()?;
            Ok(Some(seconds as u64))
        })
        .await?;
    Ok(timeout.map(|seconds| BudgetLease {
        db,
        token,
        started: Instant::now(),
        timeout: Duration::from_secs(seconds),
        parked_at_start: crate::async_jobs::approval_bridge::parked_budget_extension(),
    }))
}

fn settle(db: &SessionDB, token: &str, elapsed: u64) -> Result<()> {
    let mut conn = db.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT item_id,root_turn_id,reserved_seconds FROM assistant_budget_reservations
         WHERE token=?1 AND settled_at IS NULL",
            [token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((item, root, reserved)) = row {
        let seconds = (elapsed.min(i64::MAX as u64) as i64).min(reserved);
        tx.execute(
            "UPDATE assistant_budget_reservations SET charged_seconds=?2,settled_at=?3
            WHERE token=?1 AND settled_at IS NULL",
            params![token, seconds, chrono::Utc::now().timestamp()],
        )?;
        tx.execute(
            "UPDATE assistant_items SET active_seconds=active_seconds+?2 WHERE id=?1",
            params![item, seconds],
        )?;
        tx.execute(
            "UPDATE assistant_item_runs SET active_seconds=active_seconds+?2 WHERE turn_id=?1",
            params![root, seconds],
        )?;
    }
    tx.commit()?;
    Ok(())
}
