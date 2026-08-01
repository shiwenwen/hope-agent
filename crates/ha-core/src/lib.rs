// Hope Agent Core — zero Tauri dependency
// All business logic lives here.
#![recursion_limit = "512"]

// ── 基础层再导出（ha-base）────────────────────────────────────────
// glob 再导出让 ha-base 的模块与 util 助手同时出现在 ha-core 根命名空间：
//   · ha-core 内部 50 万行的 `crate::paths::…` / `crate::truncate_utf8` 照旧解析
//   · 下游 `ha_core::platform::…` / `ha_core::security::…` 零改动
// 搬迁对调用方完全透明，这是分期可回滚的前提。
pub use ha_base::*;

// `app_info!` 系列宏原先靠 `#[macro_use] pub mod logging;` 全 crate 可见。
// 搬进 ha-base 后必须用 `#[macro_use] extern crate`：`use ha_base::app_info`
// 只在**声明它的那个模块**内生效，不会让 500 处调用点免限定可用。
#[macro_use]
extern crate ha_base;

// 再导出，保证下游 `ha_core::app_warn` 等既有路径不变。
pub use ha_base::{app_debug, app_error, app_info, app_warn};

// ── Macros must come first ────────────────────────────────────────

// ── New abstractions ──────────────────────────────────────────────
pub mod eval_context;

// ── Initialization ────────────────────────────────────────────────
pub mod app_init;
pub mod async_jobs;
pub mod attachments;
pub mod globals;

// test-support feature：跨 crate 测试设施（ha-media 等特征 crate 的
// dev-dependencies 开启；生产构建不编译，生产代码禁调）。
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

// ── Core modules (migrated from src-tauri) ────────────────────────
pub mod agent;
pub mod agent_config;
pub mod agent_lifecycle;
pub mod agent_loader;
pub mod ask_user;
pub mod automation;
// `activity`（autonomy 活动快照）**刻意留 kernel**：它是 `impl SessionDB` 的
// 一个扩展方法，唯一 kernel 消费者是 Core 工具 `tools::goal`（无条件注册）——
// Core 工具在每种运行形态下都必须可用，把数据源放到特征钩子后面会让
// minimal / ACP **静默**缺数据（调用点是 `.ok().and_then(..).unwrap_or(Null)`，
// 只会让 activity 字段变 null，不报错）。
//
// **代价如实登记，别当零成本**：它 `use crate::loop_control::{LoopSchedule,
// LoopState}`，而 loop_control 属 ha-cron 组——留 kernel 把原本合法的
// `ha-dash → ha-cron` 兄弟边变成了 **kernel→特征** 边。这与 ha-vcs 把
// worktree / project_bootstrap 留 kernel **不同**（那两个没造出反向边），
// 别拿它当先例。更硬的一点：`list_loop_schedules_for_session(_with_cron)` 是
// 写在 `loop_control.rs` 里的 `impl SessionDB` 方法（同样是分析器不计的方法
// 语法边），ha-cron 迁出时随之走人，本文件立刻调不到。**ha-cron 那一刀必须
// 先解这条**，三选一：`LoopSchedule`/`LoopState` 与两个 impl 下沉 kernel、给
// activity 加钩子、或届时把 activity 一并迁走。
pub mod activity;
pub mod awareness;
pub mod backup;
pub mod browser_hooks;
pub mod channel;
pub mod channel_hooks;
pub mod chat_engine;
// `coding_eval` / `context_retrieval` / `evaluation` 已随阶段 5 第四刀迁出
// ha-eval-runtime——特征 crate 在 ha-core 之上，此处**不能**再导出。评测 wire
// 类型留 kernel（`coding_eval_defs`，见该模块文档）：kernel 的
// `coding_improvement` 存的就是这些报告的 JSON。
pub mod coding_eval_defs;
pub mod coding_improvement;
pub mod config;
pub mod context_compact;
pub mod crash_flush;
pub mod cron;
pub mod cron_defs;
pub mod cron_hooks;
pub mod dev_tools;
pub mod domain_eval;
pub mod domain_quality;
pub mod domain_workflow;
pub mod failover;
pub mod file_extract;
pub mod file_upload;
pub mod filesystem;
pub mod git_control;
pub mod goal;
pub mod guardian;
pub mod hooks;
pub mod i18n;
pub mod issue_reporting;
pub mod knowledge;
pub mod learning_events;
// `local_embedding` / `local_llm` 已随阶段 5 首刀迁出 ha-local-llm——特征
// crate 在 ha-core 之上，此处**不能**再导出（会构成循环依赖）。台账面
// `local_model_jobs` 留 kernel：它是通用后台任务台账，memory reembed 与
// 知识库 reembed 同样靠它记账。
pub mod local_model_jobs;
pub mod loop_control;
pub mod lsp;
pub mod manual;

pub mod mcp;
pub mod mcp_hooks;
pub mod mcp_server;
pub mod media_gen;
pub mod memory;
pub mod memory_extract;
pub mod model_usage;
pub mod oauth;
pub mod onboarding;
pub mod openclaw_import;
pub mod permission;
pub mod pet;
pub mod plan;
pub mod process_notification;
pub mod project;
pub mod project_bootstrap;
pub mod provider;
// `dashboard` / `recap` 已随阶段 5 第二刀迁出 ha-dash（特征 crate
// 在 ha-core 之上，**不能**再导出）。kernel 侧只留 `/recap` 的分发钩子；
// Learning 埋点发布面在 `learning_events`，成本折算常量在 `provider`。
pub mod recap_hooks;
pub mod review;
pub mod runtime_tasks;
pub mod sandbox;
pub mod self_diagnosis;
pub mod server_status;
pub mod session;
pub mod session_title;
pub mod settings_reset;
pub mod skills;
pub mod slash_commands;
pub mod slash_defs;
pub mod slash_hooks;
pub mod sprite;
pub mod stt;
pub mod subagent;
pub mod system_prompt;
pub mod team;
pub mod tool_actions;
pub mod tool_defs;
pub mod tools;
pub mod turn_durability;
pub mod url_preview;
pub mod user_config;
pub mod vcs_hooks;
pub mod verification;
pub mod wakeup;
pub mod workflow;
pub mod worktree;

// ── Re-exports ────────────────────────────────────────────────────
pub use app_init::{
    app_version, build_app_state, init_app_state, init_runtime, set_app_version,
    start_background_tasks, start_minimal_background_tasks,
};
#[allow(deprecated)]
pub use globals::{
    get_app_handle, get_cached_agent, get_channel_cancels, get_channel_db, get_channel_registry,
    get_codex_token_cache, get_cron_db, get_event_bus, get_knowledge_db, get_memory_backend,
    get_project_db, get_reasoning_effort_cell, get_session_db, get_subagent_cancels,
    get_terminal_manager, require_cached_agent, require_channel_cancels, require_codex_token_cache,
    require_cron_db, require_knowledge_db, require_project_db, require_reasoning_effort_cell,
    require_session_db, require_subagent_cancels, require_terminal_manager, set_event_bus,
    AppState, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY, CODEX_TOKEN_CACHE,
    CRON_DB, EVENT_BUS, KNOWLEDGE_DB, MEMORY_BACKEND, PROJECT_DB, REASONING_EFFORT, SESSION_DB,
    SUBAGENT_CANCELS, TERMINAL_MANAGER,
};
