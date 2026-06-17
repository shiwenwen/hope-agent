//! Multi-scope AllowAlways persistence.
//!
//! When the user picks "Always allow" in the approval dialog, we persist
//! a `RuleSpec` into one of four scopes based on context:
//!
//! - **Project** — `~/.hope-agent/projects/{project_id}/allowlist.json`
//! - **Session** — in-memory only, dies with the session
//! - **Agent home** — `~/.hope-agent/agents/{agent_id}/allowlist.json`
//!   (agent-scoped fallback when no project is active)
//! - **Global** — `~/.hope-agent/permission/global-allowlist.json`
//!   (default for URL domain rules and commands when no project is active)
//!
//! The backend currently picks a context-appropriate default scope. The dialog
//! can grow an explicit scope picker later without changing the rule format.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::rules::{
    normalize_lexical, path_starts_with, resolve_path_with_default, ArgMatcher, PermissionRules,
    RuleSpec,
};

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

/// Minimal context needed to choose the AllowAlways scope and build
/// context-aware path matchers.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrantContext<'a> {
    pub session_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub default_path: Option<&'a str>,
    pub home_dir: Option<&'a str>,
    /// When the owning session is incognito, [`choose_scope`] is forced to the
    /// in-memory `Session` scope so an AllowAlways grant never persists to disk
    /// and is cleared on burn-on-close. Epic E (INCOG-6).
    pub incognito: bool,
}

/// The concrete grant that was persisted after an AllowAlways response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGrant {
    pub scope: AllowScope,
    pub rule: RuleSpec,
}

static SESSION_RULES: OnceLock<RwLock<HashMap<String, PermissionRules>>> = OnceLock::new();
static PROJECT_RULES: OnceLock<RwLock<HashMap<String, PermissionRules>>> = OnceLock::new();
static AGENT_RULES: OnceLock<RwLock<HashMap<String, PermissionRules>>> = OnceLock::new();
static GLOBAL_RULES: OnceLock<RwLock<Option<PermissionRules>>> = OnceLock::new();

fn session_rules() -> &'static RwLock<HashMap<String, PermissionRules>> {
    SESSION_RULES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn project_rules() -> &'static RwLock<HashMap<String, PermissionRules>> {
    PROJECT_RULES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn agent_rules() -> &'static RwLock<HashMap<String, PermissionRules>> {
    AGENT_RULES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn global_rules() -> &'static RwLock<Option<PermissionRules>> {
    GLOBAL_RULES.get_or_init(|| RwLock::new(None))
}

/// Return `true` if any active AllowAlways scope permits this tool call.
pub fn allows_tool_call(
    tool_name: &str,
    args: &Value,
    session_id: Option<&str>,
    project_id: Option<&str>,
    agent_id: Option<&str>,
    default_path: Option<&str>,
) -> bool {
    let default_path = default_path.map(Path::new);
    let matches = |rules: PermissionRules| {
        rules
            .allow
            .iter()
            .any(|rule| rule.matches_with_default_path(tool_name, args, default_path))
    };

    if let Some(session_id) = non_empty(session_id) {
        if let Some(rules) = session_rules()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .cloned()
        {
            if matches(rules) {
                return true;
            }
        }
    }

    if let Some(project_id) = non_empty(project_id) {
        if matches(load_project_rules(project_id)) {
            return true;
        }
    }

    if let Some(agent_id) = non_empty(agent_id) {
        if matches(load_agent_rules(agent_id)) {
            return true;
        }
    }

    matches(load_global_rules())
}

/// Build and persist an AllowAlways rule for the approved tool call.
pub fn add_allow_always_for_call(
    tool_name: &str,
    args: &Value,
    ctx: GrantContext<'_>,
) -> Result<StoredGrant> {
    let rule = rule_for_call(tool_name, args, &ctx);
    let scope = choose_scope(&rule, &ctx);
    add_rule(scope, scope_key(scope, &ctx), rule.clone())?;
    Ok(StoredGrant { scope, rule })
}

