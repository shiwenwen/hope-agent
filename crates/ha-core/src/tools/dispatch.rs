//! Tool dispatch — single-source-of-truth for "what happens to this tool".
//!
//! Each LLM request triggers a fresh decision per tool: eager-inject schema,
//! deferred (discoverable via tool_search), hint-only (configure-me banner in
//! system prompt), or hidden. The decision is driven by tool tier + agent
//! per-agent tool switches + global config, with no scattered if-branches in
//! `build_tool_schemas` / `build_tools_section` / `tool_search`.
//!
//! Three-axis decision:
//! 1. **Tier-based default** — Tier 1 always eager; Memory bound to global
//!    long-term memory + agent `memory.enabled`; Mcp bound to per-agent
//!    `mcp_enabled`.
//! 2. **Per-agent override** — for Standard / Configured tools the user can
//!    flip the tier default via `capabilities.tools.allow` / `deny`.
//! 3. **Global provisioning** — Tier 3 also requires the corresponding
//!    provider to be configured globally (search backend, image provider,
//!    canvas enabled, etc.). When the user enabled the toggle but provisioning
//!    is missing, fate is `HintOnly` so the system prompt nudges the user.

use std::sync::LazyLock;

use crate::agent_config::FilterConfig;
use crate::agent_loader::is_main_agent;
use crate::config::AppConfig;

use super::definitions::{CoreSubclass, ToolDefinition, ToolTier};

/// Lightweight slice of agent + global config that the dispatcher needs to
/// reach a fate decision. Decoupled from `AgentConfig` so callers can pass
/// a cached snapshot (see `agent::AgentCapsCache`) without re-loading
/// `agent.json` on every tool-loop iteration.
#[derive(Debug)]
pub struct DispatchContext<'a> {
    pub agent_id: &'a str,
    /// Current session is incognito (`sessions.incognito`).
    pub incognito: bool,
    /// `agent.json` `capabilities.mcpEnabled`
    pub mcp_enabled: bool,
    /// `agent.json` `memory.enabled`
    pub memory_enabled: bool,
    /// Effective per-session read policy. This is separate from learning so a
    /// user can consume existing memories without contributing new ones.
    pub use_memories: bool,
    /// Effective per-session contribution policy.
    pub contribute_to_memories: bool,
    /// `agent.json` `capabilities.tools` (non-Core tool switch overrides)
    pub tools_filter: &'a FilterConfig,
    pub app_config: &'a AppConfig,
}

/// Final disposition of a tool for a particular LLM call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFate {
    /// Schema goes into the LLM tool array, model can call it directly.
    InjectEager,
    /// Schema not in LLM tool array but tool stays discoverable via
    /// `tool_search`. Model must call `tool_search(...)` first to surface
    /// the schema before invoking.
    InjectDeferred,
    /// Schema absent + not searchable, but the system prompt's
    /// `# Unconfigured Capabilities` section nudges the user/model toward
    /// the configuration entry point. Used for Tier 3 tools the user
    /// turned on but hasn't fully provisioned.
    HintOnly { config_hint: &'static str },
    /// Tool is completely absent — neither in schema, nor in tool_search,
    /// nor mentioned in the prompt. Used when:
    /// - Memory is disabled and tool tier is Memory
    /// - MCP is disabled and tool tier is Mcp
    /// - Plan-mode tool is requested but PlanAgentMode is Off
    /// - User explicitly disabled a non-Core tool via `capabilities.tools.deny`
    Hidden,
}

/// Probe whether the global side of a Tier 3 tool's provisioning is ready
/// (search providers configured, canvas/notification enabled flags set,
/// image provider keys present, etc.). Tier 3 tools without a global gate
/// (subagent / acp_spawn) just return true here — the per-agent toggle
/// alone decides them.
pub fn is_globally_configured(name: &str, app_config: &AppConfig) -> bool {
    use crate::tools::{
        TOOL_ACP_SPAWN, TOOL_CANVAS, TOOL_IMAGE_GENERATE, TOOL_SEND_NOTIFICATION, TOOL_SUBAGENT,
        TOOL_WEB_SEARCH,
    };
    match name {
        TOOL_WEB_SEARCH => crate::tools::web_search::has_enabled_provider(&app_config.web_search),
        TOOL_IMAGE_GENERATE => {
            crate::tools::image_generate::resolve_image_gen_config(&app_config.image_generate)
                .is_some()
        }
        TOOL_CANVAS => app_config.canvas.enabled,
        TOOL_SEND_NOTIFICATION => app_config.notification.enabled,
        TOOL_SUBAGENT | TOOL_ACP_SPAWN => true,
        // All `feishu_*` tools share the same provisioning gate — at least
        // one Feishu channel account configured. Falls to HintOnly when the
        // user enabled the agent capability but hasn't added an account.
        n if n.starts_with("feishu_") => crate::tools::feishu::has_any_account_configured(),
        _ => true,
    }
}

