//! The `skill` tool — model's preferred entry point to activate a skill.
//!
//! Replaces the older "model reads SKILL.md via `read`" pattern. By routing
//! through a dedicated tool we can:
//!   - Uniformly dispatch `context: fork` to a sub-agent (previously only
//!     the `/skill-name` slash command did this).
//!   - Pass arguments via `$ARGUMENTS` substitution in the inline path.
//!   - Return a concise summary instead of dumping the full SKILL.md plus
//!     every downstream tool_result into the main conversation.

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::ToolExecContext;

mod fork;
mod inline;

/// Entry point registered in `tools::execution::execute_tool_with_context`.
pub(crate) async fn tool_skill(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("'name' is required (skill name to activate)"))?;

    let invocation_args = args
        .get("args")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let cfg = crate::config::cached_config();
    let skills =
        crate::skills::get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);

    let entry = skills
        .iter()
        .find(|s| s.name == name || crate::skills::normalize_skill_command_name(&s.name) == name)
        .ok_or_else(|| {
            let catalog: Vec<&str> = skills.iter().map(|s| s.name.as_str()).take(20).collect();
            anyhow!(
                "Skill '{}' not found. Available: {}{}",
                name,
                catalog.join(", "),
                if skills.len() > 20 { ", ..." } else { "" }
            )
        })?;

    // disable_model_invocation skills are user-only (slash command); reject here.
    if entry.disable_model_invocation == Some(true) {
        return Err(anyhow!(
            "Skill '{}' is marked disable-model-invocation and can only be run via slash command",
            entry.name
        ));
    }

    if entry.context_mode.as_deref() == Some("fork") {
        fork::execute(entry, invocation_args, ctx).await
    } else {
        inline::execute(entry, invocation_args).await
    }
}
