//! Multi-scope AllowAlways persistence.
//!
//! When the user picks "Always allow" in the approval dialog, we persist
//! a `RuleSpec` into one of four scopes based on context:
//!
//! - **Project** — `~/.hope-agent/projects/{project_id}/allowlist.json`
//! - **Session** — in-memory only, dies with the session
//! - **Agent home** — `~/.hope-agent/agents/{agent_id}/allowlist.json`
//!   (only used when the rule's path/command is rooted in agent home)
//! - **Global** — `~/.hope-agent/permission/global-allowlist.json`
//!   (default for `web_fetch` domain rules and cross-project commands)
//!
//! The dialog UX picks a context-appropriate default scope but lets the user
//! override.

use serde::{Deserialize, Serialize};

/// Scope of an AllowAlways grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowScope {
    /// `~/.hope-agent/projects/{project_id}/allowlist.json`
    Project,
    /// In-memory, per session_id, lost on session close.
    Session,
    /// `~/.hope-agent/agents/{agent_id}/allowlist.json`
    AgentHome,
    /// `~/.hope-agent/permission/global-allowlist.json`
    Global,
}

impl AllowScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Session => "session",
            Self::AgentHome => "agent_home",
            Self::Global => "global",
        }
    }
}

// Persistence helpers (load / add_rule / remove_rule per scope) land here
// once the GUI / approval-dialog wiring needs them. In-memory session
// scope: `Lazy<RwLock<HashMap<SessionId, PermissionRules>>>`. File-backed
// scopes use `platform::write_secure_file` for atomic 0600 writes.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_scope_as_str() {
        assert_eq!(AllowScope::Project.as_str(), "project");
        assert_eq!(AllowScope::Session.as_str(), "session");
        assert_eq!(AllowScope::AgentHome.as_str(), "agent_home");
        assert_eq!(AllowScope::Global.as_str(), "global");
    }

    #[test]
    fn allow_scope_serde_matches_as_str() {
        for scope in [
            AllowScope::Project,
            AllowScope::Session,
            AllowScope::AgentHome,
            AllowScope::Global,
        ] {
            let via_serde = serde_json::to_value(scope)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(scope.as_str(), via_serde);
        }
    }
}
