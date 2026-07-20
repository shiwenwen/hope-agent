//! Inline skill activation — read SKILL.md from disk, substitute `$ARGUMENTS`,
//! and return the content so the LLM loads it as a tool_result on the current
//! turn. The model then follows the skill's instructions inside the main
//! conversation.

use anyhow::{anyhow, Result};

use crate::skills::SkillEntry;

pub(super) async fn execute(entry: &SkillEntry, args: &str) -> Result<String> {
    let path = entry.file_path.clone();
    let args_owned = args.to_string();

    // `ha-manual` routes the model to the on-disk manual mirror; make sure it
    // exists before the skill instructions run. This lives HERE (not in the
    // `skill` tool dispatch) because it is the chokepoint shared by BOTH
    // activation paths — the model's `skill({name})` call and the user's
    // `/manual` slash command via `render_inline` — so a startup-mirror
    // failure is retried on every activation, whichever door was used.
    // Idempotent: the fingerprint check short-circuits once mirrored.
    if entry.name == "ha-manual" {
        let _ = tokio::task::spawn_blocking(crate::manual::ensure_local_manual).await;
    }

    // Disk IO off the async runtime thread.
    let content = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
        .map_err(|e| anyhow!("Failed to read SKILL.md for '{}': {e}", entry.name))?;

    let substituted = content.replace("$ARGUMENTS", &args_owned);

    Ok(crate::skills::build_skill_context_payload(
        entry,
        &substituted,
    ))
}
