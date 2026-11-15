use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::tool_defs::ToolExecContext;

pub(super) async fn execute(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    if ctx.incognito {
        anyhow::bail!("Assistant reports are unavailable in incognito sessions");
    }
    let db = ctx
        .session_db
        .as_ref()
        .ok_or_else(|| anyhow!("Session database unavailable"))?
        .0
        .clone();
    let session = ctx
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("Live worker session required"))?;
    let turn = ctx
        .persistence_run_id
        .clone()
        .or_else(|| ctx.turn_id.clone())
        .ok_or_else(|| anyhow!("Live admitted assistant execution required"))?;
    let outcome = args
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let next = args
        .get("next_instruction")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(question) = args.get("question").and_then(Value::as_str) {
        if outcome != "waiting" {
            anyhow::bail!("A question requires outcome=waiting")
        }
        let question = question.to_owned();
        let options: Vec<crate::ask_user::AskUserQuestionOption> = serde_json::from_value(
            args.get("options")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )?;
        let group = db
            .run(move |db| db.create_assistant_question(&turn, &question, options))
            .await?;
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                crate::ask_user::EVENT_ASK_USER_REQUEST,
                serde_json::to_value(&group)?,
            );
        }
        super::approval::emit_pending_interactions_changed(Some(&group.session_id));
        return Ok(
            serde_json::json!({"status":"waiting_for_owner","questionId":group.request_id,
            "instruction":"End this turn. A durable owner answer will resume the existing item."})
            .to_string(),
        );
    }
    db.run(move |db| {
        db.report_assistant_item(&session, &turn, &outcome, &summary, next.as_deref())
    })
    .await?;
    Ok(
        serde_json::json!({"status":"recorded","delivery":"after_execution_reconciliation"})
            .to_string(),
    )
}