/// Resolve whether the user has the tool enabled at the agent level.
///
/// `capabilities.tools.allow` means "explicitly enabled" and `deny` means
/// "explicitly disabled" for non-Core user-toggleable tools. Absence from both
/// lists falls back to the tier default for this agent kind.
fn user_enables_tool(name: &str, tier: &ToolTier, ctx: &DispatchContext) -> bool {
    if ctx.tools_filter.deny.iter().any(|t| t == name) {
        return false;
    }
    if ctx.tools_filter.allow.iter().any(|t| t == name) {
        return true;
    }

    let main = is_main_agent(ctx.agent_id);
    match tier {
        ToolTier::Standard {
            default_for_main,
            default_for_others,
            ..
        } => {
            if main {
                *default_for_main
            } else {
                *default_for_others
            }
        }
        ToolTier::Configured {
            default_for_main,
            default_for_others,
            ..
        } => {
            if main {
                *default_for_main
            } else {
                *default_for_others
            }
        }
        _ => true,
    }
}

/// Whether any built-in tool is configured for deferred loading.
pub fn has_deferred_builtin_tools(app_config: &AppConfig) -> bool {
    match app_config.deferred_tools.effective_mode() {
        crate::config::DeferredToolsMode::Recommended => true,
        crate::config::DeferredToolsMode::Custom => {
            !app_config.deferred_tools.tool_names.is_empty()
        }
        crate::config::DeferredToolsMode::Disabled => false,
    }
}

/// Recommended V2 treats dynamic MCP tools like every other non-bootstrap
/// capability: discoverable by default, but not eager. Custom/disabled
/// built-in policy keeps the existing per-server MCP opt-in semantics.
pub fn should_defer_dynamic_mcp_tool(name: &str, app_config: &AppConfig) -> bool {
    matches!(
        app_config.deferred_tools.effective_mode(),
        crate::config::DeferredToolsMode::Recommended
    ) || crate::mcp::catalog::tool_belongs_to_deferred_server(name, &app_config.mcp_servers)
}

/// Small deterministic first-round set. Everything else remains eligible but
/// moves behind tool_search in recommended mode.
fn is_recommended_eager(name: &str) -> bool {
    use crate::tools::*;
    matches!(
        name,
        TOOL_ASK_USER_QUESTION
            | TOOL_RUNTIME_CANCEL
            | TOOL_SKILL
            | TOOL_READ
            | TOOL_GREP
            | TOOL_EXEC
            | TOOL_APPLY_PATCH
            | TOOL_NOTE_READ
            | TOOL_NOTE_SEARCH
            | TOOL_NOTE_CREATE
            | TOOL_NOTE_PATCH
    )
}

/// Decide whether the tool should be deferred (schema not eagerly sent).
/// Recommended mode keeps a small fixed eager set and moves every other
/// eligible built-in behind discovery. Custom mode uses `toolNames`; V2 lets
/// users place any non-bootstrap Standard/Configured tool there, while the
/// legacy `default_deferred` field remains serialization-compatible metadata.
fn is_deferred(name: &str, tier: &ToolTier, app_config: &AppConfig) -> bool {
    match app_config.deferred_tools.effective_mode() {
        crate::config::DeferredToolsMode::Disabled => return false,
        crate::config::DeferredToolsMode::Recommended => {
            return !is_recommended_eager(name)
                && !matches!(
                    tier,
                    ToolTier::Core {
                        subclass: CoreSubclass::PlanMode
                    }
                );
        }
        crate::config::DeferredToolsMode::Custom => {}
    }
    let supports_deferred = match tier {
        ToolTier::Core { subclass } => {
            !matches!(subclass, CoreSubclass::PlanMode)
                && !matches!(
                    name,
                    crate::tools::TOOL_TOOL_SEARCH
                        | crate::tools::TOOL_ASK_USER_QUESTION
                        | crate::tools::TOOL_RUNTIME_CANCEL
                        | crate::tools::TOOL_SKILL
                )
        }
        ToolTier::Memory => true,
        ToolTier::Standard { .. } | ToolTier::Configured { .. } => true,
        // Dynamic MCP servers have their own per-server deferred switch.
        _ => false,
    };
    supports_deferred
        && app_config
            .deferred_tools
            .tool_names
            .iter()
            .any(|t| t == name)
}