fn add_rule(scope: AllowScope, key: Option<String>, rule: RuleSpec) -> Result<()> {
    match scope {
        AllowScope::Session => {
            let key = key.context("session AllowAlways scope requires a session id")?;
            let mut guard = session_rules().write().unwrap_or_else(|e| e.into_inner());
            let rules = guard.entry(key).or_default();
            push_unique(&mut rules.allow, rule);
            Ok(())
        }
        AllowScope::Project => {
            let project_id = key.context("project AllowAlways scope requires a project id")?;
            let mut rules = load_project_rules(&project_id);
            push_unique(&mut rules.allow, rule);
            write_rules(&project_path(&project_id)?, &rules)?;
            project_rules()
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(project_id, rules);
            Ok(())
        }
        AllowScope::AgentHome => {
            let agent_id = key.context("agent AllowAlways scope requires an agent id")?;
            let mut rules = load_agent_rules(&agent_id);
            push_unique(&mut rules.allow, rule);
            write_rules(&agent_path(&agent_id)?, &rules)?;
            agent_rules()
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(agent_id, rules);
            Ok(())
        }
        AllowScope::Global => {
            let mut rules = load_global_rules();
            push_unique(&mut rules.allow, rule);
            write_rules(&global_path()?, &rules)?;
            *global_rules().write().unwrap_or_else(|e| e.into_inner()) = Some(rules);
            Ok(())
        }
    }
}

fn push_unique(rules: &mut Vec<RuleSpec>, rule: RuleSpec) {
    if !rules.contains(&rule) {
        rules.push(rule);
    }
}

fn choose_scope(rule: &RuleSpec, ctx: &GrantContext<'_>) -> AllowScope {
    // E5 (INCOG-6): incognito sessions must never persist an AllowAlways grant to
    // disk (project / agent-home / global). Force the in-memory `Session` scope —
    // it's wiped on burn-on-close (`clear_session_rules`), so "always allow"
    // lives and dies with the incognito session. The frontend additionally hides
    // the AllowAlways button; this is the backend backstop for any non-conforming
    // client that still sends an AllowAlways response. (If `session_id` is
    // somehow absent, `add_rule` fails the persist and the call is approved once.)
    if ctx.incognito {
        return AllowScope::Session;
    }
    if non_empty(ctx.project_id).is_some() {
        return AllowScope::Project;
    }
    if rule_prefers_global_scope(rule) {
        return AllowScope::Global;
    }
    if non_empty(ctx.agent_id).is_some() {
        return AllowScope::AgentHome;
    }
    if non_empty(ctx.session_id).is_some() && is_broad_tool_rule(rule) {
        return AllowScope::Session;
    }
    AllowScope::Global
}

fn is_broad_tool_rule(rule: &RuleSpec) -> bool {
    matches!(rule, RuleSpec::Tool { .. })
}

fn rule_prefers_global_scope(rule: &RuleSpec) -> bool {
    match rule {
        RuleSpec::Tool { .. } => false,
        RuleSpec::ToolPattern { matcher, .. } => matcher_prefers_global_scope(matcher),
    }
}

fn matcher_prefers_global_scope(matcher: &ArgMatcher) -> bool {
    match matcher {
        ArgMatcher::CommandPrefix { .. } | ArgMatcher::DomainGlob { .. } => true,
        ArgMatcher::All { matchers } => matchers.iter().any(matcher_prefers_global_scope),
        _ => false,
    }
}

fn scope_key(scope: AllowScope, ctx: &GrantContext<'_>) -> Option<String> {
    match scope {
        AllowScope::Project => non_empty(ctx.project_id).map(str::to_string),
        AllowScope::Session => non_empty(ctx.session_id).map(str::to_string),
        AllowScope::AgentHome => non_empty(ctx.agent_id).map(str::to_string),
        AllowScope::Global => None,
    }
}

