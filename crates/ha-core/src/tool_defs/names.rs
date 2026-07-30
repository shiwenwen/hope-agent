//! 全部内置工具的名字常量——纯 `&'static str` 字面量、零依赖。
//!
//! kernel 各处（permission / system_prompt / context_compact / agent /
//! async_jobs …）引用工具名一律走这里，不得依赖 `tools/`（分发注册表 +
//! adapter 目录，未来随特征上浮）。`crate::tools` 门面原样再导出，crate
//! 外路径（`ha_core::tools::TOOL_*`）不变。

/// 工具结果里内联 base64 图片的标记前缀（browser 截图 / canvas 快照 /
/// mac_control 帧共用的结果格式契约——kernel 的图片抽取与压缩管线按它
/// 识别，特征 crate 只消费不重定义）。
pub const IMAGE_BASE64_PREFIX: &str = "__IMAGE_BASE64__";

pub const TOOL_EXEC: &str = "exec";
pub const TOOL_PROCESS: &str = "process";
pub const TOOL_READ: &str = "read";
pub const TOOL_WRITE: &str = "write";
pub const TOOL_EDIT: &str = "edit";
pub const TOOL_LS: &str = "ls";
pub const TOOL_LSP: &str = "lsp";
pub const TOOL_GREP: &str = "grep";
pub const TOOL_FIND: &str = "find";
pub const TOOL_APPLY_PATCH: &str = "apply_patch";
pub const TOOL_WEB_SEARCH: &str = "web_search";
pub const TOOL_WEB_FETCH: &str = "web_fetch";
pub const TOOL_SAVE_MEMORY: &str = "save_memory";
pub const TOOL_RECALL_MEMORY: &str = "recall_memory";
pub const TOOL_UPDATE_MEMORY: &str = "update_memory";
pub const TOOL_DELETE_MEMORY: &str = "delete_memory";
pub const TOOL_UPDATE_CORE_MEMORY: &str = "update_core_memory";
pub const TOOL_CORE_MEMORY: &str = "core_memory";
pub const TOOL_PROJECT_MEMORY: &str = "project_memory";
pub const TOOL_MANAGE_CRON: &str = "manage_cron";
pub const TOOL_BROWSER: &str = "browser";
pub const TOOL_MAC_CONTROL: &str = "mac_control";
pub const TOOL_SEND_NOTIFICATION: &str = "send_notification";
pub const TOOL_SUBAGENT: &str = "subagent";
pub const TOOL_MEMORY_GET: &str = "memory_get";
pub const TOOL_AGENTS_LIST: &str = "agents_list";

