//! Top-level entry points for building an awareness prompt section.
//!
//! This is called at session bootstrap to produce a *static* preheating block
//! that is baked into the cached system prompt. The dynamic, per-turn block
//! is produced by `SessionAwareness::prepare_dynamic_suffix` and travels as a
//! separate cache breakpoint in the provider layer.

use anyhow::Result;

use super::config::{AwarenessConfig, AwarenessMode};

/// Produce the static awareness prefix block (optional).
///
/// This is intentionally short — the dynamic suffix (via `SessionAwareness`)
/// carries the actually-fresh content. The prefix is mainly useful for
/// instructing the model that the feature exists and that a suffix will be
/// appended.
pub fn build_prompt_section(
    current_session_id: Option<&str>,
    cfg: &AwarenessConfig,
) -> Result<Option<String>> {
    if !cfg.enabled || matches!(cfg.mode, AwarenessMode::Off) {
        return Ok(None);
    }
    let current = current_session_id.unwrap_or("-");
    let mut out = String::new();
    out.push_str("# Cross-Session Context (overview)\n\n");
    out.push_str(
        "You may see an additional `# Cross-Session Context` block appended near the \
end of this prompt. It is refreshed dynamically and describes what the user is \
doing in other parallel sessions right now. Use it to understand references like \
\"the thing I was working on earlier\" and to avoid re-asking for context \
established elsewhere. Do NOT assume actions taken in other sessions are visible \
in this one.",
    );
    out.push_str(&format!("\n\nCurrent session: `{}`.", current));
    Ok(Some(out))
}