fn rule_for_call(tool_name: &str, args: &Value, ctx: &GrantContext<'_>) -> RuleSpec {
    if let Some(matcher) = command_matcher(tool_name, args) {
        return RuleSpec::ToolPattern {
            name: tool_name.to_string(),
            matcher,
        };
    }
    if let Some(matcher) = path_matcher(tool_name, args, ctx) {
        return RuleSpec::ToolPattern {
            name: tool_name.to_string(),
            matcher,
        };
    }
    if let Some(matcher) = url_and_action_matcher(args) {
        return RuleSpec::ToolPattern {
            name: tool_name.to_string(),
            matcher,
        };
    }
    if let Some(matcher) = stable_field_matcher(args) {
        return RuleSpec::ToolPattern {
            name: tool_name.to_string(),
            matcher,
        };
    }
    RuleSpec::Tool {
        name: tool_name.to_string(),
    }
}

fn command_matcher(tool_name: &str, args: &Value) -> Option<ArgMatcher> {
    if tool_name != crate::tools::TOOL_EXEC {
        return None;
    }
    let command = args.get("command").and_then(|v| v.as_str())?;
    let prefix = command_prefix(command)?;
    Some(ArgMatcher::CommandPrefix { prefix })
}

fn command_prefix(command: &str) -> Option<String> {
    let trimmed = command.trim();
    let prefix = trimmed.split_whitespace().next().unwrap_or(trimmed).trim();
    (!prefix.is_empty()).then(|| prefix.to_string())
}

fn path_matcher(tool_name: &str, args: &Value, ctx: &GrantContext<'_>) -> Option<ArgMatcher> {
    let default_path = ctx.default_path.map(Path::new);
    if tool_name == crate::tools::TOOL_APPLY_PATCH {
        let patch = args.get("input").and_then(|v| v.as_str())?;
        let paths = super::rules::paths_in_patch_directives(patch);
        let prefix = path_prefix_for_paths(&paths, default_path)?;
        return Some(ArgMatcher::PathPrefix { prefix });
    }
    let prefix = super::rules::extract_path_arg(tool_name, args)
        .map(|path| path_prefix_for_call(&path, default_path))?;
    Some(ArgMatcher::PathPrefix { prefix })
}

fn path_prefix_for_paths(paths: &[PathBuf], default_path: Option<&Path>) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    let prefixes: Vec<PathBuf> = paths
        .iter()
        .map(|path| path_prefix_for_call(path, default_path))
        .collect();
    let common = common_path_prefix(&prefixes).unwrap_or_else(|| prefixes[0].clone());
    if common.parent().is_none() && prefixes[0].parent().is_some() {
        Some(prefixes[0].clone())
    } else {
        Some(common)
    }
}

fn path_prefix_for_call(path: &Path, default_path: Option<&Path>) -> PathBuf {
    let resolved = resolve_path_with_default(path, default_path);
    if let Some(default_path) = default_path {
        let default_path = normalize_lexical(&super::rules::expand_tilde(
            default_path.to_string_lossy().as_ref(),
        ));
        if path_starts_with(&resolved, &default_path) {
            return default_path;
        }
    }
    if let Some(parent) = resolved.parent() {
        if parent.parent().is_some() {
            return parent.to_path_buf();
        }
        resolved
    } else {
        resolved
    }
}

fn common_path_prefix(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let mut common: Vec<_> = first.components().collect();
    for path in &paths[1..] {
        let components: Vec<_> = path.components().collect();
        let shared_len = common
            .iter()
            .zip(components.iter())
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(shared_len);
        if common.is_empty() {
            return None;
        }
    }
    let mut out = PathBuf::new();
    for component in common {
        out.push(component.as_os_str());
    }
    (!out.as_os_str().is_empty()).then_some(normalize_lexical(&out))
}