/// Final tier-based decision. Plan-mode handling lives at the call site
/// (it depends on `PlanAgentMode` which isn't a static fact about the tool).
pub fn resolve_tool_fate(def: &ToolDefinition, ctx: &DispatchContext) -> ToolFate {
    let app_config = ctx.app_config;
    match &def.tier {
        ToolTier::Core { subclass } => match subclass {
            // Plan-mode tools are decided by the PlanAgentMode at the call
            // site — not by this dispatcher. The dispatcher hides them
            // unconditionally; `apply_plan_tools` puts them back in.
            CoreSubclass::PlanMode => ToolFate::Hidden,
            // Meta tools include framework primitives (skill, runtime_cancel)
            // plus opt-in feature gates (tool_search, job_status). The latter
            // two are eligible only when their corresponding global switch is
            // on; the Agent may promote job_status for a live session job.
            CoreSubclass::Meta => match def.name.as_str() {
                crate::tools::TOOL_TOOL_SEARCH => {
                    if has_deferred_builtin_tools(app_config)
                        || (ctx.mcp_enabled
                            && app_config.mcp_global.enabled
                            && crate::mcp::catalog::has_deferred_tool_server(
                                &app_config.mcp_servers,
                            ))
                    {
                        ToolFate::InjectEager
                    } else {
                        ToolFate::Hidden
                    }
                }
                crate::tools::TOOL_JOB_STATUS => {
                    if app_config.async_tools.enabled {
                        if matches!(
                            app_config.deferred_tools.effective_mode(),
                            crate::config::DeferredToolsMode::Recommended
                        ) {
                            ToolFate::InjectDeferred
                        } else {
                            ToolFate::InjectEager
                        }
                    } else {
                        ToolFate::Hidden
                    }
                }
                _ => {
                    if is_deferred(&def.name, &def.tier, app_config) {
                        ToolFate::InjectDeferred
                    } else {
                        ToolFate::InjectEager
                    }
                }
            },
            // In recommended V2 mode only the compact bootstrap/hot set stays
            // eager. Core remains non-disableable, but its schema may be loaded
            // on demand; this changes token placement, not capability.
            _ => {
                if is_deferred(&def.name, &def.tier, app_config) {
                    ToolFate::InjectDeferred
                } else {
                    ToolFate::InjectEager
                }
            }
        },
        ToolTier::Memory => {
            let runtime = &ctx.app_config.memory;
            let global_memory_enabled =
                runtime.effective_enabled(ctx.app_config.memory_extract.enabled);
            let is_core_memory_tool = matches!(
                def.name.as_str(),
                crate::tools::TOOL_CORE_MEMORY
                    | crate::tools::TOOL_UPDATE_CORE_MEMORY
                    | crate::tools::TOOL_PROJECT_MEMORY
            );
            let core_enabled = !runtime.rollout.enabled || runtime.core.enabled;
            let session_policy_allows = match def.name.as_str() {
                crate::tools::TOOL_SAVE_MEMORY
                | crate::tools::TOOL_UPDATE_MEMORY
                | crate::tools::TOOL_DELETE_MEMORY
                | crate::tools::TOOL_UPDATE_CORE_MEMORY => ctx.contribute_to_memories,
                crate::tools::TOOL_RECALL_MEMORY | crate::tools::TOOL_MEMORY_GET => {
                    ctx.use_memories
                }
                crate::tools::TOOL_CORE_MEMORY | crate::tools::TOOL_PROJECT_MEMORY => {
                    ctx.use_memories || ctx.contribute_to_memories
                }
                _ => true,
            };
            if !ctx.incognito
                && ctx.memory_enabled
                && global_memory_enabled
                && session_policy_allows
                && (!is_core_memory_tool || core_enabled)
            {
                if is_deferred(&def.name, &def.tier, app_config) {
                    ToolFate::InjectDeferred
                } else {
                    ToolFate::InjectEager
                }
            } else {
                ToolFate::Hidden
            }
        }
        ToolTier::Mcp => {
            if ctx.mcp_enabled && app_config.mcp_global.enabled {
                ToolFate::InjectEager
            } else {
                ToolFate::Hidden
            }
        }
        ToolTier::Standard { .. } => {
            if !user_enables_tool(&def.name, &def.tier, ctx) {
                return ToolFate::Hidden;
            }
            if is_deferred(&def.name, &def.tier, app_config) {
                ToolFate::InjectDeferred
            } else {
                ToolFate::InjectEager
            }
        }
        ToolTier::Configured { config_hint, .. } => {
            if !user_enables_tool(&def.name, &def.tier, ctx) {
                return ToolFate::Hidden;
            }
            if !is_globally_configured(&def.name, app_config) {
                return ToolFate::HintOnly { config_hint };
            }
            if is_deferred(&def.name, &def.tier, app_config) {
                ToolFate::InjectDeferred
            } else {
                ToolFate::InjectEager
            }
        }
    }
}