// Knowledge base (note_*) tools.
pub const TOOL_NOTE_CREATE: &str = "note_create";
pub const TOOL_NOTE_READ: &str = "note_read";
pub const TOOL_NOTE_UPDATE: &str = "note_update";
pub const TOOL_NOTE_PATCH: &str = "note_patch";
pub const TOOL_NOTE_APPEND: &str = "note_append";
pub const TOOL_NOTE_DELETE: &str = "note_delete";
pub const TOOL_NOTE_SEARCH: &str = "note_search";
pub const TOOL_NOTE_LINK: &str = "note_link";
pub const TOOL_NOTE_BACKLINKS: &str = "note_backlinks";
pub const TOOL_NOTE_BY_TAG: &str = "note_by_tag";
pub const TOOL_NOTE_TAGS: &str = "note_tags";
pub const TOOL_NOTE_RENAME: &str = "note_rename";
pub const TOOL_NOTE_MOVE: &str = "note_move";
pub const TOOL_NOTE_SET_FRONTMATTER: &str = "note_set_frontmatter";
pub const TOOL_NOTE_ASSIGN_BLOCK: &str = "note_assign_block";
pub const TOOL_NOTE_BROKEN_LINKS: &str = "note_broken_links";
pub const TOOL_NOTE_ORPHANS: &str = "note_orphans";
pub const TOOL_NOTE_GRAPH: &str = "note_graph";
pub const TOOL_NOTE_SIMILAR: &str = "note_similar";
pub const TOOL_NOTE_RELATED: &str = "note_related";
pub const TOOL_NOTE_SUGGEST_LINKS: &str = "note_suggest_links";
pub const TOOL_NOTE_DISTILL: &str = "note_distill";
pub const TOOL_NOTE_MOC: &str = "note_moc";
pub const TOOL_KNOWLEDGE_RECALL: &str = "knowledge_recall";
pub const TOOL_SESSION_TO_NOTE: &str = "session_to_note";
pub const TOOL_SESSIONS_LIST: &str = "sessions_list";
pub const TOOL_SESSION_STATUS: &str = "session_status";
pub const TOOL_SESSIONS_SEARCH: &str = "sessions_search";
pub const TOOL_SESSIONS_HISTORY: &str = "sessions_history";
pub const TOOL_SESSIONS_SEND: &str = "sessions_send";
pub const TOOL_IMAGE: &str = "image";
pub const TOOL_IMAGE_GENERATE: &str = "image_generate";
pub const TOOL_AUDIO_GENERATE: &str = "audio_generate";
pub const TOOL_ISSUE_REPORT: &str = "issue_report";
pub const TOOL_PDF: &str = "pdf";
pub const TOOL_CANVAS: &str = "canvas";
pub const TOOL_ARTIFACT: &str = "artifact";
pub const TOOL_DESIGN: &str = "design";
pub const TOOL_ACP_SPAWN: &str = "acp_spawn";
pub const TOOL_GET_WEATHER: &str = "get_weather";
pub const TOOL_ASK_USER_QUESTION: &str = "ask_user_question";
pub const TOOL_SUBMIT_PLAN: &str = "submit_plan";
pub const TOOL_ENTER_PLAN_MODE: &str = "enter_plan_mode";
pub const TOOL_TOOL_SEARCH: &str = "tool_search";
pub const TOOL_WORKFLOW: &str = "workflow";
pub const TOOL_TASK_CREATE: &str = "task_create";
pub const TOOL_TASK_UPDATE: &str = "task_update";
pub const TOOL_TASK_LIST: &str = "task_list";
pub const TOOL_GOAL_STATUS: &str = "goal_status";
pub const TOOL_GOAL_PREPARE_CONTRACT: &str = "goal_prepare_contract";
pub const TOOL_GOAL_CHECKPOINT: &str = "goal_checkpoint";
pub const TOOL_GOAL_RECORD_EVIDENCE: &str = "goal_record_evidence";
pub const TOOL_GOAL_EVALUATE: &str = "goal_evaluate";
pub const TOOL_GOAL_FINISH_REQUEST: &str = "goal_finish_request";
pub const TOOL_GOAL_BLOCK_REQUEST: &str = "goal_block_request";
pub const TOOL_LOOP_STATUS: &str = "loop_status";
pub const TOOL_LOOP_RESCHEDULE: &str = "loop_reschedule";
pub const TOOL_LOOP_STOP: &str = "loop_stop";
pub const TOOL_LOOP_RECORD_PROGRESS: &str = "loop_record_progress";
pub const TOOL_LOOP_WATCH: &str = "loop_watch";
pub const TOOL_LOOP_UNWATCH: &str = "loop_unwatch";
pub const TOOL_APP_UPDATE: &str = "app_update";
pub const TOOL_JOB_STATUS: &str = "job_status";
pub const TOOL_SCHEDULE_WAKEUP: &str = "schedule_wakeup";
pub const TOOL_RUNTIME_CANCEL: &str = "runtime_cancel";
pub const TOOL_TEAM: &str = "team";
pub const TOOL_PEEK_SESSIONS: &str = "peek_sessions";
pub const TOOL_GET_SETTINGS: &str = "get_settings";
pub const TOOL_UPDATE_SETTINGS: &str = "update_settings";
pub const TOOL_LIST_SETTINGS_BACKUPS: &str = "list_settings_backups";
pub const TOOL_RESTORE_SETTINGS_BACKUP: &str = "restore_settings_backup";
pub const TOOL_SEND_ATTACHMENT: &str = "send_attachment";
pub const TOOL_SKILL: &str = "skill";
pub const TOOL_MCP_RESOURCE: &str = "mcp_resource";
pub const TOOL_MCP_PROMPT: &str = "mcp_prompt";

/// Optional per-call async-job timeout injected into async-capable tool schemas.
///
/// This is intentionally separate from tool-specific timeouts such as
/// `exec.timeout`: it caps the outer async job, sets a per-call cap when the
/// user's `asyncTools.maxJobSecs` is unlimited, and can only tighten a positive
/// user-configured boundary.
pub const ASYNC_JOB_TIMEOUT_ARG: &str = "job_timeout_secs";