fn url_and_action_matcher(args: &Value) -> Option<ArgMatcher> {
    let host = args
        .get("url")
        .and_then(|v| v.as_str())
        .and_then(url_host)?;
    let mut matchers = stable_field_matchers(args);
    matchers.push(ArgMatcher::DomainGlob { glob: host });
    Some(one_or_all(matchers))
}

fn url_host(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw).ok()?;
    parsed.host_str().map(|h| h.to_ascii_lowercase())
}

fn stable_field_matcher(args: &Value) -> Option<ArgMatcher> {
    let matchers = stable_field_matchers(args);
    (!matchers.is_empty()).then(|| one_or_all(matchers))
}

fn stable_field_matchers(args: &Value) -> Vec<ArgMatcher> {
    ["action", "op", "kind", "scope"]
        .iter()
        .filter_map(|field| {
            args.get(*field)
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(|value| ArgMatcher::FieldEquals {
                    field: (*field).to_string(),
                    value: value.to_string(),
                })
        })
        .collect()
}

fn one_or_all(mut matchers: Vec<ArgMatcher>) -> ArgMatcher {
    if matchers.len() == 1 {
        matchers.remove(0)
    } else {
        ArgMatcher::All { matchers }
    }
}

fn load_project_rules(project_id: &str) -> PermissionRules {
    if let Some(rules) = project_rules()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(project_id)
        .cloned()
    {
        return rules;
    }
    let loaded = read_rules(project_path(project_id).ok());
    project_rules()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(project_id.to_string(), loaded.clone());
    loaded
}

fn load_agent_rules(agent_id: &str) -> PermissionRules {
    if let Some(rules) = agent_rules()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
        .cloned()
    {
        return rules;
    }
    let loaded = read_rules(agent_path(agent_id).ok());
    agent_rules()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(agent_id.to_string(), loaded.clone());
    loaded
}

fn load_global_rules() -> PermissionRules {
    if let Some(rules) = global_rules()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return rules;
    }
    let loaded = read_rules(global_path().ok());
    *global_rules().write().unwrap_or_else(|e| e.into_inner()) = Some(loaded.clone());
    loaded
}

fn read_rules(path: Option<PathBuf>) -> PermissionRules {
    let Some(path) = path else {
        return PermissionRules::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            app_warn!(
                "permission",
                "allowlist",
                "Failed to parse AllowAlways rules at {}: {}",
                path.display(),
                e
            );
            PermissionRules::default()
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => PermissionRules::default(),
        Err(e) => {
            app_warn!(
                "permission",
                "allowlist",
                "Failed to read AllowAlways rules at {}: {}",
                path.display(),
                e
            );
            PermissionRules::default()
        }
    }
}

fn write_rules(path: &Path, rules: &PermissionRules) -> Result<()> {
    let json = serde_json::to_vec_pretty(rules)?;
    crate::platform::write_secure_file(path, &json)
        .with_context(|| format!("write AllowAlways rules to {}", path.display()))?;
    Ok(())
}

fn project_path(project_id: &str) -> Result<PathBuf> {
    Ok(crate::paths::project_dir(project_id)?.join("allowlist.json"))
}

fn agent_path(agent_id: &str) -> Result<PathBuf> {
    Ok(crate::paths::agent_dir(agent_id)?.join("allowlist.json"))
}

