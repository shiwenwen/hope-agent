//! Environment assembly for hook handlers (design doc §11).
//!
//! `build_for_command` produces the §11.1 variable set. `CLAUDE_ENV_FILE`
//! (§11.3) is intentionally out of this phase.

use std::collections::HashMap;
use std::path::PathBuf;

use super::types::CommonHookInput;

/// Environment variables handed to a `command` hook (overwriting the inherited
/// process environment for the listed keys).
#[derive(Debug, Clone, Default)]
pub struct HookEnv {
    pub(crate) vars: HashMap<String, String>,
    /// Working directory for the child process (the session/project cwd). The
    /// command runner sets `current_dir` to this when it exists, so a hook's
    /// relative paths resolve against the project root — not the hope-agent
    /// process cwd. `None` (e.g. `empty()`) leaves the inherited cwd.
    pub(crate) cwd: Option<PathBuf>,
}

impl HookEnv {
    /// An env carrying no overrides.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The override map (for spawning a child process).
    pub fn as_vars(&self) -> &HashMap<String, String> {
        &self.vars
    }

    /// Build the §11.1 environment for a `command` hook. `CLAUDE_PROJECT_DIR`
    /// and `HOPE_PROJECT_DIR` are double-injected with the same value (session
    /// cwd / project root) so official scripts paste-and-run.
    pub fn build_for_command(common: &CommonHookInput) -> Self {
        let mut vars = HashMap::new();
        let project_dir = common.cwd.to_string_lossy().to_string();
        vars.insert("CLAUDE_PROJECT_DIR".to_string(), project_dir.clone());
        vars.insert("HOPE_PROJECT_DIR".to_string(), project_dir);
        vars.insert(
            "HOPE_AGENT_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        vars.insert("HOPE_SESSION_ID".to_string(), common.session_id.clone());
        vars.insert(
            "HOPE_TRANSCRIPT_PATH".to_string(),
            common.transcript_path.to_string_lossy().to_string(),
        );
        // Desktop = local; server / ACP = remote (official `CLAUDE_CODE_REMOTE`).
        let remote = if crate::app_init::is_desktop() {
            "false"
        } else {
            "true"
        };
        vars.insert("CLAUDE_CODE_REMOTE".to_string(), remote.to_string());
        // Official `CLAUDE_EFFORT` — present only when the session exposes an
        // effort level.
        if let Some(effort) = &common.effort {
            vars.insert("CLAUDE_EFFORT".to_string(), effort.level.clone());
        }
        // Resolve the login shell PATH so `npm` / `python` are findable
        // (Unix). On Windows this is `None` and we leave PATH inherited.
        if let Some(path) = crate::tools::exec::get_login_shell_path() {
            vars.insert("PATH".to_string(), path.to_string());
        }
        Self {
            vars,
            cwd: Some(common.cwd.clone()),
        }
    }
}