/// Process-wide cache of every built-in + conditionally-injected tool.
///
/// Hot-path callers (`build_tool_schemas`, `build_full_system_prompt`,
/// `build_tools_section`, `tool_search`) hit this on every LLM round. Each
/// `ToolDefinition` carries a `parameters: Value` JSON tree built via the
/// `json!{...}` macro — re-allocating ~50 of those per round was the
/// largest single source of allocator pressure pre-cache.
///
/// `image_generate`'s description is technically dynamic (lists configured
/// providers), but we cache it with the *default* config here. Call sites
/// that need the runtime-rendered description (only `build_tool_schemas`
/// today) substitute via `get_image_generate_tool_dynamic` at injection
/// time. Every other consumer reads tier metadata only and doesn't care.
static ALL_DISPATCHABLE_TOOLS: LazyLock<Vec<ToolDefinition>> = LazyLock::new(|| {
    use super::definitions::{
        get_acp_spawn_tool, get_available_tools, get_canvas_tool, get_enter_plan_mode_tool,
        get_image_generate_tool_dynamic, get_notification_tool, get_subagent_tool,
        get_submit_plan_tool, get_tool_search_tool, get_web_search_tool,
    };
    let mut tools = get_available_tools();
    tools.extend([
        get_notification_tool(),
        get_subagent_tool(),
        get_image_generate_tool_dynamic(&crate::tools::image_generate::ImageGenConfig::default()),
        get_canvas_tool(),
        get_acp_spawn_tool(),
        get_tool_search_tool(),
        get_web_search_tool(),
        get_enter_plan_mode_tool(),
        get_submit_plan_tool(),
        super::job_status::get_job_status_tool(),
        super::schedule_wakeup::get_schedule_wakeup_tool(),
    ]);
    tools.extend(super::feishu::get_feishu_tools());
    tools
});

