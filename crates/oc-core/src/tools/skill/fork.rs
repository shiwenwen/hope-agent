//! Fork skill activation — spawn a sub-agent with the SKILL.md injected as
//! extra system context, block until it terminates, and return only the final
//! assistant text as the tool_result. Main conversation never sees the sub-agent
//! transcript, which is the whole point of `context: fork`.

use anyhow::{anyhow, Result};

use crate::skills::{self, SkillEntry};
use crate::tools::ToolExecContext;

pub(super) async fn execute(
    entry: &SkillEntry,
    args: &str,
    ctx: &ToolExecContext,
) -> Result<String> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("Cannot fork skill outside of a chat session"))?;
    let agent_id = ctx.agent_id.as_deref().unwrap_or("default");

    // skip_parent_injection=true: the skill tool itself feeds the result back
    // as a tool_result; the EventBus injection path would otherwise deliver
    // the same text a second time as a user message.
    let run_id = skills::spawn_skill_fork(entry, args, session_id, agent_id, true).await?;

    skills::extract_fork_result(&run_id, &entry.name).await
}
