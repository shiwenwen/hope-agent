//! Default-agent resolution rules.
//!
//! There are eight places a default agent can come from. They are tried in
//! the order below; the first non-empty one wins.
//!
//! 1. **Explicit caller** — caller passes an `agent_id` directly. The
//!    explicit override beats every other level.
//! 2. **Project default** — `project.default_agent_id`. The project says
//!    "every new session in me uses this agent".
//! 3. **IM topic override** — `TelegramTopicConfig.agent_id`. Specific to
//!    one Telegram forum topic (most-specific IM scope).
//! 4. **IM group override** — `TelegramGroupConfig.agent_id`. Per-group
//!    override (parent of topic).
//! 5. **IM Telegram-channel override** — `TelegramChannelConfig.agent_id`
//!    for broadcast-style channels.
//! 6. **Channel-account default** — `ChannelAccountConfig.agent_id`. The
//!    soft default users see when they configure a Telegram / Slack /
//!    LINE / etc. account.
//! 7. **Global default** — `AppConfig.default_agent_id`, configured in
//!    settings. Defaults to `"default"`.
//! 8. **Hardcoded fallback** — the literal string `"default"`. Last-resort
//!    safety net so we always return a non-empty id.
//!
//! Pass `None` for any level you do not have in scope (e.g. desktop flows
//! pass `None` for every IM-related level).
//!
//! See [`docs/architecture/api-reference.md`] and `AGENTS.md` for the full
//! contract.

use crate::channel::{
    ChannelAccountConfig, TelegramChannelConfig, TelegramGroupConfig, TelegramTopicConfig,
};
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
///
/// Convenience wrapper around [`resolve_default_agent_id_full`] for callers
/// without IM topic / group / channel scope (desktop / HTTP).
pub fn resolve_default_agent_id(
    project: Option<&Project>,
    channel_account: Option<&ChannelAccountConfig>,
) -> String {
    resolve_default_agent_id_full(None, project, None, None, None, channel_account).0
}

/// Where the resolved agent id came from. Surfaced to the user via /status so
/// they can debug why a particular agent was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    /// Caller passed an explicit override.
    Explicit,
    Project,
    /// Telegram topic (forum-thread) override.
    Topic,
    /// Telegram group override.
    Group,
    /// Telegram broadcast-channel override.
    ChannelOverride,
    /// Channel-account default (soft default in account config).
    ChannelAccount,
    GlobalConfig,
    Hardcoded,
}

impl AgentSource {
    pub fn label(&self) -> &'static str {
        match self {
            AgentSource::Explicit => "explicit",
            AgentSource::Project => "project",
            AgentSource::Topic => "topic",
            AgentSource::Group => "group",
            AgentSource::ChannelOverride => "channel-override",
            AgentSource::ChannelAccount => "channel",
            AgentSource::GlobalConfig => "global",
            AgentSource::Hardcoded => "hardcoded",
        }
    }
}

/// Convenience wrapper retained for desktop / HTTP callers — same as
/// [`resolve_default_agent_id_full`] without IM topic / group / channel
/// override scope.
pub fn resolve_default_agent_id_with_source(
    project: Option<&Project>,
    channel_account: Option<&ChannelAccountConfig>,
) -> (String, AgentSource) {
    resolve_default_agent_id_full(None, project, None, None, None, channel_account)
}

/// Full agent-resolution helper covering every level of the precedence chain.
///
/// Pass `None` for any level you do not have in scope. The function
/// short-circuits at the first non-empty id and reports its [`AgentSource`].
///
/// IM dispatch (topic > group > channel-override > channel-account) is
/// folded in here so the channel worker doesn't reinvent the chain
/// privately.
pub fn resolve_default_agent_id_full(
    explicit: Option<&str>,
    project: Option<&Project>,
    topic: Option<&TelegramTopicConfig>,
    group: Option<&TelegramGroupConfig>,
    channel: Option<&TelegramChannelConfig>,
    channel_account: Option<&ChannelAccountConfig>,
) -> (String, AgentSource) {
    if let Some(id) = explicit {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return (trimmed.to_string(), AgentSource::Explicit);
        }
    }
    if let Some(p) = project {
        if let Some(id) = p.default_agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::Project);
            }
        }
    }
    if let Some(t) = topic {
        if let Some(id) = t.agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::Topic);
            }
        }
    }
    if let Some(g) = group {
        if let Some(id) = g.agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::Group);
            }
        }
    }
    if let Some(c) = channel {
        if let Some(id) = c.agent_id.as_ref() {
            if !id.trim().is_empty() {
                return (id.clone(), AgentSource::ChannelOverride);
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
