use serde::Serialize;
use serde_json::Value;

use super::types::{CoreSubclass, ToolDefinition, ToolTier};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadFileSystem,
    WriteFileSystem,
    ExecuteProcess,
    NetworkAccess,
    BrowserAutomation,
    DesktopAutomation,
    ExternalService,
    ExternalServiceWrite,
    MemoryRead,
    MemoryWrite,
    KnowledgeRead,
    KnowledgeWrite,
    SessionRead,
    SessionWrite,
    SettingsRead,
    SettingsWrite,
    TaskRead,
    TaskWrite,
    GoalRead,
    GoalWrite,
    UserInteraction,
    RuntimeControl,
    AgentDelegation,
    Scheduling,
    MediaRead,
    MediaWrite,
    AppUpdate,
}

impl ToolEffect {
    pub fn as_search_tag(self) -> &'static str {
        match self {
            ToolEffect::ReadFileSystem => "read filesystem file inspect local",
            ToolEffect::WriteFileSystem => "write filesystem edit modify create delete local",
            ToolEffect::ExecuteProcess => "execute process shell command terminal",
            ToolEffect::NetworkAccess => "network web http fetch search url",
            ToolEffect::BrowserAutomation => "browser chrome page automation cdp",
            ToolEffect::DesktopAutomation => "desktop macos ui automation accessibility",
            ToolEffect::ExternalService => "external service integration remote api",
            ToolEffect::ExternalServiceWrite => {
                "external service write create update delete remote api"
            }
            ToolEffect::MemoryRead => "memory recall read search",
            ToolEffect::MemoryWrite => "memory save update delete",
            ToolEffect::KnowledgeRead => "knowledge note read search graph",
            ToolEffect::KnowledgeWrite => "knowledge note write edit link delete",
            ToolEffect::SessionRead => "session history read search status",
            ToolEffect::SessionWrite => "session send message modify",
            ToolEffect::SettingsRead => "settings config read backup",
            ToolEffect::SettingsWrite => "settings config update restore",
            ToolEffect::TaskRead => "task todo progress list read",
            ToolEffect::TaskWrite => "task todo progress create update",
            ToolEffect::GoalRead => "goal objective criteria audit evidence budget read",
            ToolEffect::GoalWrite => {
                "goal checkpoint evidence evaluate finish block progress update"
            }
            ToolEffect::UserInteraction => "ask user question notify attachment",
            ToolEffect::RuntimeControl => "runtime cancel status job process wakeup",
            ToolEffect::AgentDelegation => "agent delegate subagent team acp worker",
            ToolEffect::Scheduling => "schedule cron reminder wakeup",
            ToolEffect::MediaRead => "image pdf media read vision",
            ToolEffect::MediaWrite => "image canvas media generate export",
            ToolEffect::AppUpdate => "app update install upgrade",
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    Low,
    Medium,
    High,
    Strict,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolInterruptBehavior {
    Immediate,
    Graceful,
    LongRunning,
    HumanBlocked,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionSubject {
    Internal,
    LocalFileSystem,
    Process,
    Network,
    Browser,
    Desktop,
    ExternalService,
    Memory,
    Knowledge,
    Session,
    Settings,
    UserInteraction,
    Runtime,
    AgentDelegation,
    Scheduling,
    Media,
    Application,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalHint {
    NeverInternal,
    UsuallyAllow,
    PolicyDependent,
    StrictPossible,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolPermissionMetadata {
    pub subject: ToolPermissionSubject,
    pub approval_hint: ToolApprovalHint,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolInputMetadata {
    pub required: Vec<String>,
    pub strict_schema: bool,
    pub action_param: Option<String>,
    pub path_params: Vec<String>,
    pub command_params: Vec<String>,
    pub url_params: Vec<String>,
    pub id_params: Vec<String>,
    pub query_params: Vec<String>,
    pub content_params: Vec<String>,
    pub timeout_params: Vec<String>,
    pub alias_params: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolPathExtractorMetadata {
    pub path_params: Vec<String>,
    pub primary_path_param: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolValidationMetadata {
    pub strict_schema: bool,
    pub required: Vec<String>,
    pub alias_params: Vec<String>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultKind {
    Text,
    Json,
    FileContent,
    FileDiff,
    SearchResults,
    UrlContent,
    Image,
    Pdf,
    Canvas,
    Notification,
    TaskList,
    SessionList,
    Status,
    ExternalRun,
    UserQuestion,
    Unknown,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolRenderMetadata {
    pub result_kind: ToolResultKind,
    pub primary_resource: Option<String>,
    pub searchable_fields: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolMetadata {
    pub aliases: Vec<String>,
    pub search_hints: Vec<String>,
    pub effects: Vec<ToolEffect>,
    pub risk: ToolRisk,
    pub read_only: bool,
    pub destructive: bool,
    pub open_world: bool,
    pub strict: bool,
    pub interrupt_behavior: ToolInterruptBehavior,
    pub permission: ToolPermissionMetadata,
    pub permission_matcher: ToolPermissionMetadata,
    pub input: ToolInputMetadata,
    pub path_extractor: ToolPathExtractorMetadata,
    pub validation: ToolValidationMetadata,
    pub render: ToolRenderMetadata,
    pub search_text: String,
    pub auto_classifier_input: Vec<String>,
    pub classifier_tags: Vec<String>,
}

impl ToolMetadata {
    pub fn for_definition(def: &ToolDefinition) -> Self {
        let mut aliases = generic_aliases(&def.name);
        let mut search_hints = Vec::new();
        let mut effects = Vec::new();
        let mut classifier_tags = Vec::new();
        let mut render = ToolRenderMetadata {
            result_kind: ToolResultKind::Json,
            primary_resource: None,
            searchable_fields: vec![
                "name".to_string(),
                "description".to_string(),
                "parameters".to_string(),
            ],
        };

        add_tier_tags(def, &mut classifier_tags);
        let input = input_metadata(&def.parameters);

        let name = def.name.as_str();
        let lower = name.to_ascii_lowercase();

        match name {
            crate::tools::TOOL_READ => {
                push_all(
                    &mut aliases,
                    &["cat", "view file", "open file", "inspect file"],
                );
                push_all(
                    &mut search_hints,
                    &["read a file", "inspect file content", "view image file"],
                );
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::FileContent;
                render.primary_resource = Some("path".to_string());
            }
            crate::tools::TOOL_WRITE => {
                push_all(
                    &mut aliases,
                    &["create file", "overwrite file", "save file", "write file"],
                );
                push_all(
                    &mut search_hints,
                    &["write complete file content", "create a new file"],
                );
                push_unique(&mut effects, ToolEffect::WriteFileSystem);
                render.result_kind = ToolResultKind::FileDiff;
                render.primary_resource = Some("path".to_string());
            }
            crate::tools::TOOL_EDIT => {
                push_all(
                    &mut aliases,
                    &["modify file", "replace text", "old string", "targeted edit"],
                );
                push_all(
                    &mut search_hints,
                    &["edit a file in place", "replace one exact text span"],
                );
                push_unique(&mut effects, ToolEffect::WriteFileSystem);
                render.result_kind = ToolResultKind::FileDiff;
                render.primary_resource = Some("path".to_string());
            }
            crate::tools::TOOL_APPLY_PATCH => {
                push_all(
                    &mut aliases,
                    &["patch", "diff", "apply diff", "delete file", "move file"],
                );
                push_all(
                    &mut search_hints,
                    &[
                        "apply a multi-file patch",
                        "create update move or delete files",
                    ],
                );
                push_unique(&mut effects, ToolEffect::WriteFileSystem);
                render.result_kind = ToolResultKind::FileDiff;
                render.primary_resource = Some("input".to_string());
            }
            crate::tools::TOOL_EXEC => {
                push_all(
                    &mut aliases,
                    &["bash", "shell", "terminal", "command", "run command"],
                );
                push_all(
                    &mut search_hints,
                    &["execute a shell command", "run build test or script"],
                );
                push_unique(&mut effects, ToolEffect::ExecuteProcess);
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::Text;
                render.primary_resource = Some("command".to_string());
            }
            crate::tools::TOOL_PROCESS => {
                push_all(
                    &mut aliases,
                    &["poll process", "kill process", "exec session"],
                );
                push_all(
                    &mut search_hints,
                    &[
                        "manage a running exec process session",
                        "poll logs or kill process",
                    ],
                );
                push_unique(&mut effects, ToolEffect::RuntimeControl);
                push_unique(&mut effects, ToolEffect::ExecuteProcess);
                render.result_kind = ToolResultKind::Status;
                render.primary_resource = Some("session_id".to_string());
            }
            crate::tools::TOOL_LS => {
                push_all(&mut aliases, &["list directory", "dir", "files"]);
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::SearchResults;
                render.primary_resource = Some("path".to_string());
            }
            crate::tools::TOOL_GREP => {
                push_all(&mut aliases, &["rg", "search text", "search contents"]);
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::SearchResults;
                render.primary_resource = Some("pattern".to_string());
            }
            crate::tools::TOOL_FIND => {
                push_all(&mut aliases, &["glob", "file search", "find files"]);
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::SearchResults;
                render.primary_resource = Some("pattern".to_string());
            }
            crate::tools::TOOL_LSP => {
                push_all(
                    &mut aliases,
                    &[
                        "language server",
                        "semantic code search",
                        "definition references diagnostics hover symbols",
                    ],
                );
                push_unique(&mut effects, ToolEffect::ReadFileSystem);
                render.result_kind = ToolResultKind::Json;
                render.primary_resource = Some("path".to_string());
            }
            crate::tools::TOOL_WEB_FETCH => {
                push_all(&mut aliases, &["fetch url", "read webpage", "web page"]);
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::UrlContent;
                render.primary_resource = Some("url".to_string());
            }
            crate::tools::TOOL_WEB_SEARCH => {
                push_all(
                    &mut aliases,
                    &["search web", "internet search", "current info"],
                );
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::SearchResults;
                render.primary_resource = Some("query".to_string());
            }
            crate::tools::TOOL_BROWSER => {
                push_all(&mut aliases, &["chrome", "browser automation", "web ui"]);
                push_unique(&mut effects, ToolEffect::BrowserAutomation);
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::Json;
                render.primary_resource = Some("action".to_string());
            }
            crate::tools::TOOL_MAC_CONTROL => {
                push_all(
                    &mut aliases,
                    &["macos control", "desktop ui", "accessibility"],
                );
                push_unique(&mut effects, ToolEffect::DesktopAutomation);
                render.result_kind = ToolResultKind::Json;
                render.primary_resource = Some("action".to_string());
            }
            crate::tools::TOOL_SUBAGENT
            | crate::tools::TOOL_TEAM
            | crate::tools::TOOL_ACP_SPAWN => {
                push_all(&mut aliases, &["delegate", "worker", "agent"]);
                push_unique(&mut effects, ToolEffect::AgentDelegation);
                push_unique(&mut effects, ToolEffect::RuntimeControl);
                render.result_kind = ToolResultKind::ExternalRun;
                render.primary_resource = Some("task".to_string());
            }
            crate::tools::TOOL_IMAGE | crate::tools::TOOL_PDF => {
                push_unique(&mut effects, ToolEffect::MediaRead);
                render.result_kind = if name == crate::tools::TOOL_IMAGE {
                    ToolResultKind::Image
                } else {
                    ToolResultKind::Pdf
                };
            }
            crate::tools::TOOL_IMAGE_GENERATE => {
                push_all(&mut aliases, &["generate image", "create image"]);
                push_unique(&mut effects, ToolEffect::MediaWrite);
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::Image;
                render.primary_resource = Some("prompt".to_string());
            }
            crate::tools::TOOL_CANVAS => {
                push_all(&mut aliases, &["preview", "artifact", "html app", "visual"]);
                push_unique(&mut effects, ToolEffect::MediaWrite);
                render.result_kind = ToolResultKind::Canvas;
                render.primary_resource = Some("project_id".to_string());
            }
            crate::tools::TOOL_ARTIFACT => {
                push_all(
                    &mut aliases,
                    &["artifact", "report", "dashboard", "deliverable"],
                );
                push_unique(&mut effects, ToolEffect::MediaWrite);
                render.result_kind = ToolResultKind::Canvas;
                render.primary_resource = Some("artifact_id".to_string());
            }
            crate::tools::TOOL_ASK_USER_QUESTION => {
                push_all(&mut aliases, &["ask user", "clarify", "question"]);
                push_unique(&mut effects, ToolEffect::UserInteraction);
                render.result_kind = ToolResultKind::UserQuestion;
            }
            crate::tools::TOOL_SEND_NOTIFICATION | crate::tools::TOOL_SEND_ATTACHMENT => {
                push_unique(&mut effects, ToolEffect::UserInteraction);
                render.result_kind = ToolResultKind::Notification;
            }
            crate::tools::TOOL_TASK_CREATE | crate::tools::TOOL_TASK_UPDATE => {
                push_all(&mut aliases, &["todo", "task list", "progress"]);
                push_unique(&mut effects, ToolEffect::TaskWrite);
                render.result_kind = ToolResultKind::TaskList;
            }
            crate::tools::TOOL_TASK_LIST => {
                push_all(&mut aliases, &["todo", "task list", "progress"]);
                push_unique(&mut effects, ToolEffect::TaskRead);
                render.result_kind = ToolResultKind::TaskList;
            }
            crate::tools::TOOL_GOAL_STATUS => {
                push_all(&mut aliases, &["goal", "objective", "completion", "audit"]);
                push_unique(&mut effects, ToolEffect::GoalRead);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_GOAL_CHECKPOINT
            | crate::tools::TOOL_GOAL_RECORD_EVIDENCE
            | crate::tools::TOOL_GOAL_EVALUATE
            | crate::tools::TOOL_GOAL_FINISH_REQUEST
            | crate::tools::TOOL_GOAL_BLOCK_REQUEST => {
                push_all(
                    &mut aliases,
                    &["goal", "objective", "checkpoint", "completion", "evidence"],
                );
                push_unique(&mut effects, ToolEffect::GoalWrite);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_MANAGE_CRON | crate::tools::TOOL_SCHEDULE_WAKEUP => {
                push_all(&mut aliases, &["schedule", "reminder", "wakeup"]);
                push_unique(&mut effects, ToolEffect::Scheduling);
                push_unique(&mut effects, ToolEffect::RuntimeControl);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_JOB_STATUS | crate::tools::TOOL_RUNTIME_CANCEL => {
                push_all(&mut aliases, &["job", "background task", "runtime"]);
                push_unique(&mut effects, ToolEffect::RuntimeControl);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_TOOL_SEARCH | crate::tools::TOOL_SKILL => {
                push_all(&mut aliases, &["discover tools", "load tool", "capability"]);
                push_unique(&mut effects, ToolEffect::RuntimeControl);
                render.result_kind = ToolResultKind::Json;
            }
            crate::tools::TOOL_ENTER_PLAN_MODE | crate::tools::TOOL_SUBMIT_PLAN => {
                push_all(&mut aliases, &["plan", "planning", "design contract"]);
                push_unique(&mut effects, ToolEffect::UserInteraction);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_GET_SETTINGS
            | crate::tools::TOOL_LIST_SETTINGS_BACKUPS
            | crate::tools::TOOL_RESTORE_SETTINGS_BACKUP
            | crate::tools::TOOL_UPDATE_SETTINGS => {
                if name.contains("get") || name.contains("list") {
                    push_unique(&mut effects, ToolEffect::SettingsRead);
                } else {
                    push_unique(&mut effects, ToolEffect::SettingsWrite);
                }
                render.result_kind = ToolResultKind::Json;
            }
            crate::tools::TOOL_APP_UPDATE => {
                push_unique(&mut effects, ToolEffect::AppUpdate);
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::Status;
            }
            crate::tools::TOOL_GET_WEATHER => {
                push_unique(&mut effects, ToolEffect::NetworkAccess);
                render.result_kind = ToolResultKind::Json;
                render.primary_resource = Some("location".to_string());
            }
            crate::tools::TOOL_ISSUE_REPORT => {
                push_unique(&mut effects, ToolEffect::ExternalService);
                render.result_kind = ToolResultKind::Status;
            }
            _ => apply_prefix_rules(name, &mut effects, &mut aliases, &mut render),
        }

        if effects.is_empty() {
            apply_tier_defaults(def, &mut effects);
        }

        for effect in &effects {
            push_all(&mut search_hints, &[effect.as_search_tag()]);
            classifier_tags.extend(
                effect
                    .as_search_tag()
                    .split_whitespace()
                    .map(|s| s.to_string()),
            );
        }

        for token in lower.split('_').filter(|s| !s.is_empty()) {
            push_unique_string(&mut classifier_tags, token);
        }

        let destructive = is_destructive_tool(name, &effects);
        let strict = may_trigger_strict_approval(name, &effects);
        let open_world = is_open_world(&effects);
        let risk = risk_for(def, &effects, destructive, strict, open_world);
        let read_only = is_read_only(&effects, destructive);
        let interrupt_behavior = interrupt_behavior_for(def, &effects, name);
        let permission = permission_for(def, &effects, risk, strict);
        let path_extractor = ToolPathExtractorMetadata {
            primary_path_param: input.path_params.first().cloned(),
            path_params: input.path_params.clone(),
        };
        let validation = ToolValidationMetadata {
            strict_schema: input.strict_schema,
            required: input.required.clone(),
            alias_params: input.alias_params.clone(),
        };

        sort_dedup(&mut aliases);
        sort_dedup(&mut search_hints);
        sort_dedup(&mut classifier_tags);
        let search_text = build_search_text(
            &aliases,
            &search_hints,
            &classifier_tags,
            &input,
            &render,
            risk,
            interrupt_behavior,
        );
        let auto_classifier_input = classifier_tags.clone();

        Self {
            aliases,
            search_hints,
            effects,
            risk,
            read_only,
            destructive,
            open_world,
            strict,
            interrupt_behavior,
            permission: permission.clone(),
            permission_matcher: permission,
            input,
            path_extractor,
            validation,
            render,
            search_text,
            auto_classifier_input,
            classifier_tags,
        }
    }

    pub fn searchable_text(&self) -> String {
        self.search_text.clone()
    }
}

fn build_search_text(
    aliases: &[String],
    search_hints: &[String],
    classifier_tags: &[String],
    input: &ToolInputMetadata,
    render: &ToolRenderMetadata,
    risk: ToolRisk,
    interrupt_behavior: ToolInterruptBehavior,
) -> String {
    let mut parts = Vec::new();
    parts.extend(aliases.iter().cloned());
    parts.extend(search_hints.iter().cloned());
    parts.extend(classifier_tags.iter().cloned());
    parts.extend(input.required.iter().cloned());
    parts.extend(input.path_params.iter().cloned());
    parts.extend(input.command_params.iter().cloned());
    parts.extend(input.url_params.iter().cloned());
    parts.extend(input.query_params.iter().cloned());
    parts.extend(input.content_params.iter().cloned());
    parts.extend(render.searchable_fields.iter().cloned());
    parts.push(format!("{:?}", risk).to_ascii_lowercase());
    parts.push(format!("{:?}", interrupt_behavior).to_ascii_lowercase());
    parts.join(" ")
}

fn apply_prefix_rules(
    name: &str,
    effects: &mut Vec<ToolEffect>,
    aliases: &mut Vec<String>,
    render: &mut ToolRenderMetadata,
) {
    if name.starts_with("note_") || name == crate::tools::TOOL_SESSION_TO_NOTE {
        push_all(aliases, &["knowledge", "note", "second brain"]);
        if name.contains("read")
            || name.contains("search")
            || name.contains("backlink")
            || name.contains("tag")
            || name.contains("graph")
            || name.contains("similar")
            || name.contains("related")
            || name.contains("orphan")
            || name.contains("broken")
        {
            push_unique(effects, ToolEffect::KnowledgeRead);
        } else {
            push_unique(effects, ToolEffect::KnowledgeWrite);
        }
        render.result_kind = ToolResultKind::Json;
        return;
    }

    if name == crate::tools::TOOL_KNOWLEDGE_RECALL {
        push_all(aliases, &["knowledge recall", "memory and notes"]);
        push_unique(effects, ToolEffect::KnowledgeRead);
        push_unique(effects, ToolEffect::MemoryRead);
        render.result_kind = ToolResultKind::SearchResults;
        return;
    }

    if name.contains("memory") {
        if name.contains("recall") || name.contains("get") {
            push_unique(effects, ToolEffect::MemoryRead);
            render.result_kind = ToolResultKind::SearchResults;
        } else {
            push_unique(effects, ToolEffect::MemoryWrite);
            render.result_kind = ToolResultKind::Json;
        }
        return;
    }

    if name.starts_with("sessions_")
        || name == crate::tools::TOOL_SESSION_STATUS
        || name == crate::tools::TOOL_PEEK_SESSIONS
        || name == crate::tools::TOOL_AGENTS_LIST
    {
        if name.ends_with("_send") {
            push_unique(effects, ToolEffect::SessionWrite);
        } else {
            push_unique(effects, ToolEffect::SessionRead);
        }
        render.result_kind = ToolResultKind::SessionList;
        return;
    }

    if name.starts_with("feishu_") {
        push_all(aliases, &["feishu", "lark", "external service"]);
        push_unique(effects, ToolEffect::ExternalService);
        if name.contains("create")
            || name.contains("update")
            || name.contains("delete")
            || name.contains("cancel")
            || name.contains("upload")
            || name.contains("append")
            || name.contains("subscribe")
        {
            push_unique(effects, ToolEffect::ExternalServiceWrite);
        }
        render.result_kind = ToolResultKind::Json;
        return;
    }

    if name.starts_with("mcp__") {
        push_all(aliases, &["mcp", "external tool"]);
        push_unique(effects, ToolEffect::ExternalService);
        render.result_kind = ToolResultKind::Json;
    }
}

fn apply_tier_defaults(def: &ToolDefinition, effects: &mut Vec<ToolEffect>) {
    match &def.tier {
        ToolTier::Core { subclass } => match subclass {
            CoreSubclass::FileSystem => push_unique(effects, ToolEffect::ReadFileSystem),
            CoreSubclass::Interaction => push_unique(effects, ToolEffect::UserInteraction),
            CoreSubclass::SessionAware => push_unique(effects, ToolEffect::SessionRead),
            CoreSubclass::Meta => push_unique(effects, ToolEffect::RuntimeControl),
            CoreSubclass::PlanMode => push_unique(effects, ToolEffect::UserInteraction),
        },
        ToolTier::Standard { .. } | ToolTier::Configured { .. } => {
            push_unique(effects, ToolEffect::ExternalService)
        }
        ToolTier::Memory => push_unique(effects, ToolEffect::MemoryRead),
        ToolTier::Mcp => push_unique(effects, ToolEffect::ExternalService),
    }
}

fn input_metadata(parameters: &Value) -> ToolInputMetadata {
    let mut required = parameters
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    sort_dedup(&mut required);

    let strict_schema = parameters
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .map(|v| !v)
        .unwrap_or(false);

    let mut meta = ToolInputMetadata {
        required,
        strict_schema,
        action_param: None,
        path_params: Vec::new(),
        command_params: Vec::new(),
        url_params: Vec::new(),
        id_params: Vec::new(),
        query_params: Vec::new(),
        content_params: Vec::new(),
        timeout_params: Vec::new(),
        alias_params: Vec::new(),
    };

    let Some(props) = parameters.get("properties").and_then(|v| v.as_object()) else {
        return meta;
    };

    for (name, schema) in props {
        let desc = schema
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let lower = name.to_ascii_lowercase();

        if lower == "action" {
            meta.action_param = Some(name.clone());
        }
        if lower.contains("path")
            || lower == "cwd"
            || lower == "file"
            || lower == "files"
            || lower == "pdfs"
            || desc.contains("file path")
        {
            push_unique_string(&mut meta.path_params, name);
        }
        if lower.contains("command") || lower == "cmd" {
            push_unique_string(&mut meta.command_params, name);
        }
        if lower == "url" || lower.ends_with("_url") || desc.contains("url") {
            push_unique_string(&mut meta.url_params, name);
        }
        if lower == "id" || lower.ends_with("_id") || lower.contains("session_id") {
            push_unique_string(&mut meta.id_params, name);
        }
        if lower.contains("query") || lower == "pattern" || lower == "prompt" {
            push_unique_string(&mut meta.query_params, name);
        }
        if lower.contains("content")
            || lower.contains("text")
            || lower == "input"
            || lower == "patch"
            || lower == "html"
            || lower == "css"
            || lower == "js"
        {
            push_unique_string(&mut meta.content_params, name);
        }
        if lower.contains("timeout") || lower.contains("delay") || lower.ends_with("_secs") {
            push_unique_string(&mut meta.timeout_params, name);
        }
        if desc.contains("also accepts") || desc.contains("alias") {
            push_unique_string(&mut meta.alias_params, name);
        }
    }

    meta
}

fn generic_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    if name.contains('_') {
        aliases.push(name.replace('_', " "));
        aliases.push(name.replace('_', "-"));
    }
    aliases
}

fn add_tier_tags(def: &ToolDefinition, tags: &mut Vec<String>) {
    match &def.tier {
        ToolTier::Core { subclass } => {
            push_unique_string(tags, "core");
            push_unique_string(tags, subclass.as_str());
        }
        ToolTier::Standard { .. } => push_unique_string(tags, "standard"),
        ToolTier::Configured { .. } => push_unique_string(tags, "configured"),
        ToolTier::Memory => push_unique_string(tags, "memory"),
        ToolTier::Mcp => push_unique_string(tags, "mcp"),
    }
    if def.internal {
        push_unique_string(tags, "internal");
    }
    if def.concurrent_safe {
        push_unique_string(tags, "concurrent_safe");
        push_unique_string(tags, "parallel");
    }
    if def.async_capable {
        push_unique_string(tags, "async_capable");
        push_unique_string(tags, "background");
    }
    if def.supports_deferred() {
        push_unique_string(tags, "deferred");
        push_unique_string(tags, "discoverable");
    }
}

fn is_destructive_tool(name: &str, effects: &[ToolEffect]) -> bool {
    if effects.contains(&ToolEffect::ExecuteProcess)
        || name == crate::tools::TOOL_WRITE
        || name == crate::tools::TOOL_EDIT
        || name == crate::tools::TOOL_APPLY_PATCH
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.contains("delete")
        || lower.contains("remove")
        || lower.contains("kill")
        || lower.contains("cancel")
        || lower.contains("restore")
        || lower.contains("update")
        || lower.contains("patch")
        || lower.contains("move")
        || lower.contains("rename")
}

fn may_trigger_strict_approval(name: &str, effects: &[ToolEffect]) -> bool {
    effects.contains(&ToolEffect::ExecuteProcess)
        || effects.contains(&ToolEffect::BrowserAutomation)
        || effects.contains(&ToolEffect::DesktopAutomation)
        || name == crate::tools::TOOL_WRITE
        || name == crate::tools::TOOL_EDIT
        || name == crate::tools::TOOL_APPLY_PATCH
        || name == crate::tools::TOOL_UPDATE_SETTINGS
        || name == crate::tools::TOOL_RESTORE_SETTINGS_BACKUP
        || name == crate::tools::TOOL_APP_UPDATE
}

fn is_open_world(effects: &[ToolEffect]) -> bool {
    effects.iter().any(|e| {
        matches!(
            e,
            ToolEffect::ExecuteProcess
                | ToolEffect::NetworkAccess
                | ToolEffect::BrowserAutomation
                | ToolEffect::DesktopAutomation
                | ToolEffect::ExternalService
                | ToolEffect::ExternalServiceWrite
                | ToolEffect::AgentDelegation
        )
    })
}

fn is_read_only(effects: &[ToolEffect], destructive: bool) -> bool {
    if destructive {
        return false;
    }
    !effects.iter().any(|e| {
        matches!(
            e,
            ToolEffect::WriteFileSystem
                | ToolEffect::MemoryWrite
                | ToolEffect::KnowledgeWrite
                | ToolEffect::SessionWrite
                | ToolEffect::SettingsWrite
                | ToolEffect::TaskWrite
                | ToolEffect::GoalWrite
                | ToolEffect::UserInteraction
                | ToolEffect::ExternalServiceWrite
                | ToolEffect::AgentDelegation
                | ToolEffect::Scheduling
                | ToolEffect::MediaWrite
                | ToolEffect::AppUpdate
        )
    })
}

fn risk_for(
    def: &ToolDefinition,
    effects: &[ToolEffect],
    destructive: bool,
    strict: bool,
    open_world: bool,
) -> ToolRisk {
    if strict {
        return ToolRisk::Strict;
    }
    if effects.iter().any(|e| {
        matches!(
            e,
            ToolEffect::DesktopAutomation
                | ToolEffect::BrowserAutomation
                | ToolEffect::AppUpdate
                | ToolEffect::SettingsWrite
        )
    }) {
        return ToolRisk::High;
    }
    if destructive
        || effects.iter().any(|e| {
            matches!(
                e,
                ToolEffect::WriteFileSystem
                    | ToolEffect::ExternalService
                    | ToolEffect::ExternalServiceWrite
                    | ToolEffect::AgentDelegation
                    | ToolEffect::Scheduling
                    | ToolEffect::MediaWrite
                    | ToolEffect::MemoryWrite
                    | ToolEffect::KnowledgeWrite
                    | ToolEffect::SessionWrite
            )
        })
    {
        return ToolRisk::Medium;
    }
    if open_world && !def.internal {
        return ToolRisk::Medium;
    }
    ToolRisk::Low
}

fn interrupt_behavior_for(
    def: &ToolDefinition,
    effects: &[ToolEffect],
    name: &str,
) -> ToolInterruptBehavior {
    if name == crate::tools::TOOL_ASK_USER_QUESTION {
        return ToolInterruptBehavior::HumanBlocked;
    }
    if def.async_capable
        || effects.contains(&ToolEffect::AgentDelegation)
        || effects.contains(&ToolEffect::Scheduling)
        || name == crate::tools::TOOL_EXEC
        || name == crate::tools::TOOL_PROCESS
    {
        return ToolInterruptBehavior::LongRunning;
    }
    if effects.contains(&ToolEffect::RuntimeControl) {
        return ToolInterruptBehavior::Graceful;
    }
    ToolInterruptBehavior::Immediate
}

fn permission_for(
    def: &ToolDefinition,
    effects: &[ToolEffect],
    risk: ToolRisk,
    strict: bool,
) -> ToolPermissionMetadata {
    let subject = if def.internal {
        ToolPermissionSubject::Internal
    } else if effects.contains(&ToolEffect::ExecuteProcess) {
        ToolPermissionSubject::Process
    } else if effects.contains(&ToolEffect::WriteFileSystem)
        || effects.contains(&ToolEffect::ReadFileSystem)
    {
        ToolPermissionSubject::LocalFileSystem
    } else if effects.contains(&ToolEffect::BrowserAutomation) {
        ToolPermissionSubject::Browser
    } else if effects.contains(&ToolEffect::DesktopAutomation) {
        ToolPermissionSubject::Desktop
    } else if effects.contains(&ToolEffect::NetworkAccess) {
        ToolPermissionSubject::Network
    } else if effects.contains(&ToolEffect::ExternalService)
        || effects.contains(&ToolEffect::ExternalServiceWrite)
    {
        ToolPermissionSubject::ExternalService
    } else if effects.contains(&ToolEffect::MemoryRead)
        || effects.contains(&ToolEffect::MemoryWrite)
    {
        ToolPermissionSubject::Memory
    } else if effects.contains(&ToolEffect::KnowledgeRead)
        || effects.contains(&ToolEffect::KnowledgeWrite)
    {
        ToolPermissionSubject::Knowledge
    } else if effects.contains(&ToolEffect::SessionRead)
        || effects.contains(&ToolEffect::SessionWrite)
    {
        ToolPermissionSubject::Session
    } else if effects.contains(&ToolEffect::SettingsRead)
        || effects.contains(&ToolEffect::SettingsWrite)
    {
        ToolPermissionSubject::Settings
    } else if effects.contains(&ToolEffect::UserInteraction) {
        ToolPermissionSubject::UserInteraction
    } else if effects.contains(&ToolEffect::RuntimeControl) {
        ToolPermissionSubject::Runtime
    } else if effects.contains(&ToolEffect::AgentDelegation) {
        ToolPermissionSubject::AgentDelegation
    } else if effects.contains(&ToolEffect::Scheduling) {
        ToolPermissionSubject::Scheduling
    } else if effects.contains(&ToolEffect::MediaRead) || effects.contains(&ToolEffect::MediaWrite)
    {
        ToolPermissionSubject::Media
    } else if effects.contains(&ToolEffect::AppUpdate) {
        ToolPermissionSubject::Application
    } else {
        ToolPermissionSubject::Internal
    };

    let approval_hint = if def.internal {
        ToolApprovalHint::NeverInternal
    } else if strict || risk == ToolRisk::Strict {
        ToolApprovalHint::StrictPossible
    } else if risk >= ToolRisk::Medium {
        ToolApprovalHint::PolicyDependent
    } else {
        ToolApprovalHint::UsuallyAllow
    };

    ToolPermissionMetadata {
        subject,
        approval_hint,
    }
}

fn push_all(vec: &mut Vec<String>, values: &[&str]) {
    for value in values {
        push_unique_string(vec, value);
    }
}

fn push_unique<T: PartialEq>(vec: &mut Vec<T>, value: T) {
    if !vec.contains(&value) {
        vec.push(value);
    }
}

fn push_unique_string(vec: &mut Vec<String>, value: &str) {
    if value.is_empty() {
        return;
    }
    let value = value.to_string();
    if !vec.contains(&value) {
        vec.push(value);
    }
}

fn sort_dedup(vec: &mut Vec<String>) {
    vec.sort();
    vec.dedup();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tools::definitions::{CoreSubclass, ToolTier};

    fn def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "test tool".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path"},
                    "command": {"type": "string"},
                    "url": {"type": "string"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            tier: ToolTier::Core {
                subclass: CoreSubclass::FileSystem,
            },
            internal: false,
            concurrent_safe: false,
            async_capable: false,
        }
    }

    #[test]
    fn read_is_read_only_filesystem() {
        let md = ToolMetadata::for_definition(&def(crate::tools::TOOL_READ));
        assert!(md.read_only);
        assert!(md.effects.contains(&ToolEffect::ReadFileSystem));
        assert_eq!(md.risk, ToolRisk::Low);
        assert!(md.input.path_params.contains(&"path".to_string()));
    }

    #[test]
    fn exec_is_strict_open_world_process() {
        let md = ToolMetadata::for_definition(&def(crate::tools::TOOL_EXEC));
        assert!(!md.read_only);
        assert!(md.open_world);
        assert!(md.strict);
        assert_eq!(md.risk, ToolRisk::Strict);
        assert!(md.effects.contains(&ToolEffect::ExecuteProcess));
        assert!(md.input.command_params.contains(&"command".to_string()));
    }

    #[test]
    fn note_update_is_knowledge_write() {
        let md = ToolMetadata::for_definition(&def(crate::tools::TOOL_NOTE_UPDATE));
        assert!(!md.read_only);
        assert!(md.effects.contains(&ToolEffect::KnowledgeWrite));
    }

    #[test]
    fn runtime_status_tools_remain_read_only() {
        let tool_search = ToolMetadata::for_definition(&def(crate::tools::TOOL_TOOL_SEARCH));
        assert!(tool_search.read_only);
        assert!(tool_search.effects.contains(&ToolEffect::RuntimeControl));

        let job_status = ToolMetadata::for_definition(&def(crate::tools::TOOL_JOB_STATUS));
        assert!(job_status.read_only);
        assert!(job_status.effects.contains(&ToolEffect::RuntimeControl));

        let runtime_cancel = ToolMetadata::for_definition(&def(crate::tools::TOOL_RUNTIME_CANCEL));
        assert!(!runtime_cancel.read_only);
        assert!(runtime_cancel.destructive);
    }

    #[test]
    fn feishu_write_is_external_service_write_not_settings_write() {
        let md = ToolMetadata::for_definition(&def("feishu_docx_update_block_text"));
        assert!(!md.read_only);
        assert_eq!(md.risk, ToolRisk::Medium);
        assert!(md.effects.contains(&ToolEffect::ExternalService));
        assert!(md.effects.contains(&ToolEffect::ExternalServiceWrite));
        assert!(!md.effects.contains(&ToolEffect::SettingsWrite));
        assert_eq!(
            md.permission.subject,
            ToolPermissionSubject::ExternalService
        );
    }

    #[test]
    fn task_tools_distinguish_read_and_write() {
        let list = ToolMetadata::for_definition(&def(crate::tools::TOOL_TASK_LIST));
        assert!(list.read_only);
        assert!(list.effects.contains(&ToolEffect::TaskRead));
        assert!(!list.effects.contains(&ToolEffect::TaskWrite));

        let update = ToolMetadata::for_definition(&def(crate::tools::TOOL_TASK_UPDATE));
        assert!(!update.read_only);
        assert!(update.effects.contains(&ToolEffect::TaskWrite));
    }

    #[test]
    fn all_dispatchable_tools_have_v2_metadata() {
        for tool in crate::tools::dispatch::all_dispatchable_tools() {
            let md = tool.v2_metadata();
            assert!(
                !md.effects.is_empty(),
                "{} should declare at least one v2 effect",
                tool.name
            );
            assert!(
                !md.classifier_tags.is_empty(),
                "{} should expose classifier tags",
                tool.name
            );
            assert!(
                !md.render.searchable_fields.is_empty(),
                "{} should expose render/search fields",
                tool.name
            );
            assert!(
                !md.search_text.is_empty(),
                "{} should expose search text",
                tool.name
            );
            assert!(
                !md.auto_classifier_input.is_empty(),
                "{} should expose auto-classifier input",
                tool.name
            );
        }
    }
}
