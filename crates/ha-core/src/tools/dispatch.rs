//! Tool dispatch — single-source-of-truth for "what happens to this tool".
//!
//! Each LLM request triggers a fresh decision per tool: eager-inject schema,
//! deferred (discoverable via tool_search), hint-only (configure-me banner in
//! system prompt), or hidden. The decision is driven by tool tier + agent
//! capability toggles + global config, with no scattered if-branches in
//! `build_tool_schemas` / `build_tools_section` / `tool_search`.
//!
//! Three-axis decision:
//! 1. **Tier-based default** — Tier 1 always eager; Memory bound to global
//!    `memory.enabled`; Mcp bound to per-agent `mcp_enabled`.
//! 2. **Per-agent override** — for Tier 2 / Tier 3 the user can flip the
//!    default off via `capabilities.tools.deny` (Tier 2) or
//!    `capabilities.capability_toggles.<name>` (Tier 3).
//! 3. **Global provisioning** — Tier 3 also requires the corresponding
//!    provider to be configured globally (search backend, image provider,
//!    canvas enabled, etc.). When the user enabled the toggle but provisioning
//!    is missing, fate is `HintOnly` so the system prompt nudges the user.

use std::sync::LazyLock;

use crate::agent_config::{CapabilityToggles, FilterConfig};
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
    /// `agent.json` `capabilities.mcpEnabled`
    pub mcp_enabled: bool,
    /// `agent.json` `memory.enabled`
    pub memory_enabled: bool,
    /// `agent.json` `capabilities.tools` (allow/deny filter)
    pub tools_filter: &'a FilterConfig,
    /// `agent.json` `capabilities.capabilityToggles`
    pub capability_toggles: &'a CapabilityToggles,
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
    /// - User explicitly denied the tool via `capabilities.tools.deny`
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
        _ => true,
    }
}

/// Resolve whether the user has the tool enabled at the agent level.
///
/// - For Tier 2 (Standard) we use the existing `tools.deny` filter as the
///   on/off switch (user can move tools into deny via the GUI).
/// - For Tier 3 (Configured) we use the dedicated `capability_toggles`
///   field, falling back to tier defaults when unset.
fn user_enables_tool(name: &str, tier: &ToolTier, ctx: &DispatchContext) -> bool {
    let main = is_main_agent(ctx.agent_id);
    match tier {
        ToolTier::Standard {
            default_for_main,
            default_for_others,
            ..
        } => {
            if ctx.tools_filter.deny.iter().any(|t| t == name) {
                return false;
            }
            if ctx.tools_filter.allow.iter().any(|t| t == name) {
                return true;
            }
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
            if let Some(explicit) = ctx.capability_toggles.override_for(name) {
                return explicit;
            }
            if main {
                *default_for_main
            } else {
                *default_for_others
            }
        }
        _ => true,
    }
}

/// Whether any built-in tool is explicitly configured for deferred loading.
pub fn has_deferred_builtin_tools(app_config: &AppConfig) -> bool {
    app_config.deferred_tools.enabled && !app_config.deferred_tools.tool_names.is_empty()
}

/// Decide whether the tool should be deferred (schema not eagerly sent).
/// The global switch only enables the mechanism; individual built-in tools
/// move to the deferred pool only when their name appears in
/// `deferredTools.toolNames`.
fn is_deferred(name: &str, tier: &ToolTier, app_config: &AppConfig) -> bool {
    if !app_config.deferred_tools.enabled {
        return false;
    }
    let supports_deferred = match tier {
        ToolTier::Standard {
            default_deferred, ..
        }
        | ToolTier::Configured {
            default_deferred, ..
        } => *default_deferred,
        // Tier 1 / Memory / Mcp ignore the deferred switch entirely.
        _ => false,
    };
    supports_deferred && app_config.deferred_tools.tool_names.iter().any(|t| t == name)
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
            // two only appear when their corresponding global switch is on.
            CoreSubclass::Meta => match def.name.as_str() {
                crate::tools::TOOL_TOOL_SEARCH => {
                    if has_deferred_builtin_tools(app_config)
                        || (ctx.mcp_enabled
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
                        ToolFate::InjectEager
                    } else {
                        ToolFate::Hidden
                    }
                }
                _ => ToolFate::InjectEager,
            },
            // FileSystem / Interaction / SessionAware — always eager.
            _ => ToolFate::InjectEager,
        },
        ToolTier::Memory => {
            if ctx.memory_enabled {
                ToolFate::InjectEager
            } else {
                ToolFate::Hidden
            }
        }
        ToolTier::Mcp => {
            if ctx.mcp_enabled {
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
        get_acp_spawn_tool, get_amend_plan_tool, get_available_tools, get_canvas_tool,
        get_image_generate_tool_dynamic, get_notification_tool, get_plan_step_tool,
        get_subagent_tool, get_submit_plan_tool, get_tool_search_tool, get_web_search_tool,
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
        get_submit_plan_tool(),
        get_amend_plan_tool(),
        get_plan_step_tool(),
        super::job_status::get_job_status_tool(),
    ]);
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
    use crate::agent_config::{CapabilityToggles, FilterConfig};

    /// Test fixture — owns the data so each `&` reference in DispatchContext
    /// is valid for the duration of the test.
    struct Fixture {
        toggles: CapabilityToggles,
        filter: FilterConfig,
        app: AppConfig,
        mcp_enabled: bool,
        memory_enabled: bool,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                toggles: CapabilityToggles::default(),
                filter: FilterConfig::default(),
                app: AppConfig::default(),
                mcp_enabled: true,
                memory_enabled: true,
            }
        }

        fn ctx<'a>(&'a self, agent_id: &'a str) -> DispatchContext<'a> {
            DispatchContext {
                agent_id,
                mcp_enabled: self.mcp_enabled,
                memory_enabled: self.memory_enabled,
                tools_filter: &self.filter,
                capability_toggles: &self.toggles,
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
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
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
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_memory_hidden_when_agent_memory_off() {
        let mut f = Fixture::new();
        f.memory_enabled = false;
        let def = def_with_tier("save_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(fate, ToolFate::Hidden);
    }

    #[test]
    fn tier_memory_eager_when_enabled() {
        let f = Fixture::new();
        let def = def_with_tier("save_memory", ToolTier::Memory);
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(fate, ToolFate::InjectEager);
    }

    #[test]
    fn tier_mcp_hidden_when_agent_mcp_disabled() {
        let mut f = Fixture::new();
        f.mcp_enabled = false;
        let def = def_with_tier("mcp_resource", ToolTier::Mcp);
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
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
        let main_fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(main_fate, ToolFate::InjectEager);
        let other_fate = resolve_tool_fate(&def, &f.ctx("translator"));
        assert_eq!(other_fate, ToolFate::Hidden);
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
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(fate, ToolFate::Hidden);
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
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert!(matches!(fate, ToolFate::HintOnly { .. }));
    }

    #[test]
    fn tier_configured_user_off_takes_precedence_over_unconfigured() {
        let mut f = Fixture::new();
        f.toggles.web_search = Some(false);
        let def = def_with_tier(
            "web_search",
            ToolTier::Configured {
                default_for_main: true,
                default_for_others: true,
                default_deferred: false,
                config_hint: "Settings → Tools → Web Search",
            },
        );
        let fate = resolve_tool_fate(&def, &f.ctx("default"));
        assert_eq!(fate, ToolFate::Hidden);
    }
}
