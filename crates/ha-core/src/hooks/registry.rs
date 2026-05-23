//! Compiled hook registry: `event → [compiled matcher group]`.
//!
//! This phase builds the registry from a [`HooksConfig`] and answers
//! "which handlers match this event/target". The global `ArcSwap` instance and
//! `config:changed` hot-reload subscriber are wired in C8.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;

use super::config::{HookHandlerConfig, HooksConfig};
use super::matcher::{compile, MatcherKind};
use super::types::HookEvent;

/// Process-global compiled registry, hot-swapped on config change.
static HOOK_REGISTRY: OnceLock<ArcSwap<HookRegistry>> = OnceLock::new();

fn cell() -> &'static ArcSwap<HookRegistry> {
    HOOK_REGISTRY.get_or_init(|| ArcSwap::from_pointee(HookRegistry::empty()))
}

/// Load the current global registry. Cheap (an `Arc` clone) — safe on the hot
/// path. A dispatch that holds the returned `Arc` keeps a consistent snapshot
/// even if a concurrent `reload_from_config` swaps in a new one.
pub fn global() -> Arc<HookRegistry> {
    cell().load_full()
}

/// Rebuild the global registry from the current `AppConfig.hooks` (user scope
/// this phase) and atomically swap it in. Called once at startup and on every
/// `config:changed` for a hooks-relevant category.
pub fn reload_from_config() {
    let cfg = crate::config::cached_config();
    let registry = if cfg.disable_all_hooks {
        // Master kill switch: behave as if nothing is configured.
        HookRegistry::empty()
    } else {
        HookRegistry::from_config(&cfg.hooks)
    };
    cell().store(Arc::new(registry));
}

/// A matcher group with its matcher pre-compiled. Handlers in the group share
/// the one matcher (compiled once).
pub struct CompiledGroup {
    pub matcher: MatcherKind,
    pub handlers: Vec<HookHandlerConfig>,
}

/// Per-event compiled hook index.
#[derive(Default)]
pub struct HookRegistry {
    by_event: HashMap<HookEvent, Vec<CompiledGroup>>,
}

const ALL_EVENTS: [HookEvent; 28] = [
    HookEvent::SessionStart,
    HookEvent::SessionEnd,
    HookEvent::UserPromptSubmit,
    HookEvent::UserPromptExpansion,
    HookEvent::PreToolUse,
    HookEvent::PostToolUse,
    HookEvent::PostToolUseFailure,
    HookEvent::PostToolBatch,
    HookEvent::PermissionRequest,
    HookEvent::PermissionDenied,
    HookEvent::Stop,
    HookEvent::StopFailure,
    HookEvent::PreCompact,
    HookEvent::PostCompact,
    HookEvent::Notification,
    HookEvent::SubagentStart,
    HookEvent::SubagentStop,
    HookEvent::TaskCreated,
    HookEvent::TaskCompleted,
    HookEvent::TeammateIdle,
    HookEvent::ConfigChange,
    HookEvent::CwdChanged,
    HookEvent::FileChanged,
    HookEvent::InstructionsLoaded,
    HookEvent::Elicitation,
    HookEvent::ElicitationResult,
    HookEvent::WorktreeCreate,
    HookEvent::WorktreeRemove,
];

impl HookRegistry {
    /// An empty registry (no hooks configured).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Compile a registry from config. Each event's matcher groups are
    /// compiled once; invalid regex matchers become never-match (logged in
    /// `matcher::compile`).
    pub fn from_config(cfg: &HooksConfig) -> Self {
        let mut by_event = HashMap::new();
        for event in ALL_EVENTS {
            let groups = cfg.groups_for(event);
            if groups.is_empty() {
                continue;
            }
            let compiled: Vec<CompiledGroup> = groups
                .iter()
                .map(|g| CompiledGroup {
                    matcher: compile(g.matcher.as_deref()),
                    handlers: g.hooks.clone(),
                })
                .collect();
            by_event.insert(event, compiled);
        }
        Self { by_event }
    }

    /// True when no event has any configured handler.
    pub fn is_empty(&self) -> bool {
        self.by_event.is_empty()
    }

    /// True when a specific event has at least one configured group. Lets the
    /// hot-path dispatcher skip building a `HookInput` when nothing listens.
    pub fn has_handlers_for(&self, event: HookEvent) -> bool {
        self.by_event.get(&event).is_some_and(|g| !g.is_empty())
    }

    /// All handlers whose group matcher fires for `target`, in config order.
    pub fn matching_handlers(
        &self,
        event: HookEvent,
        target: Option<&str>,
    ) -> Vec<&HookHandlerConfig> {
        let Some(groups) = self.by_event.get(&event) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for g in groups {
            if g.matcher.is_match(target) {
                out.extend(g.handlers.iter());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_from(json: &str) -> HooksConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_registry() {
        let r = HookRegistry::empty();
        assert!(r.is_empty());
        assert!(!r.has_handlers_for(HookEvent::PreToolUse));
        assert!(r
            .matching_handlers(HookEvent::PreToolUse, Some("Bash"))
            .is_empty());
    }

    #[test]
    fn matches_by_tool_name() {
        let cfg = cfg_from(
            r#"{
                "PostToolUse": [
                    { "matcher": "Write|Edit", "hooks": [ {"type":"command","command":"a"} ] },
                    { "matcher": "Bash", "hooks": [ {"type":"command","command":"b"} ] }
                ]
            }"#,
        );
        let r = HookRegistry::from_config(&cfg);
        assert!(r.has_handlers_for(HookEvent::PostToolUse));
        assert!(!r.has_handlers_for(HookEvent::PreToolUse));

        let write_hooks = r.matching_handlers(HookEvent::PostToolUse, Some("Write"));
        assert_eq!(write_hooks.len(), 1);

        let bash_hooks = r.matching_handlers(HookEvent::PostToolUse, Some("Bash"));
        assert_eq!(bash_hooks.len(), 1);

        let read_hooks = r.matching_handlers(HookEvent::PostToolUse, Some("Read"));
        assert!(read_hooks.is_empty());
    }

    #[test]
    fn wildcard_group_fires_for_all() {
        let cfg = cfg_from(
            r#"{ "SessionStart": [ { "hooks": [ {"type":"command","command":"x"} ] } ] }"#,
        );
        let r = HookRegistry::from_config(&cfg);
        assert_eq!(
            r.matching_handlers(HookEvent::SessionStart, Some("startup"))
                .len(),
            1
        );
        // Wildcard fires even with no target.
        assert_eq!(r.matching_handlers(HookEvent::SessionStart, None).len(), 1);
    }

    #[test]
    fn multiple_groups_accumulate() {
        let cfg = cfg_from(
            r#"{
                "PreToolUse": [
                    { "hooks": [ {"type":"command","command":"all"} ] },
                    { "matcher": "Bash", "hooks": [ {"type":"command","command":"bash"} ] }
                ]
            }"#,
        );
        let r = HookRegistry::from_config(&cfg);
        // Wildcard group + Bash group both fire for Bash.
        assert_eq!(
            r.matching_handlers(HookEvent::PreToolUse, Some("Bash"))
                .len(),
            2
        );
        // Only the wildcard group fires for Read.
        assert_eq!(
            r.matching_handlers(HookEvent::PreToolUse, Some("Read"))
                .len(),
            1
        );
    }
}