/// Borrow the cached static catalog. Consumers that need ownership (e.g.
/// `list_builtin_tools` returning JSON) can `.iter().cloned()`.
pub fn all_dispatchable_tools() -> &'static [ToolDefinition] {
    &ALL_DISPATCHABLE_TOOLS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_config::FilterConfig;
    use crate::agent_loader::DEFAULT_AGENT_ID;

    /// Test fixture — owns the data so each `&` reference in DispatchContext
    /// is valid for the duration of the test.
    struct Fixture {
        filter: FilterConfig,
        app: AppConfig,
        incognito: bool,
        mcp_enabled: bool,
        memory_enabled: bool,
        use_memories: bool,
        contribute_to_memories: bool,
    }

    impl Fixture {
        fn new() -> Self {
            let mut app = AppConfig::default();
            // Most dispatcher fixtures exercise the legacy/custom name-list
            // semantics. Recommended-V2 behavior has dedicated tests below.
            app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Custom);
            Self {
                filter: FilterConfig::default(),
                app,
                incognito: false,
                mcp_enabled: true,
                memory_enabled: true,
                use_memories: true,
                contribute_to_memories: true,
            }
        }

        fn ctx<'a>(&'a self, agent_id: &'a str) -> DispatchContext<'a> {
            DispatchContext {
                agent_id,
                incognito: self.incognito,
                mcp_enabled: self.mcp_enabled,
                memory_enabled: self.memory_enabled,
                use_memories: self.use_memories,
                contribute_to_memories: self.contribute_to_memories,
                tools_filter: &self.filter,
                app_config: &self.app,
            }
        }
    }

    fn def_with_tier(name: &str, tier: ToolTier) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: "test".into(),
            parameters: serde_json::json!({}),
            tier,
            internal: false,
            concurrent_safe: false,
            async_capable: false,
        }
    }

    #[test]
    fn tier_core_filesystem_always_eager() {
        let f = Fixture::new();
        let def = def_with_tier(
            "exec",
            ToolTier::Core {
                subclass: CoreSubclass::FileSystem,
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn tier_core_planmode_hidden_at_dispatcher() {
        let f = Fixture::new();
        let def = def_with_tier(
            "submit_plan",
            ToolTier::Core {
                subclass: CoreSubclass::PlanMode,
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_memory_hidden_when_agent_memory_off() {
        let mut f = Fixture::new();
        f.memory_enabled = false;
        let def = def_with_tier("save_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_memory_hidden_when_global_memory_off() {
        let mut f = Fixture::new();
        f.app.memory.enabled = false;
        let def = def_with_tier("save_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_memory_hidden_when_incognito() {
        let mut f = Fixture::new();
        f.incognito = true;
        let def = def_with_tier("recall_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn builtin_memory_tools_share_memory_tier_gate() {
        let memory_tool_names = [
            crate::tools::TOOL_SAVE_MEMORY,
            crate::tools::TOOL_RECALL_MEMORY,
            crate::tools::TOOL_UPDATE_MEMORY,
            crate::tools::TOOL_DELETE_MEMORY,
            crate::tools::TOOL_UPDATE_CORE_MEMORY,
            crate::tools::TOOL_CORE_MEMORY,
            crate::tools::TOOL_PROJECT_MEMORY,
            crate::tools::TOOL_MEMORY_GET,
        ];

        let mut incognito = Fixture::new();
        incognito.incognito = true;
        let mut memory_off = Fixture::new();
        memory_off.app.memory.enabled = false;

        for name in memory_tool_names {
            let def = all_dispatchable_tools()
                .iter()
                .find(|def| def.name == name)
                .unwrap_or_else(|| panic!("missing built-in memory tool: {name}"));
            assert!(
                matches!(def.tier, ToolTier::Memory),
                "{name} must stay ToolTier::Memory so incognito/off gates apply"
            );
            assert_eq!(
                resolve_tool_fate(def, &incognito.ctx(DEFAULT_AGENT_ID)),
                ToolFate::Hidden,
                "{name} must be hidden in incognito sessions"
            );
            assert_eq!(
                resolve_tool_fate(def, &memory_off.ctx(DEFAULT_AGENT_ID)),
                ToolFate::Hidden,
                "{name} must be hidden when long-term memory is off"
            );
        }
    }

    #[test]
    fn tier_memory_eager_when_enabled() {
        let f = Fixture::new();
        let def = def_with_tier("save_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn memory_read_tools_remain_callable_when_automatic_recall_is_off() {
        let mut f = Fixture::new();
        f.app.memory.recall.enabled = false;

        for name in [
            crate::tools::TOOL_RECALL_MEMORY,
            crate::tools::TOOL_MEMORY_GET,
        ] {
            let def = all_dispatchable_tools()
                .iter()
                .find(|def| def.name == name)
                .unwrap_or_else(|| panic!("missing built-in memory tool: {name}"));
            assert_eq!(
                resolve_tool_fate(def, &f.ctx(DEFAULT_AGENT_ID)),
                ToolFate::InjectEager,
                "automatic recall consent must not hide model-invoked tool {name}"
            );
        }
    }

    #[test]
    fn session_read_and_contribution_policies_hide_only_their_tool_classes() {
        let mut no_read = Fixture::new();
        no_read.use_memories = false;
        let recall = all_dispatchable_tools()
            .iter()
            .find(|def| def.name == crate::tools::TOOL_RECALL_MEMORY)
            .expect("recall_memory definition");
        let save = all_dispatchable_tools()
            .iter()
            .find(|def| def.name == crate::tools::TOOL_SAVE_MEMORY)
            .expect("save_memory definition");
        assert_eq!(
            resolve_tool_fate(recall, &no_read.ctx(DEFAULT_AGENT_ID)),
            ToolFate::Hidden
        );
        assert_ne!(
            resolve_tool_fate(save, &no_read.ctx(DEFAULT_AGENT_ID)),
            ToolFate::Hidden
        );

        let mut no_contribution = Fixture::new();
        no_contribution.contribute_to_memories = false;
        assert_eq!(
            resolve_tool_fate(save, &no_contribution.ctx(DEFAULT_AGENT_ID)),
            ToolFate::Hidden
        );
        assert_ne!(
            resolve_tool_fate(recall, &no_contribution.ctx(DEFAULT_AGENT_ID)),
            ToolFate::Hidden
        );
    }

    #[test]
    fn tier_mcp_hidden_when_agent_mcp_disabled() {
        let mut f = Fixture::new();
        f.mcp_enabled = false;
        let def = def_with_tier("mcp_resource", ToolTier::Mcp);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_mcp_hidden_when_global_mcp_disabled() {
        let mut f = Fixture::new();
        f.app.mcp_global.enabled = false;
        let def = def_with_tier("mcp_prompt", ToolTier::Mcp);
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_standard_default_main_vs_others() {
        let f = Fixture::new();
        let def = def_with_tier(
            "get_settings",
            ToolTier::Standard {
                default_for_main: true,
                default_for_others: false,
                default_deferred: false,
            },
        );
        let main_fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(main_fate, ToolFate::InjectEager);
        let other_fate = resolve_tool_fate(&def, &f.ctx("translator"));
        assert_eq!(other_fate, ToolFate::Hidden);
    }

    #[test]
    fn default_deferred_tool_moves_to_search_pool() {
        let f = Fixture::new();
        let def = def_with_tier(
            crate::tools::TOOL_BROWSER,
            ToolTier::Standard {
                default_for_main: true,
                default_for_others: true,
                default_deferred: true,
            },
        );
        assert_eq!(
            resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectDeferred
        );
    }

    #[test]
    fn default_deferred_requires_configured_tool_name() {
        let f = Fixture::new();
        let def = def_with_tier(
            "custom_low_frequency_tool",
            ToolTier::Standard {
                default_for_main: true,
                default_for_others: true,
                default_deferred: true,
            },
        );
        assert_eq!(
            resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectEager
        );
    }

    #[test]
    fn deferred_tools_can_be_disabled_globally() {
        let mut f = Fixture::new();
        f.app.deferred_tools.enabled = false;
        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Disabled);
        let def = def_with_tier(
            crate::tools::TOOL_BROWSER,
            ToolTier::Standard {
                default_for_main: true,
                default_for_others: true,
                default_deferred: true,
            },
        );
        assert_eq!(
            resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectEager
        );
    }

    #[test]
    fn recommended_mode_defers_non_bootstrap_core_without_hiding_it() {
        let mut f = Fixture::new();
        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Recommended);
        let def = def_with_tier(
            crate::tools::TOOL_SESSIONS_HISTORY,
            ToolTier::Core {
                subclass: CoreSubclass::SessionAware,
            },
        );
        assert_eq!(
            resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectDeferred
        );
    }

    #[test]
    fn recommended_mode_defers_job_status_until_a_session_has_live_work() {
        let mut f = Fixture::new();
        f.app.async_tools.enabled = true;
        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Recommended);
        let def = all_dispatchable_tools()
            .iter()
            .find(|def| def.name == crate::tools::TOOL_JOB_STATUS)
            .expect("job_status definition");
        assert_eq!(
            resolve_tool_fate(def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectDeferred
        );

        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Disabled);
        assert_eq!(
            resolve_tool_fate(def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectEager
        );
    }

    #[test]
    fn recommended_mode_defers_dynamic_mcp_without_per_server_opt_in() {
        let mut app = AppConfig::default();
        app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Recommended);
        assert!(should_defer_dynamic_mcp_tool(
            "mcp__example__large_tool",
            &app
        ));

        app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Disabled);
        assert!(!should_defer_dynamic_mcp_tool(
            "mcp__example__large_tool",
            &app
        ));
    }

    #[test]
    fn recommended_eager_schema_budget_and_capability_partition() {
        let mut f = Fixture::new();
        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Recommended);
        let ctx = f.ctx(DEFAULT_AGENT_ID);
        let mut schema_bytes = 0usize;
        for def in all_dispatchable_tools() {
            match resolve_tool_fate(def, &ctx) {
                ToolFate::InjectEager => {
                    // Canonical empty-session fixture has no attached KB, so
                    // the final live gate removes note_* schemas before the
                    // request is built.
                    if !crate::tools::is_kb_scoped_tool(&def.name) {
                        schema_bytes += serde_json::to_vec(
                            &def.to_provider_schema(crate::tools::ToolProvider::OpenAI),
                        )
                        .unwrap()
                        .len();
                    }
                }
                ToolFate::InjectDeferred | ToolFate::HintOnly { .. } | ToolFate::Hidden => {}
            }
            if matches!(def.tier, ToolTier::Core { subclass } if subclass != CoreSubclass::PlanMode)
            {
                assert!(
                    matches!(
                        resolve_tool_fate(def, &ctx),
                        ToolFate::InjectEager | ToolFate::InjectDeferred
                    ),
                    "enabled core capability {} was hidden",
                    def.name
                );
            }
        }
        assert!(
            schema_bytes / crate::context_compact::CHARS_PER_TOKEN <= 4_000,
            "recommended eager schemas exceed 4k token heuristic: {} bytes",
            schema_bytes
        );
    }

    #[test]
    fn tool_search_is_injected_when_default_deferred_tools_exist() {
        let f = Fixture::new();
        let def = ToolDefinition {
            name: crate::tools::TOOL_TOOL_SEARCH.into(),
            description: "test".into(),
            parameters: serde_json::json!({}),
            tier: ToolTier::Core {
                subclass: CoreSubclass::Meta,
            },
            internal: true,
            concurrent_safe: false,
            async_capable: false,
        };
        assert_eq!(
            resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectEager
        );
    }

    #[test]
    fn mac_control_schema_is_readonly_and_main_agent_default() {
        let def = all_dispatchable_tools()
            .iter()
            .find(|def| def.name == crate::tools::TOOL_MAC_CONTROL)
            .expect("mac_control tool is registered");
        let ToolTier::Standard {
            default_for_main,
            default_for_others,
            default_deferred,
        } = &def.tier
        else {
            panic!("mac_control should be a Standard tool");
        };
        assert!(*default_for_main);
        assert!(!*default_for_others);
        assert!(*default_deferred);
        assert!(!def.internal);
        assert!(!def.concurrent_safe);
        assert!(!def.async_capable);

        let actions = def
            .parameters
            .pointer("/properties/action/enum")
            .and_then(|v| v.as_array())
            .expect("action enum exists")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            vec![
                "status",
                "permissions",
                "diagnostics",
                "snapshot",
                "visual",
                "elements",
                "wait",
                "apps",
                "dock",
                "spaces",
                "windows",
                "act",
                "menu",
                "clipboard",
                "dialog"
            ]
        );
        let ops = def
            .parameters
            .pointer("/properties/op/enum")
            .and_then(|v| v.as_array())
            .expect("op enum exists")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert!(ops.contains(&"summary"));
        assert!(ops.contains(&"export"));
        assert!(ops.contains(&"find"));
        assert!(ops.contains(&"dry_run"));
        assert!(ops.contains(&"perform_action"));
        assert!(ops.contains(&"click"));
        assert!(ops.contains(&"click_point"));
        assert!(ops.contains(&"move_cursor"));
        assert!(ops.contains(&"quit"));
        assert!(ops.contains(&"close"));
        assert!(ops.contains(&"double_click"));
        assert!(ops.contains(&"right_click"));
        assert!(ops.contains(&"type"));
        assert!(ops.contains(&"paste"));
        assert!(ops.contains(&"press"));
        assert!(ops.contains(&"drag"));
        assert!(ops.contains(&"swipe"));
        assert!(ops.contains(&"get"));
        assert!(ops.contains(&"set"));
        assert!(ops.contains(&"clear"));
        assert!(ops.contains(&"inspect"));
        assert!(ops.contains(&"accept"));
        assert!(ops.contains(&"dismiss"));
        let menu_scopes = def
            .parameters
            .pointer("/properties/scope/enum")
            .and_then(|v| v.as_array())
            .expect("menu scope enum exists")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(menu_scopes, vec!["app", "system"]);
        let window_scopes = def
            .parameters
            .pointer("/properties/windowScope/enum")
            .and_then(|v| v.as_array())
            .expect("window scope enum exists")
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(window_scopes, vec!["frontmost", "all"]);
        assert!(def
            .parameters
            .pointer("/properties/includeSnapshot")
            .is_some());

        let f = Fixture::new();
        assert_eq!(
            resolve_tool_fate(def, &f.ctx(DEFAULT_AGENT_ID)),
            ToolFate::InjectDeferred
        );
        assert_eq!(
            resolve_tool_fate(def, &f.ctx("translator")),
            ToolFate::Hidden
        );
    }

    #[test]
    fn tier_standard_user_deny_hides() {
        let mut f = Fixture::new();
        f.filter.deny.push("browser".into());
        let def = def_with_tier(
            "browser",
            ToolTier::Standard {
                default_for_main: true,
                default_for_others: true,
                default_deferred: false,
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_standard_allow_enables_default_off() {
        let mut f = Fixture::new();
        f.filter.allow.push("get_weather".into());
        f.app.deferred_tools.mode = Some(crate::config::DeferredToolsMode::Disabled);
        let def = def_with_tier(
            "get_weather",
            ToolTier::Standard {
                default_for_main: false,
                default_for_others: false,
                default_deferred: false,
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn tier_configured_unconfigured_emits_hint() {
        let mut f = Fixture::new();
        // Clear all providers — default config ships with DuckDuckGo enabled.
        f.app.web_search.providers.iter_mut().for_each(|p| {
            p.enabled = false;
        });
        let def = def_with_tier(
            "web_search",
            ToolTier::Configured {
                default_for_main: true,
                default_for_others: true,
                default_deferred: false,
                config_hint: "Settings → Tools → Web Search",
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert!(matches!(fate, ToolFate::HintOnly { .. }));
    }

    #[test]
    fn tier_core_user_deny_does_not_hide_core_tools() {
        let mut f = Fixture::new();
        f.filter.deny.push("read".into());
        let def = def_with_tier(
            "read",
            ToolTier::Core {
                subclass: CoreSubclass::FileSystem,
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn tier_core_internal_tool_survives_tool_switch_overrides() {
        // Core tools must not be affected by non-Core tool switch overrides.
        let mut f = Fixture::new();
        f.filter.allow.push("read".into());
        let def = ToolDefinition {
            name: "skill".into(),
            description: "test".into(),
            parameters: serde_json::json!({}),
            tier: ToolTier::Core {
                subclass: CoreSubclass::Meta,
            },
            internal: true,
            concurrent_safe: false,
            async_capable: false,
        };
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn tier_configured_user_deny_takes_precedence_over_unconfigured() {
        let mut f = Fixture::new();
        f.filter.deny.push("web_search".into());
        let def = def_with_tier(
            "web_search",
            ToolTier::Configured {
                default_for_main: true,
                default_for_others: true,
                default_deferred: false,
                config_hint: "Settings → Tools → Web Search",
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_configured_allow_enables_default_off() {
        let mut f = Fixture::new();
        f.filter.allow.push("custom_configured".into());
        let def = def_with_tier(
            "custom_configured",
            ToolTier::Configured {
                default_for_main: false,
                default_for_others: false,
                default_deferred: false,
                config_hint: "Settings → Tools",
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx(DEFAULT_AGENT_ID));
        assert_eq!(fate, ToolFate::InjectEager);
    }
}
