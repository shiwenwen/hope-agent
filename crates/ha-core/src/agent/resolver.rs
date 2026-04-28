//! Default-agent resolution rules.
//!
//! There are five places a default agent can come from. They are tried in the
//! order:
//!
//! 1. **Explicit caller** — caller passes an `agent_id` directly. This is the
//!    only level resolved outside this helper; if the caller has a value it
//!    short-circuits.
//! 2. **Project default** — `project.default_agent_id`. The project says
//!    "every new session in me uses this agent". More specific than channel
//!    binding because the project is the context the user explicitly chose.
//! 3. **Channel-account default** — `ChannelAccountConfig.agent_id`. Scoped
//!    to one IM channel account. Only relevant in IM-driven flows.
//! 4. **Global default** — `AppConfig.default_agent_id`. User-configurable in
//!    the settings page; defaults to `"default"`.
//! 5. **Hardcoded fallback** — the literal string `"default"`. Last-resort
//!    safety net so we always return a non-empty id.
//!
//! Levels (2)–(4) are merged here. Pass `None` for any level you do not have
//! in scope (e.g. the channel-binding level is `None` in desktop flows).
//!
//! See [`docs/architecture/api-reference.md`] and `AGENTS.md` for the full
//! contract.

use crate::channel::ChannelAccountConfig;
use crate::project::Project;

/// Hardcoded last-resort agent id.
pub const HARDCODED_DEFAULT_AGENT_ID: &str = "default";

/// Normalize an incoming `default_agent_id` override: trim whitespace, treat
/// empty/whitespace-only as `None`. Used by every write path
/// (Tauri command / HTTP route / `update_settings` tool branch) to keep the
/// "empty string == clear" semantics consistent.
pub fn normalize_default_agent_id(input: Option<&str>) -> Option<String> {
    input.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Resolve the default agent id given the optional project and channel-account
/// context. The `AppConfig.default_agent_id` field is read from the cached
/// global config snapshot.
///
/// Returns a non-empty `String` (always — falls back to `"default"`).
pub fn resolve_default_agent_id(
    project: Option<&Project>,
    channel_account: Option<&ChannelAccountConfig>,
) -> String {
    if let Some(p) = project {
        if let Some(id) = p.default_agent_id.as_ref() {
            if !id.trim().is_empty() {
                return id.clone();
            }
        }
    }
    if let Some(c) = channel_account {
        if let Some(id) = c.agent_id.as_ref() {
            if !id.trim().is_empty() {
                return id.clone();
            }
        }
    }
    if let Some(id) = crate::config::cached_config().default_agent_id.as_ref() {
        if !id.trim().is_empty() {
            return id.clone();
        }
    }
    HARDCODED_DEFAULT_AGENT_ID.to_string()
}

/// Where the resolved agent id came from. Surfaced to the user via /status so
/// they can debug why a particular agent was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    Project,
    ChannelAccount,
    GlobalConfig,
    Hardcoded,
}

impl AgentSource {
    pub fn label(&self) -> &'static str {
        match self {
            AgentSource::Project => "project",
            AgentSource::ChannelAccount => "channel",
            AgentSource::GlobalConfig => "global",
            AgentSource::Hardcoded => "hardcoded",
        }
    }
}

/// Same as [`resolve_default_agent_id`] but also reports which level supplied
/// the value. Cheap by design — no extra DB / config reads beyond what the
/// non-explained version already does.
pub fn resolve_default_agent_id_with_source(
    project: Option<&Project>,
    channel_account: Option<&ChannelAccountConfig>,
) -> (String, AgentSource) {
    if let Some(p) = project {
        if let Some(id) = p.default_agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::Project);
            }
        }
    }
    if let Some(c) = channel_account {
        if let Some(id) = c.agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::ChannelAccount);
            }
        }
    }
    if let Some(id) = crate::config::cached_config().default_agent_id.as_ref() {
        if !id.trim().is_empty() {
            return (id.clone(), AgentSource::GlobalConfig);
        }
    }
    (
        HARDCODED_DEFAULT_AGENT_ID.to_string(),
        AgentSource::Hardcoded,
    )
}
