use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::runtime_tasks::{cancel_runtime_task, RuntimeTaskKind};

pub(crate) async fn tool_runtime_cancel(args: &Value) -> Result<String> {
    let kind_str = args
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("runtime_cancel: missing required `kind` parameter"))?;
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("runtime_cancel: missing required `id` parameter"))?;

    let kind = match kind_str {
        "async_job" => RuntimeTaskKind::AsyncJob,
        "subagent" => RuntimeTaskKind::Subagent,
        "process" => RuntimeTaskKind::Process,
        "cron" => RuntimeTaskKind::Cron,
        other => return Err(anyhow!("runtime_cancel: unknown kind `{}`", other)),
    };

    let result = cancel_runtime_task(kind, id).await?;
    Ok(serde_json::to_string(&result)?)
}
