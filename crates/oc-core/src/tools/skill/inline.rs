//! Inline skill activation — read SKILL.md from disk, substitute `$ARGUMENTS`,
//! and return the content so the LLM loads it as a tool_result on the current
//! turn. The model then follows the skill's instructions inside the main
//! conversation.

use anyhow::{anyhow, Result};

use crate::skills::SkillEntry;

pub(super) async fn execute(entry: &SkillEntry, args: &str) -> Result<String> {
    let path = entry.file_path.clone();
    let args_owned = args.to_string();

    // Disk IO off the async runtime thread.
    let content = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
        .map_err(|e| anyhow!("Failed to read SKILL.md for '{}': {e}", entry.name))?;

    let substituted = content.replace("$ARGUMENTS", &args_owned);

    Ok(substituted)
}
