//! Section-scoped settings reset.
//!
//! The settings UI owns several pages that mix ordinary preferences with
//! user-created resources.  This module is the single source of truth for
//! restoring the former without deleting the latter.  In particular, no
//! scope below mutates LLM providers or global model selection.

use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::permission::{dangerous_commands, edit_commands, protected_paths};
use crate::tools::web_search;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsResetScope {
    General,
    Tools,
    Memory,
    Knowledge,
    Design,
    Chat,
    Cron,
    Plan,
    Recap,
    Server,
    Files,
    Sandbox,
    Browser,
    Acp,
    Notifications,
    Approval,
    Security,
    Logs,
}

impl SettingsResetScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Tools => "tools",
            Self::Memory => "memory",
            Self::Knowledge => "knowledge",
            Self::Design => "design",
            Self::Chat => "chat",
            Self::Cron => "cron",
            Self::Plan => "plan",
            Self::Recap => "recap",
            Self::Server => "server",
            Self::Files => "files",
            Self::Sandbox => "sandbox",
            Self::Browser => "browser",
            Self::Acp => "acp",
            Self::Notifications => "notifications",
            Self::Approval => "approval",
            Self::Security => "security",
            Self::Logs => "logs",
        }
    }
}

impl FromStr for SettingsResetScope {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "general" => Self::General,
            "tools" => Self::Tools,
            "memory" => Self::Memory,
            "knowledge" => Self::Knowledge,
            "design" => Self::Design,
            "chat" => Self::Chat,
            "cron" => Self::Cron,
            "plan" => Self::Plan,
            "recap" => Self::Recap,
            "server" => Self::Server,
            "files" => Self::Files,
            "sandbox" => Self::Sandbox,
            "browser" => Self::Browser,
            "acp" => Self::Acp,
            "notifications" => Self::Notifications,
            "approval" => Self::Approval,
            "security" => Self::Security,
            "logs" => Self::Logs,
            _ => return Err(anyhow!("unknown settings reset scope: {value}")),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsResetSection {
    GeneralAppearance,
    GeneralSystem,
    GeneralNetwork,
    ToolsGeneral,
    ToolsWebSearch,
    ToolsWebFetch,
    ToolsMediaGen,
    ToolsCanvas,
    ToolsAsyncTools,
    ToolsIssueReporting,
    ChatBasic,
    ChatAwareness,
    ChatContextCompact,
    SecurityDangerous,
    SecuritySsrf,
    NotificationsGlobal,
    NotificationsStartup,
    MemoryExtract,
    MemoryRecallSummary,
    MemoryBudget,
    MemoryRetrieval,
    MemoryDreaming,
    KnowledgeCompile,
    KnowledgeVision,
    KnowledgeNoteTools,
    KnowledgeSearch,
    KnowledgePassiveRecall,
    KnowledgeSourceLimits,
    KnowledgeMediaRetention,
    KnowledgeMaintenance,
    KnowledgeSprite,
    ApprovalProtectedPaths,
    ApprovalEditCommands,
    ApprovalDangerousCommands,
}

impl SettingsResetSection {
    pub fn parse(scope: SettingsResetScope, value: &str) -> Result<Self> {
        let section = match (scope, value) {
            (SettingsResetScope::General, "appearance") => Self::GeneralAppearance,
            (SettingsResetScope::General, "system") => Self::GeneralSystem,
            (SettingsResetScope::General, "network") => Self::GeneralNetwork,
            (SettingsResetScope::Tools, "general") => Self::ToolsGeneral,
            (SettingsResetScope::Tools, "web_search") => Self::ToolsWebSearch,
            (SettingsResetScope::Tools, "web_fetch") => Self::ToolsWebFetch,
            (SettingsResetScope::Tools, "media_gen") => Self::ToolsMediaGen,
            (SettingsResetScope::Tools, "canvas") => Self::ToolsCanvas,
            (SettingsResetScope::Tools, "async_tools") => Self::ToolsAsyncTools,
            (SettingsResetScope::Tools, "issue_reporting") => Self::ToolsIssueReporting,
            (SettingsResetScope::Chat, "basic") => Self::ChatBasic,
            (SettingsResetScope::Chat, "awareness") => Self::ChatAwareness,
            (SettingsResetScope::Chat, "context_compact") => Self::ChatContextCompact,
            (SettingsResetScope::Security, "dangerous") => Self::SecurityDangerous,
            (SettingsResetScope::Security, "ssrf") => Self::SecuritySsrf,
            (SettingsResetScope::Notifications, "global") => Self::NotificationsGlobal,
            (SettingsResetScope::Notifications, "startup") => Self::NotificationsStartup,
            (SettingsResetScope::Memory, "extract") => Self::MemoryExtract,
            (SettingsResetScope::Memory, "recall_summary") => Self::MemoryRecallSummary,
            (SettingsResetScope::Memory, "budget") => Self::MemoryBudget,
            (SettingsResetScope::Memory, "retrieval") => Self::MemoryRetrieval,
            (SettingsResetScope::Memory, "dreaming") => Self::MemoryDreaming,
            (SettingsResetScope::Knowledge, "compile") => Self::KnowledgeCompile,
            (SettingsResetScope::Knowledge, "vision") => Self::KnowledgeVision,
            (SettingsResetScope::Knowledge, "note_tools") => Self::KnowledgeNoteTools,
            (SettingsResetScope::Knowledge, "search") => Self::KnowledgeSearch,
            (SettingsResetScope::Knowledge, "passive_recall") => Self::KnowledgePassiveRecall,
            (SettingsResetScope::Knowledge, "source_limits") => Self::KnowledgeSourceLimits,
            (SettingsResetScope::Knowledge, "media_retention") => Self::KnowledgeMediaRetention,
            (SettingsResetScope::Knowledge, "maintenance") => Self::KnowledgeMaintenance,
            (SettingsResetScope::Knowledge, "sprite") => Self::KnowledgeSprite,
            (SettingsResetScope::Approval, "protected_paths") => Self::ApprovalProtectedPaths,
            (SettingsResetScope::Approval, "edit_commands") => Self::ApprovalEditCommands,
            (SettingsResetScope::Approval, "dangerous_commands") => Self::ApprovalDangerousCommands,
            _ => {
                return Err(anyhow!(
                    "unknown settings reset section '{value}' for scope '{}'",
                    scope.as_str()
                ));
            }
        };
        Ok(section)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GeneralAppearance => "appearance",
            Self::GeneralSystem => "system",
            Self::GeneralNetwork => "network",
            Self::ToolsGeneral => "general",
            Self::ToolsWebSearch => "web_search",
            Self::ToolsWebFetch => "web_fetch",
            Self::ToolsMediaGen => "media_gen",
            Self::ToolsCanvas => "canvas",
            Self::ToolsAsyncTools => "async_tools",
            Self::ToolsIssueReporting => "issue_reporting",
            Self::ChatBasic => "basic",
            Self::ChatAwareness => "awareness",
            Self::ChatContextCompact => "context_compact",
            Self::SecurityDangerous => "dangerous",
            Self::SecuritySsrf => "ssrf",
            Self::NotificationsGlobal => "global",
            Self::NotificationsStartup => "startup",
            Self::MemoryExtract => "extract",
            Self::MemoryRecallSummary => "recall_summary",
            Self::MemoryBudget => "budget",
            Self::MemoryRetrieval => "retrieval",
            Self::MemoryDreaming => "dreaming",
            Self::KnowledgeCompile => "compile",
            Self::KnowledgeVision => "vision",
            Self::KnowledgeNoteTools => "note_tools",
            Self::KnowledgeSearch => "search",
            Self::KnowledgePassiveRecall => "passive_recall",
            Self::KnowledgeSourceLimits => "source_limits",
            Self::KnowledgeMediaRetention => "media_retention",
            Self::KnowledgeMaintenance => "maintenance",
            Self::KnowledgeSprite => "sprite",
            Self::ApprovalProtectedPaths => "protected_paths",
            Self::ApprovalEditCommands => "edit_commands",
            Self::ApprovalDangerousCommands => "dangerous_commands",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsResetTarget {
    scope: SettingsResetScope,
    section: Option<SettingsResetSection>,
}

impl SettingsResetTarget {
    fn parse(scope: SettingsResetScope, section: Option<&str>) -> Result<Self> {
        Ok(Self {
            scope,
            section: section
                .map(|value| SettingsResetSection::parse(scope, value))
                .transpose()?,
        })
    }

    fn category(self) -> String {
        match self.section {
            Some(section) => format!(
                "settings_reset.{}.{}",
                self.scope.as_str(),
                section.as_str()
            ),
            None => format!("settings_reset.{}", self.scope.as_str()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResetResult {
    pub scope: SettingsResetScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub changed: bool,
    pub reindex_started: bool,
    pub warning_codes: Vec<String>,
}

static RESET_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn json_changed<T: Serialize>(before: &T, after: &T) -> Result<bool> {
    Ok(serde_json::to_value(before)? != serde_json::to_value(after)?)
}

fn reset_web_search(current: &web_search::WebSearchConfig) -> web_search::WebSearchConfig {
    let mut defaults = web_search::WebSearchConfig {
        searxng_docker_managed: current.searxng_docker_managed,
        ..Default::default()
    };

    for entry in &mut defaults.providers {
        if let Some(saved) = current.providers.iter().find(|item| item.id == entry.id) {
            entry.api_key = saved.api_key.clone();
            entry.api_key2 = saved.api_key2.clone();
            entry.base_url = saved.base_url.clone();
        }
    }
    defaults
}

fn reset_media_gen(config: &mut AppConfig) {
    // Reset behavior (chains + defaults) only; providers carry user
    // credentials and survive a reset — same contract as the old per-slot
    // reset that preserved api_key/base_url.
    config.media_gen.chains = Default::default();
    config.media_gen.image_defaults = Default::default();
    config.media_gen.audio_defaults = Default::default();
}

fn reset_memory_budget(config: &mut AppConfig, defaults: &AppConfig) {
    config.memory_budget = defaults.memory_budget.clone();

    config.memory.core.total_tokens = defaults.memory.core.total_tokens;
    config.memory.core.hard_max_tokens = defaults.memory.core.hard_max_tokens;
    config.memory.core.global_tokens = defaults.memory.core.global_tokens;
    config.memory.core.agent_tokens = defaults.memory.core.agent_tokens;
    config.memory.core.project_tokens = defaults.memory.core.project_tokens;
    config.memory.core.protocol_tokens = defaults.memory.core.protocol_tokens;
    config.memory.core.topic_read_max_tokens = defaults.memory.core.topic_read_max_tokens;

    config.memory.recall.max_tokens = defaults.memory.recall.max_tokens;
    config.memory.recall.max_selected = defaults.memory.recall.max_selected;
    config.memory.recall.candidate_limit = defaults.memory.recall.candidate_limit;
    config.memory.recall.timeout_ms = defaults.memory.recall.timeout_ms;

    config.memory.deep_recall.budget_tokens = defaults.memory.deep_recall.budget_tokens;
    config.memory.deep_recall.timeout_ms = defaults.memory.deep_recall.timeout_ms;
    config.memory.deep_recall.cache_ttl_secs = defaults.memory.deep_recall.cache_ttl_secs;
    config.memory.deep_recall.max_chars = defaults.memory.deep_recall.max_chars;
}

fn apply_app_target(config: &mut AppConfig, target: SettingsResetTarget) {
    let defaults = AppConfig::default();
    if let Some(section) = target.section {
        match section {
            SettingsResetSection::GeneralAppearance => {
                config.theme = defaults.theme;
                config.enhanced_focus_indicators = defaults.enhanced_focus_indicators;
                config.language = defaults.language;
                config.ui_effects_enabled = defaults.ui_effects_enabled;
                config.sidebar_ui_mode = defaults.sidebar_ui_mode;
            }
            SettingsResetSection::GeneralSystem => {
                config.prevent_sleep = defaults.prevent_sleep;
                config.shortcuts = defaults.shortcuts;
            }
            SettingsResetSection::GeneralNetwork => {
                let proxy_url = config.proxy.url.clone();
                config.proxy = defaults.proxy;
                config.proxy.url = proxy_url;
            }
            SettingsResetSection::ToolsGeneral => {
                config.image = defaults.image;
                config.pdf = defaults.pdf;
                config.tool_timeout = defaults.tool_timeout;
                config.timeout_policy = defaults.timeout_policy;
                config.tool_result_disk_threshold = defaults.tool_result_disk_threshold;
                config.deferred_tools = defaults.deferred_tools;
                config.permission.approval_timeout_enabled =
                    defaults.permission.approval_timeout_enabled;
                config.permission.approval_timeout_secs = defaults.permission.approval_timeout_secs;
                config.permission.approval_timeout_action =
                    defaults.permission.approval_timeout_action;
            }
            SettingsResetSection::ToolsWebSearch => {
                config.web_search = reset_web_search(&config.web_search);
            }
            SettingsResetSection::ToolsWebFetch => config.web_fetch = defaults.web_fetch,
            SettingsResetSection::ToolsMediaGen => {
                reset_media_gen(config);
            }
            SettingsResetSection::ToolsCanvas => config.canvas = defaults.canvas,
            SettingsResetSection::ToolsAsyncTools => config.async_tools = defaults.async_tools,
            SettingsResetSection::ToolsIssueReporting => {
                config.issue_reporting = defaults.issue_reporting;
            }
            SettingsResetSection::ChatBasic => {
                config.session_title = defaults.session_title;
                config.tool_call_narration_enabled = defaults.tool_call_narration_enabled;
            }
            SettingsResetSection::ChatAwareness => config.awareness = defaults.awareness,
            SettingsResetSection::ChatContextCompact => config.compact = defaults.compact,
            SettingsResetSection::SecurityDangerous => config.permission.global_yolo = false,
            SettingsResetSection::SecuritySsrf => config.ssrf = defaults.ssrf,
            SettingsResetSection::NotificationsGlobal => {
                config.notification = defaults.notification;
            }
            SettingsResetSection::NotificationsStartup => {
                config.startup_notification = defaults.startup_notification;
            }
            SettingsResetSection::MemoryExtract => config.memory_extract = defaults.memory_extract,
            SettingsResetSection::MemoryRecallSummary => {
                config.recall_summary = defaults.recall_summary;
            }
            SettingsResetSection::MemoryBudget => reset_memory_budget(config, &defaults),
            SettingsResetSection::MemoryRetrieval => {
                config.memory_selection = defaults.memory_selection;
                config.dedup = defaults.dedup;
                config.hybrid_search = defaults.hybrid_search;
                config.temporal_decay = defaults.temporal_decay;
                config.mmr = defaults.mmr;
                config.multimodal = defaults.multimodal;
                config.embedding_cache = defaults.embedding_cache;
            }
            SettingsResetSection::MemoryDreaming => config.dreaming = defaults.dreaming,
            SettingsResetSection::KnowledgeCompile => {
                config.knowledge_compile = defaults.knowledge_compile;
            }
            SettingsResetSection::KnowledgeVision => {
                config.knowledge_vision = defaults.knowledge_vision;
            }
            SettingsResetSection::KnowledgeNoteTools => config.note_tools = defaults.note_tools,
            SettingsResetSection::KnowledgeSearch => {
                config.knowledge_search = defaults.knowledge_search;
            }
            SettingsResetSection::KnowledgePassiveRecall => {
                config.knowledge_passive_recall = defaults.knowledge_passive_recall;
            }
            SettingsResetSection::KnowledgeSourceLimits => {
                config.knowledge_source_limits = defaults.knowledge_source_limits;
            }
            SettingsResetSection::KnowledgeMediaRetention => {
                config.knowledge_media_retention = defaults.knowledge_media_retention;
            }
            SettingsResetSection::KnowledgeMaintenance => {
                config.knowledge_maintenance = defaults.knowledge_maintenance;
            }
            SettingsResetSection::KnowledgeSprite => config.sprite = defaults.sprite,
            SettingsResetSection::ApprovalProtectedPaths
            | SettingsResetSection::ApprovalEditCommands
            | SettingsResetSection::ApprovalDangerousCommands => {}
        }
        return;
    }

    match target.scope {
        SettingsResetScope::General => {
            config.theme = defaults.theme;
            config.enhanced_focus_indicators = defaults.enhanced_focus_indicators;
            config.language = defaults.language;
            config.ui_effects_enabled = defaults.ui_effects_enabled;
            config.prevent_sleep = defaults.prevent_sleep;
            config.sidebar_ui_mode = defaults.sidebar_ui_mode;
            let proxy_url = config.proxy.url.clone();
            config.proxy = defaults.proxy;
            config.proxy.url = proxy_url;
            config.shortcuts = defaults.shortcuts;
        }
        SettingsResetScope::Tools => {
            config.web_search = reset_web_search(&config.web_search);
            config.web_fetch = defaults.web_fetch;
            reset_media_gen(config);
            config.issue_reporting = defaults.issue_reporting;
            config.canvas = defaults.canvas;
            config.image = defaults.image;
            config.pdf = defaults.pdf;
            config.tool_timeout = defaults.tool_timeout;
            config.timeout_policy = defaults.timeout_policy;
            config.tool_result_disk_threshold = defaults.tool_result_disk_threshold;
            config.deferred_tools = defaults.deferred_tools;
            config.async_tools = defaults.async_tools;
            config.permission.approval_timeout_enabled =
                defaults.permission.approval_timeout_enabled;
            config.permission.approval_timeout_secs = defaults.permission.approval_timeout_secs;
            config.permission.approval_timeout_action = defaults.permission.approval_timeout_action;
        }
        SettingsResetScope::Memory => {
            config.memory_embedding = defaults.memory_embedding;
            config.memory_extract = defaults.memory_extract;
            config.memory = defaults.memory;
            config.memory_selection = defaults.memory_selection;
            config.memory_budget = defaults.memory_budget;
            config.dedup = defaults.dedup;
            config.hybrid_search = defaults.hybrid_search;
            config.temporal_decay = defaults.temporal_decay;
            config.mmr = defaults.mmr;
            config.multimodal = defaults.multimodal;
            config.embedding_cache = defaults.embedding_cache;
            config.dreaming = defaults.dreaming;
            config.recall_summary = defaults.recall_summary;
        }
        SettingsResetScope::Knowledge => {
            config.knowledge_embedding = defaults.knowledge_embedding;
            config.knowledge_chunk = defaults.knowledge_chunk;
            config.knowledge_search = defaults.knowledge_search;
            config.knowledge_compile = defaults.knowledge_compile;
            config.knowledge_maintenance = defaults.knowledge_maintenance;
            config.knowledge_vision = defaults.knowledge_vision;
            config.note_tools = defaults.note_tools;
            config.knowledge_passive_recall = defaults.knowledge_passive_recall;
            config.knowledge_media_retention = defaults.knowledge_media_retention;
            config.knowledge_source_limits = defaults.knowledge_source_limits;
            config.sprite = defaults.sprite;
        }
        SettingsResetScope::Design => {
            let last_model = config.design.last_model.clone();
            config.design = defaults.design;
            config.design.last_model = last_model;
        }
        SettingsResetScope::Chat => {
            config.compact = defaults.compact;
            config.session_title = defaults.session_title;
            config.awareness = defaults.awareness;
            config.tool_call_narration_enabled = defaults.tool_call_narration_enabled;
        }
        SettingsResetScope::Cron => config.cron = defaults.cron,
        SettingsResetScope::Plan => {
            config.plan_subagent = defaults.plan_subagent;
            config.ask_user_question_timeout_enabled = defaults.ask_user_question_timeout_enabled;
            config.ask_user_question_timeout_secs = defaults.ask_user_question_timeout_secs;
        }
        SettingsResetScope::Recap => config.recap = defaults.recap,
        SettingsResetScope::Server => {
            let api_key = config.server.api_key.clone();
            let knowledge_agent_read_token = config.server.knowledge_agent_read_token.clone();
            let public_base_url = config.server.public_base_url.clone();
            config.server = defaults.server;
            config.server.api_key = api_key;
            config.server.knowledge_agent_read_token = knowledge_agent_read_token;
            config.server.public_base_url = public_base_url;
        }
        SettingsResetScope::Files => {
            let allow_remote_writes = config.filesystem.allow_remote_writes;
            config.filesystem = defaults.filesystem;
            config.filesystem.allow_remote_writes = allow_remote_writes;
        }
        SettingsResetScope::Browser => {
            if let Some(current) = config.browser.as_ref() {
                let extension = current.extension.as_ref().map(|saved_extension| {
                    crate::browser::extension::BrowserExtensionConfig {
                        native_host_name: saved_extension.native_host_name.clone(),
                        extension_ids: saved_extension.extension_ids.clone(),
                        store_url: saved_extension.store_url.clone(),
                        ..Default::default()
                    }
                });
                let next = crate::browser::BrowserConfig {
                    profiles: current.profiles.clone(),
                    extension,
                    ..Default::default()
                };
                config.browser = Some(next);
            }
        }
        SettingsResetScope::Acp => {
            let backends = config.acp_control.backends.clone();
            config.acp_control = defaults.acp_control;
            config.acp_control.backends = backends;
        }
        SettingsResetScope::Notifications => {
            config.notification = defaults.notification;
            config.startup_notification = defaults.startup_notification;
        }
        SettingsResetScope::Approval => config.permission = defaults.permission,
        SettingsResetScope::Security => {
            config.permission.global_yolo = false;
            config.ssrf = defaults.ssrf;
        }
        SettingsResetScope::Sandbox | SettingsResetScope::Logs => {}
    }
}

#[cfg(test)]
fn apply_app_scope(config: &mut AppConfig, scope: SettingsResetScope) {
    apply_app_target(
        config,
        SettingsResetTarget {
            scope,
            section: None,
        },
    );
}

struct UserScopeReset {
    before: crate::user_config::UserConfig,
    after: crate::user_config::UserConfig,
    changed: bool,
}

fn apply_user_target(config: &mut crate::user_config::UserConfig, target: SettingsResetTarget) {
    let defaults = crate::user_config::UserConfig::default();
    if let Some(section) = target.section {
        match section {
            SettingsResetSection::GeneralAppearance => {
                config.chat_display_mode = defaults.chat_display_mode;
            }
            SettingsResetSection::ChatBasic => {
                config.auto_send_pending = defaults.auto_send_pending;
                config.auto_expand_thinking = defaults.auto_expand_thinking;
                config.auto_collapse_completed_turns = defaults.auto_collapse_completed_turns;
            }
            _ => {}
        }
        return;
    }

    match target.scope {
        SettingsResetScope::General => config.chat_display_mode = defaults.chat_display_mode,
        SettingsResetScope::Chat => {
            config.auto_send_pending = defaults.auto_send_pending;
            config.auto_expand_thinking = defaults.auto_expand_thinking;
            config.auto_collapse_completed_turns = defaults.auto_collapse_completed_turns;
        }
        SettingsResetScope::Server => config.server_mode = defaults.server_mode,
        _ => {}
    }
}

#[cfg(test)]
fn apply_user_scope(config: &mut crate::user_config::UserConfig, scope: SettingsResetScope) {
    apply_user_target(
        config,
        SettingsResetTarget {
            scope,
            section: None,
        },
    );
}

fn prepare_user_target(target: SettingsResetTarget) -> Result<Option<UserScopeReset>> {
    if !matches!(
        (target.scope, target.section),
        (SettingsResetScope::General, None)
            | (
                SettingsResetScope::General,
                Some(SettingsResetSection::GeneralAppearance)
            )
            | (SettingsResetScope::Chat, None)
            | (
                SettingsResetScope::Chat,
                Some(SettingsResetSection::ChatBasic)
            )
            | (SettingsResetScope::Server, None)
    ) {
        return Ok(None);
    }

    let current = crate::user_config::load_user_config()?;
    let mut next = current.clone();
    apply_user_target(&mut next, target);
    Ok(Some(UserScopeReset {
        changed: json_changed(&current, &next)?,
        before: current,
        after: next,
    }))
}

struct PermissionListsSnapshot {
    protected: Vec<String>,
    dangerous: Vec<String>,
    edit: Vec<String>,
}

fn current_permission_lists() -> PermissionListsSnapshot {
    PermissionListsSnapshot {
        protected: (*protected_paths::current_patterns()).clone(),
        dangerous: (*dangerous_commands::current_patterns()).clone(),
        edit: (*edit_commands::current_patterns()).clone(),
    }
}

fn restore_permission_lists(snapshot: &PermissionListsSnapshot) -> Result<()> {
    protected_paths::save_patterns(&snapshot.protected)?;
    dangerous_commands::save_patterns(&snapshot.dangerous)?;
    edit_commands::save_patterns(&snapshot.edit)?;
    Ok(())
}

fn reset_permission_lists(
    section: Option<SettingsResetSection>,
) -> Result<(bool, PermissionListsSnapshot)> {
    let before = current_permission_lists();

    let protected_defaults = protected_paths::defaults()
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let dangerous_defaults = dangerous_commands::defaults()
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let edit_defaults = edit_commands::defaults()
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();

    let reset_protected = matches!(
        section,
        None | Some(SettingsResetSection::ApprovalProtectedPaths)
    );
    let reset_dangerous = matches!(
        section,
        None | Some(SettingsResetSection::ApprovalDangerousCommands)
    );
    let reset_edit = matches!(
        section,
        None | Some(SettingsResetSection::ApprovalEditCommands)
    );
    let changed = (reset_protected && before.protected != protected_defaults)
        || (reset_dangerous && before.dangerous != dangerous_defaults)
        || (reset_edit && before.edit != edit_defaults);

    if let Err(error) = (|| -> Result<()> {
        if reset_protected {
            protected_paths::save_patterns(&protected_defaults)?;
        }
        if reset_dangerous {
            dangerous_commands::save_patterns(&dangerous_defaults)?;
        }
        if reset_edit {
            edit_commands::save_patterns(&edit_defaults)?;
        }
        Ok(())
    })() {
        let _ = restore_permission_lists(&before);
        return Err(error).context("reset approval pattern lists");
    }
    Ok((changed, before))
}

pub fn reset_settings_section(
    scope: SettingsResetScope,
    section: Option<&str>,
    source: &str,
) -> Result<SettingsResetResult> {
    let target = SettingsResetTarget::parse(scope, section)?;
    let _reset_guard = RESET_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow!("settings reset lock is poisoned"))?;
    let mut changed = false;
    let mut warning_codes = Vec::new();
    let result_section = target.section.map(|value| value.as_str().to_string());

    if scope == SettingsResetScope::Sandbox {
        let current = crate::sandbox::load_sandbox_config()?;
        let defaults = crate::sandbox::SandboxConfig::default();
        changed = json_changed(&current, &defaults)?;
        if changed {
            crate::sandbox::save_sandbox_config(&defaults)?;
        }
        return Ok(SettingsResetResult {
            scope,
            section: result_section,
            changed,
            reindex_started: false,
            warning_codes,
        });
    }

    if scope == SettingsResetScope::Logs {
        let current = crate::logging::load_log_config()?;
        let defaults = crate::logging::LogConfig::default();
        changed = json_changed(&current, &defaults)?;
        if changed {
            crate::logging::save_log_config(&defaults)?;
            if let Some(logger) = crate::get_logger() {
                logger.update_config(defaults);
            }
        }
        return Ok(SettingsResetResult {
            scope,
            section: result_section,
            changed,
            reindex_started: false,
            warning_codes,
        });
    }

    let user_reset = prepare_user_target(target)?;
    let user_changed = user_reset.as_ref().is_some_and(|reset| reset.changed);
    if let Some(reset) = user_reset.as_ref().filter(|reset| reset.changed) {
        crate::user_config::save_user_config_to_disk(&reset.after)?;
    }
    changed |= user_changed;

    let permission_reset = if scope == SettingsResetScope::Approval {
        match reset_permission_lists(target.section) {
            Ok(reset) => Some(reset),
            Err(error) => {
                if let Some(reset) = user_reset.as_ref().filter(|reset| reset.changed) {
                    let _ = crate::user_config::save_user_config_to_disk(&reset.before);
                }
                return Err(error);
            }
        }
    } else {
        None
    };
    let list_changed = permission_reset
        .as_ref()
        .is_some_and(|(changed, _)| *changed);
    changed |= list_changed;

    // Always enter mutate_config's process-wide write lock.  Computing
    // `app_changed` from cached_config() before taking the lock lets a
    // concurrent HTTP writer change this scope after the preview and makes a
    // reset incorrectly skip the mutation.  It also makes Knowledge's
    // reindex decision describe a stale candidate rather than the config that
    // was actually committed.
    let category = target.category();
    let mutation = crate::config::mutate_config((&category, source), move |config| {
        let before = serde_json::to_value(&*config)?;
        let resets_knowledge_index =
            scope == SettingsResetScope::Knowledge && target.section.is_none();
        let old_chunk = resets_knowledge_index
            .then(|| serde_json::to_value(&config.knowledge_chunk))
            .transpose()?;
        let old_embedding = resets_knowledge_index
            .then(|| serde_json::to_value(&config.knowledge_embedding))
            .transpose()?;

        apply_app_target(config, target);

        let app_changed = before != serde_json::to_value(&*config)?;
        let knowledge_index_signature_changed = if resets_knowledge_index {
            old_chunk.as_ref() != Some(&serde_json::to_value(&config.knowledge_chunk)?)
                || old_embedding.as_ref()
                    != Some(&serde_json::to_value(&config.knowledge_embedding)?)
        } else {
            false
        };
        Ok((app_changed, knowledge_index_signature_changed))
    });
    let (app_changed, knowledge_index_signature_changed) = match mutation {
        Ok(result) => result,
        Err(error) => {
            if let Some(reset) = user_reset.as_ref().filter(|reset| reset.changed) {
                let _ = crate::user_config::save_user_config_to_disk(&reset.before);
            }
            if let Some((_, snapshot)) = permission_reset.as_ref() {
                let _ = restore_permission_lists(snapshot);
            }
            return Err(error).context("persist section reset");
        }
    };
    changed |= app_changed;

    if scope == SettingsResetScope::Memory && target.section.is_none() && app_changed {
        if let Err(error) = crate::memory::helpers::apply_memory_embedding_from_config(source) {
            crate::app_warn!(
                "settings",
                "reset",
                "memory defaults saved but embedding runtime reload failed: {}",
                error
            );
            warning_codes.push("memory_embedding_reload_failed".to_string());
        }
    }
    if scope == SettingsResetScope::Knowledge && target.section.is_none() && app_changed {
        crate::knowledge::apply_knowledge_embedding_from_config(source);
    }

    let mut reindex_started = false;
    if knowledge_index_signature_changed {
        match crate::get_knowledge_db() {
            Some(registry) => match registry.list_all_ids() {
                Ok(ids) if !ids.is_empty() => {
                    match crate::knowledge::start_knowledge_reembed_job(Some(ids), source) {
                        Ok(_) => reindex_started = true,
                        Err(error) => {
                            crate::app_warn!(
                                "settings",
                                "reset",
                                "knowledge defaults saved but reindex did not start: {}",
                                error
                            );
                            warning_codes.push("knowledge_reindex_not_started".to_string());
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    crate::app_warn!(
                        "settings",
                        "reset",
                        "knowledge defaults saved but KB enumeration failed: {}",
                        error
                    );
                    warning_codes.push("knowledge_reindex_not_started".to_string());
                }
            },
            None => warning_codes.push("knowledge_reindex_not_started".to_string()),
        }
    }

    Ok(SettingsResetResult {
        scope,
        section: result_section,
        changed,
        reindex_started,
        warning_codes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALL_SCOPES: [(SettingsResetScope, &str); 18] = [
        (SettingsResetScope::General, "general"),
        (SettingsResetScope::Tools, "tools"),
        (SettingsResetScope::Memory, "memory"),
        (SettingsResetScope::Knowledge, "knowledge"),
        (SettingsResetScope::Design, "design"),
        (SettingsResetScope::Chat, "chat"),
        (SettingsResetScope::Cron, "cron"),
        (SettingsResetScope::Plan, "plan"),
        (SettingsResetScope::Recap, "recap"),
        (SettingsResetScope::Server, "server"),
        (SettingsResetScope::Files, "files"),
        (SettingsResetScope::Sandbox, "sandbox"),
        (SettingsResetScope::Browser, "browser"),
        (SettingsResetScope::Acp, "acp"),
        (SettingsResetScope::Notifications, "notifications"),
        (SettingsResetScope::Approval, "approval"),
        (SettingsResetScope::Security, "security"),
        (SettingsResetScope::Logs, "logs"),
    ];

    const ALL_SECTIONS: [(SettingsResetScope, &str); 34] = [
        (SettingsResetScope::General, "appearance"),
        (SettingsResetScope::General, "system"),
        (SettingsResetScope::General, "network"),
        (SettingsResetScope::Tools, "general"),
        (SettingsResetScope::Tools, "web_search"),
        (SettingsResetScope::Tools, "web_fetch"),
        (SettingsResetScope::Tools, "media_gen"),
        (SettingsResetScope::Tools, "canvas"),
        (SettingsResetScope::Tools, "async_tools"),
        (SettingsResetScope::Tools, "issue_reporting"),
        (SettingsResetScope::Chat, "basic"),
        (SettingsResetScope::Chat, "awareness"),
        (SettingsResetScope::Chat, "context_compact"),
        (SettingsResetScope::Security, "dangerous"),
        (SettingsResetScope::Security, "ssrf"),
        (SettingsResetScope::Notifications, "global"),
        (SettingsResetScope::Notifications, "startup"),
        (SettingsResetScope::Memory, "extract"),
        (SettingsResetScope::Memory, "recall_summary"),
        (SettingsResetScope::Memory, "budget"),
        (SettingsResetScope::Memory, "retrieval"),
        (SettingsResetScope::Memory, "dreaming"),
        (SettingsResetScope::Knowledge, "compile"),
        (SettingsResetScope::Knowledge, "vision"),
        (SettingsResetScope::Knowledge, "note_tools"),
        (SettingsResetScope::Knowledge, "search"),
        (SettingsResetScope::Knowledge, "passive_recall"),
        (SettingsResetScope::Knowledge, "source_limits"),
        (SettingsResetScope::Knowledge, "media_retention"),
        (SettingsResetScope::Knowledge, "maintenance"),
        (SettingsResetScope::Knowledge, "sprite"),
        (SettingsResetScope::Approval, "protected_paths"),
        (SettingsResetScope::Approval, "edit_commands"),
        (SettingsResetScope::Approval, "dangerous_commands"),
    ];

    fn global_model_snapshot(config: &AppConfig) -> serde_json::Value {
        json!({
            "providers": config.providers,
            "activeModel": config.active_model,
            "fallbackModels": config.fallback_models,
            "temperature": config.temperature,
            "reasoningEffort": config.reasoning_effort,
            "functionModels": config.function_models,
        })
    }

    #[test]
    fn all_scope_names_are_stable_and_serializable() {
        for (scope, name) in ALL_SCOPES {
            assert_eq!(scope.as_str(), name);
            assert_eq!(name.parse::<SettingsResetScope>().unwrap(), scope);
            assert_eq!(serde_json::to_value(scope).unwrap(), json!(name));
        }
        assert!("model_config".parse::<SettingsResetScope>().is_err());
        assert!("providers".parse::<SettingsResetScope>().is_err());
    }

    #[test]
    fn all_section_names_are_stable_and_parent_scoped() {
        for (scope, name) in ALL_SECTIONS {
            let section = SettingsResetSection::parse(scope, name).unwrap();
            assert_eq!(section.as_str(), name);
            assert_eq!(
                SettingsResetTarget::parse(scope, Some(name))
                    .unwrap()
                    .section,
                Some(section)
            );
        }
        assert!(SettingsResetSection::parse(SettingsResetScope::Tools, "appearance").is_err());
        assert!(SettingsResetSection::parse(SettingsResetScope::Knowledge, "chunk").is_err());
        assert!(SettingsResetSection::parse(SettingsResetScope::Memory, "embedding").is_err());
    }

    #[test]
    fn result_shape_matches_both_transport_contracts() {
        let value = serde_json::to_value(SettingsResetResult {
            scope: SettingsResetScope::Knowledge,
            section: None,
            changed: true,
            reindex_started: true,
            warning_codes: vec!["runtime_warning".into()],
        })
        .unwrap();
        assert_eq!(
            value,
            json!({
                "scope": "knowledge",
                "changed": true,
                "reindexStarted": true,
                "warningCodes": ["runtime_warning"],
            })
        );

        let section_value = serde_json::to_value(SettingsResetResult {
            scope: SettingsResetScope::Tools,
            section: Some("web_search".into()),
            changed: false,
            reindex_started: false,
            warning_codes: Vec::new(),
        })
        .unwrap();
        assert_eq!(section_value["section"], json!("web_search"));
    }

    #[test]
    fn every_app_scope_preserves_all_global_model_settings() {
        let mut config = AppConfig::default();
        config.providers.push(crate::provider::ProviderConfig::new(
            "Provider".into(),
            crate::provider::ApiType::OpenaiChat,
            "https://example.com".into(),
            "provider-secret".into(),
        ));
        let primary = crate::provider::ActiveModel {
            provider_id: "provider".into(),
            model_id: "primary".into(),
        };
        config.active_model = Some(primary.clone());
        config.fallback_models.push(crate::provider::ActiveModel {
            provider_id: "fallback".into(),
            model_id: "fallback-model".into(),
        });
        config.temperature = Some(1.25);
        config.reasoning_effort = "high".into();
        config.function_models.vision = Some(primary.clone());
        config.function_models.automation = Some(crate::provider::ModelChain {
            primary,
            fallbacks: Vec::new(),
        });
        let expected = global_model_snapshot(&config);

        for (scope, _) in ALL_SCOPES {
            let mut candidate = config.clone();
            apply_app_scope(&mut candidate, scope);
            assert_eq!(
                global_model_snapshot(&candidate),
                expected,
                "scope {} changed global model settings",
                scope.as_str()
            );
        }
    }

    #[test]
    fn every_app_section_preserves_all_global_model_settings() {
        let mut config = AppConfig::default();
        config.providers.push(crate::provider::ProviderConfig::new(
            "Provider".into(),
            crate::provider::ApiType::OpenaiChat,
            "https://example.com".into(),
            "provider-secret".into(),
        ));
        config.active_model = Some(crate::provider::ActiveModel {
            provider_id: "provider".into(),
            model_id: "primary".into(),
        });
        config.temperature = Some(1.25);
        config.reasoning_effort = "high".into();
        let expected = global_model_snapshot(&config);

        for (scope, name) in ALL_SECTIONS {
            let mut candidate = config.clone();
            apply_app_target(
                &mut candidate,
                SettingsResetTarget::parse(scope, Some(name)).unwrap(),
            );
            assert_eq!(
                global_model_snapshot(&candidate),
                expected,
                "{scope:?}.{name}"
            );
        }
    }

    #[test]
    fn safe_memory_and_knowledge_sections_preserve_index_signatures_and_consent() {
        let mut config = AppConfig::default();
        config.memory_embedding.enabled = true;
        config.memory_embedding.model_config_id = Some("memory-model".into());
        config.knowledge_embedding.enabled = true;
        config.knowledge_embedding.model_config_id = Some("knowledge-model".into());
        config.knowledge_chunk.max_chars = 4321;
        config.memory.recall.enabled = true;
        config.memory.recall.user_configured = true;
        config.memory.deep_recall.enabled = true;
        config.memory.recall.max_tokens = 99;

        let memory_embedding = serde_json::to_value(&config.memory_embedding).unwrap();
        let knowledge_embedding = serde_json::to_value(&config.knowledge_embedding).unwrap();
        let knowledge_chunk = serde_json::to_value(&config.knowledge_chunk).unwrap();

        apply_app_target(
            &mut config,
            SettingsResetTarget::parse(SettingsResetScope::Memory, Some("budget")).unwrap(),
        );
        assert!(config.memory.recall.enabled);
        assert!(config.memory.recall.user_configured);
        assert!(config.memory.deep_recall.enabled);
        assert_eq!(
            config.memory.recall.max_tokens,
            AppConfig::default().memory.recall.max_tokens
        );

        for name in [
            "compile",
            "vision",
            "note_tools",
            "search",
            "passive_recall",
            "source_limits",
            "media_retention",
            "maintenance",
            "sprite",
        ] {
            apply_app_target(
                &mut config,
                SettingsResetTarget::parse(SettingsResetScope::Knowledge, Some(name)).unwrap(),
            );
        }

        assert_eq!(
            serde_json::to_value(&config.memory_embedding).unwrap(),
            memory_embedding
        );
        assert_eq!(
            serde_json::to_value(&config.knowledge_embedding).unwrap(),
            knowledge_embedding
        );
        assert_eq!(
            serde_json::to_value(&config.knowledge_chunk).unwrap(),
            knowledge_chunk
        );
    }

    #[test]
    fn tools_preserve_credentials_and_global_models() {
        let mut config = AppConfig {
            active_model: Some(crate::provider::ActiveModel {
                provider_id: "provider".into(),
                model_id: "model".into(),
            }),
            ..Default::default()
        };
        config.fallback_models.push(crate::provider::ActiveModel {
            provider_id: "fallback".into(),
            model_id: "model".into(),
        });
        let mut provider = crate::media_gen::MediaProviderConfig::new(
            "OpenAI",
            crate::media_gen::MediaVendorKind::Openai,
        );
        provider.api_key = "secret".into();
        config.media_gen.providers.push(provider);
        config.media_gen.image_defaults.timeout_seconds = 999;

        let active = config.active_model.clone();
        let fallbacks = config.fallback_models.clone();
        apply_app_scope(&mut config, SettingsResetScope::Tools);

        assert_eq!(
            config.active_model.as_ref().map(|m| &m.model_id),
            active.as_ref().map(|m| &m.model_id)
        );
        assert_eq!(config.fallback_models.len(), fallbacks.len());
        // Providers (credentials) survive a tools reset; defaults return to
        // factory values.
        assert_eq!(config.media_gen.providers[0].api_key, "secret");
        assert_eq!(
            config.media_gen.image_defaults.timeout_seconds,
            crate::media_gen::ImageGenDefaults::default().timeout_seconds
        );
    }

    #[test]
    fn resource_backed_scopes_keep_resources() {
        let mut config = AppConfig::default();
        config
            .embedding_models
            .push(crate::memory::EmbeddingModelConfig {
                id: "embedding".into(),
                name: "Embedding".into(),
                provider_type: Default::default(),
                api_base_url: Some("https://example.com".into()),
                api_key: Some("secret".into()),
                api_model: Some("embedding-model".into()),
                api_dimensions: Some(1024),
                source: None,
            });
        config.memory_providers.enabled = true;
        config.design.last_model = Some(crate::provider::ActiveModel {
            provider_id: "provider".into(),
            model_id: "model".into(),
        });
        config.browser = Some(crate::browser::BrowserConfig::default());
        config.browser.as_mut().unwrap().profiles.insert(
            "custom".into(),
            crate::browser::BrowserProfileConfig::default(),
        );
        let backends = config.acp_control.backends.clone();

        config.proxy.url = Some("http://127.0.0.1:7890".into());
        config.server.api_key = Some("owner-token".into());
        config.server.knowledge_agent_read_token = Some("reader-token".into());
        config.server.public_base_url = Some("https://agent.example.com".into());
        config.server.bind_addr = "0.0.0.0:9999".into();
        config.filesystem.allow_remote_writes = true;
        config.filesystem.max_chat_attachment_mb = 999;

        apply_app_scope(&mut config, SettingsResetScope::General);
        assert_eq!(config.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));

        apply_app_scope(&mut config, SettingsResetScope::Memory);
        assert_eq!(config.embedding_models.len(), 1);
        assert!(config.memory_providers.enabled);

        apply_app_scope(&mut config, SettingsResetScope::Design);
        assert!(config.design.last_model.is_some());

        apply_app_scope(&mut config, SettingsResetScope::Browser);
        assert!(config
            .browser
            .as_ref()
            .unwrap()
            .profiles
            .contains_key("custom"));

        apply_app_scope(&mut config, SettingsResetScope::Acp);
        assert_eq!(config.acp_control.backends.len(), backends.len());

        apply_app_scope(&mut config, SettingsResetScope::Server);
        assert_eq!(config.server.api_key.as_deref(), Some("owner-token"));
        assert_eq!(
            config.server.knowledge_agent_read_token.as_deref(),
            Some("reader-token")
        );
        assert_eq!(
            config.server.public_base_url.as_deref(),
            Some("https://agent.example.com")
        );
        assert_eq!(
            config.server.bind_addr,
            crate::config::EmbeddedServerConfig::default().bind_addr
        );

        apply_app_scope(&mut config, SettingsResetScope::Files);
        assert!(config.filesystem.allow_remote_writes);
        assert_eq!(
            config.filesystem.max_chat_attachment_mb,
            crate::config::FilesystemConfig::default().max_chat_attachment_mb
        );
    }

    #[test]
    fn user_scopes_preserve_profile_weather_and_remote_credentials() {
        let mut user = crate::user_config::UserConfig {
            name: Some("Ada".into()),
            weather_city: Some("Shanghai".into()),
            remote_server_url: Some("https://remote.example.com".into()),
            remote_api_key: Some("remote-secret".into()),
            auto_send_pending: true,
            auto_expand_thinking: true,
            auto_collapse_completed_turns: false,
            chat_display_mode: Some("timeline".into()),
            server_mode: Some(crate::user_config::SERVER_MODE_REMOTE.into()),
            ..Default::default()
        };

        apply_user_scope(&mut user, SettingsResetScope::Chat);
        assert_eq!(user.name.as_deref(), Some("Ada"));
        assert_eq!(user.weather_city.as_deref(), Some("Shanghai"));
        assert!(!user.auto_send_pending);
        assert!(!user.auto_expand_thinking);
        assert!(user.auto_collapse_completed_turns);
        assert_eq!(user.chat_display_mode.as_deref(), Some("timeline"));

        apply_user_target(
            &mut user,
            SettingsResetTarget::parse(SettingsResetScope::General, Some("appearance")).unwrap(),
        );
        assert_eq!(user.chat_display_mode, None);

        apply_user_scope(&mut user, SettingsResetScope::Server);
        assert_eq!(user.server_mode, None);
        assert_eq!(
            user.remote_server_url.as_deref(),
            Some("https://remote.example.com")
        );
        assert_eq!(user.remote_api_key.as_deref(), Some("remote-secret"));
    }

    #[test]
    fn approval_and_security_have_separate_reset_boundaries() {
        let defaults = AppConfig::default();
        let mut approval = defaults.clone();
        approval.permission.global_yolo = true;
        approval.permission.approval_timeout_enabled = true;
        approval.permission.approval_timeout_secs = 7;
        apply_app_scope(&mut approval, SettingsResetScope::Approval);
        assert_eq!(
            serde_json::to_value(&approval.permission).unwrap(),
            serde_json::to_value(&defaults.permission).unwrap()
        );

        let mut security = defaults.clone();
        security.permission.global_yolo = true;
        security.permission.approval_timeout_enabled = true;
        apply_app_scope(&mut security, SettingsResetScope::Security);
        assert!(!security.permission.global_yolo);
        assert!(security.permission.approval_timeout_enabled);
        assert_eq!(
            serde_json::to_value(&security.ssrf).unwrap(),
            serde_json::to_value(&defaults.ssrf).unwrap()
        );
    }
}