fn global_path() -> Result<PathBuf> {
    Ok(crate::paths::permission_dir()?.join("global-allowlist.json"))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|v| {
        let trimmed = v.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

/// Remove all cached permission rules for a session. Called by the session
/// cleanup watcher on delete / purge so a deleted session's in-memory
/// AllowAlways rules don't linger for the process lifetime (INCOG-7).
pub fn clear_session_rules(session_id: &str) {
    if let Some(cache) = SESSION_RULES.get() {
        cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }
}

#[cfg(test)]
pub(crate) fn clear_caches_for_tests() {
    if let Some(cache) = SESSION_RULES.get() {
        cache.write().unwrap_or_else(|e| e.into_inner()).clear();
    }
    if let Some(cache) = PROJECT_RULES.get() {
        cache.write().unwrap_or_else(|e| e.into_inner()).clear();
    }
    if let Some(cache) = AGENT_RULES.get() {
        cache.write().unwrap_or_else(|e| e.into_inner()).clear();
    }
    if let Some(cache) = GLOBAL_RULES.get() {
        *cache.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn incognito_forces_in_memory_session_scope() {
        let args = json!({ "path": "src/lib.rs" });
        let tmp = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", tmp.path())], || {
            clear_caches_for_tests();
            // A project-attached session would normally persist the grant to the
            // on-disk Project scope. E5 (INCOG-6): incognito must override to the
            // in-memory Session scope so the grant is wiped on burn-on-close.
            let incog = GrantContext {
                session_id: Some("sess-incognito"),
                project_id: Some("proj"),
                agent_id: Some("agent"),
                default_path: Some("/tmp/work"),
                incognito: true,
                ..Default::default()
            };
            let grant =
                add_allow_always_for_call("write", &args, incog).expect("persist incognito grant");
            assert_eq!(grant.scope, AllowScope::Session);

            // The identical call WITHOUT incognito lands in the persistent
            // Project scope — proving the override is what redirected it.
            clear_caches_for_tests();
            let normal = GrantContext {
                incognito: false,
                ..incog
            };
            let grant2 =
                add_allow_always_for_call("write", &args, normal).expect("persist normal grant");
            assert_eq!(grant2.scope, AllowScope::Project);
            clear_caches_for_tests();
        });
    }

    #[test]
    fn builds_workspace_path_rule_for_relative_write() {
        let ctx = GrantContext {
            project_id: Some("proj"),
            default_path: Some("/tmp/work"),
            ..Default::default()
        };
        let rule = rule_for_call("write", &json!({"path": "src/lib.rs"}), &ctx);
        assert!(rule.matches_with_default_path(
            "write",
            &json!({"path": "src/main.rs"}),
            Some(Path::new("/tmp/work"))
        ));
        assert!(!rule.matches_with_default_path(
            "write",
            &json!({"path": "/tmp/other/main.rs"}),
            Some(Path::new("/tmp/work"))
        ));
    }

    #[test]
    fn apply_patch_path_rule_checks_patch_directives() {
        let ctx = GrantContext {
            project_id: Some("proj"),
            default_path: Some("/tmp/work"),
            ..Default::default()
        };
        let rule = rule_for_call(
            crate::tools::TOOL_APPLY_PATCH,
            &json!({"input": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n*** End Patch\n"}),
            &ctx,
        );
        assert!(rule.matches_with_default_path(
            crate::tools::TOOL_APPLY_PATCH,
            &json!({"input": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n*** End Patch\n"}),
            Some(Path::new("/tmp/work"))
        ));
        assert!(!rule.matches_with_default_path(
            crate::tools::TOOL_APPLY_PATCH,
            &json!({"input": "*** Begin Patch\n*** Update File: /tmp/other/main.rs\n@@\n*** End Patch\n"}),
            Some(Path::new("/tmp/work"))
        ));
    }

    #[test]
    fn builds_action_op_rule_for_mac_control() {
        let ctx = GrantContext::default();
        let rule = rule_for_call(
            crate::tools::TOOL_MAC_CONTROL,
            &json!({"action": "act", "op": "click", "x": 10, "y": 20}),
            &ctx,
        );
        assert!(rule.matches(
            crate::tools::TOOL_MAC_CONTROL,
            &json!({"action": "act", "op": "click", "x": 99, "y": 100})
        ));
        assert!(!rule.matches(
            crate::tools::TOOL_MAC_CONTROL,
            &json!({"action": "act", "op": "type", "text": "hello"})
        ));
    }
}
