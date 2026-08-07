//! Builtin 工具分发条目（阶段 2.5：由 `execution.rs` 静态 match 机械迁移，
//! 行为零变化——每个条目的表达式与原 match 臂逐字对应，仅 `dispatch_ctx`
//! 更名 `ctx`、裸模块路径补 `super::`）。
//!
//! 新增内置工具：在此加条目（勿在 dispatch 里重新写 match 臂）；特征 crate
//! 的工具走 `registry::register_external_tools`（装配期）。

use super::registry::{tool_handler, BuiltinToolEntry};

#[rustfmt::skip]
pub(crate) fn builtin_entries() -> Vec<BuiltinToolEntry> {
    vec![
        BuiltinToolEntry { name: super::TOOL_EXEC, aliases: &[], handler: tool_handler!(|args, ctx| super::exec::tool_exec(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_PROCESS, aliases: &[], handler: tool_handler!(|args, ctx| super::process::tool_process(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_READ, aliases: &["read_file"], handler: tool_handler!(|args, ctx| super::read::tool_read_file(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_WRITE, aliases: &["write_file"], handler: tool_handler!(|args, ctx| super::write::tool_write_file(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_EDIT, aliases: &["patch_file"], handler: tool_handler!(|args, ctx| super::edit::tool_edit(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_LS, aliases: &["list_dir"], handler: tool_handler!(|args, ctx| super::ls::tool_ls(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_LSP, aliases: &[], handler: tool_handler!(|args, ctx| super::lsp::tool_lsp(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_GREP, aliases: &[], handler: tool_handler!(|args, ctx| super::grep::tool_grep(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_FIND, aliases: &[], handler: tool_handler!(|args, ctx| super::find::tool_find(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_APPLY_PATCH, aliases: &[], handler: tool_handler!(|args, ctx| super::apply_patch::tool_apply_patch(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_WEB_SEARCH, aliases: &[], handler: tool_handler!(|args, ctx| super::web_search::tool_web_search(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_WEB_FETCH, aliases: &[], handler: tool_handler!(|args, ctx| super::web_fetch::tool_web_fetch(args).await) },
        BuiltinToolEntry { name: super::TOOL_SAVE_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_save_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_RECALL_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_recall_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_UPDATE_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_update_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_DELETE_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_delete_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_UPDATE_CORE_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_update_core_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_CORE_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::core_memory::tool_core_memory(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_PROJECT_MEMORY, aliases: &[], handler: tool_handler!(|args, ctx| super::project_memory::tool_project_memory(args, ctx).await) },
        // `manage_cron` 的 handler 随 ha-cron 迁出，由 `ha_cron::wire()` 注册外部
        // 分发条目；schema 仍在 definitions::core_tools。
        BuiltinToolEntry { name: super::TOOL_SEND_NOTIFICATION, aliases: &[], handler: tool_handler!(|args, ctx| super::notification::tool_send_notification(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_SUBAGENT, aliases: &[], handler: tool_handler!(|args, ctx| super::subagent::tool_subagent(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_TEAM, aliases: &[], handler: tool_handler!(|args, ctx| super::team::tool_team(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_WORKFLOW, aliases: &[], handler: tool_handler!(|args, ctx| super::workflow_tool::tool_workflow(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_MEMORY_GET, aliases: &[], handler: tool_handler!(|args, ctx| super::memory::tool_memory_get(args, ctx).await) },
        // 24 个知识空间 handler（22 个 `note_*` + `knowledge_recall` +
        // `session_to_note`）随 ha-knowledge 迁出，由 `ha_knowledge::wire()`
        // 注册外部分发条目；schema 仍在 definitions::core_tools。
        BuiltinToolEntry { name: super::TOOL_AGENTS_LIST, aliases: &[], handler: tool_handler!(|args, ctx| super::agents::tool_agents_list(args).await) },
        BuiltinToolEntry { name: super::TOOL_SESSIONS_LIST, aliases: &[], handler: tool_handler!(|args, ctx| super::sessions::tool_sessions_list(args).await) },
        BuiltinToolEntry { name: super::TOOL_SESSION_STATUS, aliases: &[], handler: tool_handler!(|args, ctx| super::sessions::tool_session_status(args).await) },
        BuiltinToolEntry { name: super::TOOL_SESSIONS_SEARCH, aliases: &[], handler: tool_handler!(|args, ctx| super::sessions::tool_sessions_search(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_SESSIONS_HISTORY, aliases: &[], handler: tool_handler!(|args, ctx| super::sessions::tool_sessions_history(args).await) },
        BuiltinToolEntry { name: super::TOOL_SESSIONS_SEND, aliases: &[], handler: tool_handler!(|args, ctx| Box::pin(super::sessions::tool_sessions_send(args, ctx)).await) },
        BuiltinToolEntry { name: super::TOOL_IMAGE, aliases: &[], handler: tool_handler!(|args, ctx| super::image::tool_image(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_ISSUE_REPORT, aliases: &[], handler: tool_handler!(|args, ctx| super::issue_report::tool_issue_report(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_PDF, aliases: &[], handler: tool_handler!(|args, ctx| super::pdf::tool_pdf(args).await) },
        BuiltinToolEntry { name: super::TOOL_ASK_USER_QUESTION, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::ask_user_question::execute(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_ENTER_PLAN_MODE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::enter_plan_mode::execute(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_SUBMIT_PLAN, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::submit_plan::execute(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_TASK_CREATE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::task::tool_task_create(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_TASK_UPDATE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::task::tool_task_update(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_TASK_LIST, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::task::tool_task_list(args, ctx.session_id.as_deref()).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_STATUS, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_status(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_PREPARE_CONTRACT, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_prepare_contract(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_CHECKPOINT, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_checkpoint(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_RECORD_EVIDENCE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_record_evidence(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_EVALUATE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_evaluate(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_FINISH_REQUEST, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_finish_request(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_GOAL_BLOCK_REQUEST, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::goal::tool_goal_block_request(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_STATUS, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_status(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_RESCHEDULE, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_reschedule(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_STOP, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_stop(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_RECORD_PROGRESS, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_record_progress(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_WATCH, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_watch(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_LOOP_UNWATCH, aliases: &[], handler: tool_handler!(|args, ctx| Ok(super::loop_tool::tool_loop_unwatch(args, ctx).await)) },
        BuiltinToolEntry { name: super::TOOL_JOB_STATUS, aliases: &[], handler: tool_handler!(|args, ctx| super::job_status::tool_job_status(args, ctx.session_id.as_deref()).await) },
        BuiltinToolEntry { name: super::TOOL_SCHEDULE_WAKEUP, aliases: &[], handler: tool_handler!(|args, ctx| super::schedule_wakeup::tool_schedule_wakeup(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_RUNTIME_CANCEL, aliases: &[], handler: tool_handler!(|args, ctx| super::runtime_cancel::tool_runtime_cancel(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_TOOL_SEARCH, aliases: &[], handler: tool_handler!(|args, ctx| super::tool_search::tool_search(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_PEEK_SESSIONS, aliases: &[], handler: tool_handler!(|args, ctx| crate::awareness::run_peek_sessions(args, ctx.session_id.as_deref()) .map_err(|e| anyhow::anyhow!(e))) },
        BuiltinToolEntry { name: super::TOOL_GET_SETTINGS, aliases: &[], handler: tool_handler!(|args, ctx| super::settings::tool_get_settings(args).await) },
        BuiltinToolEntry { name: super::TOOL_UPDATE_SETTINGS, aliases: &[], handler: tool_handler!(|args, ctx| super::settings::tool_update_settings(args, ctx).await) },
        BuiltinToolEntry { name: super::TOOL_LIST_SETTINGS_BACKUPS, aliases: &[], handler: tool_handler!(|args, ctx| super::settings::tool_list_settings_backups(args).await) },
        BuiltinToolEntry { name: super::TOOL_RESTORE_SETTINGS_BACKUP, aliases: &[], handler: tool_handler!(|args, ctx| super::settings::tool_restore_settings_backup(args).await) },
        BuiltinToolEntry { name: super::TOOL_SEND_ATTACHMENT, aliases: &[], handler: tool_handler!(|args, ctx| super::send_attachment::tool_send_attachment(args, ctx).await) },
        // `skill` handler 随 ha-skills 迁出，由 `ha_skills::wire()` 注册外部
        // 分发条目；名字常量与 schema 仍在 tool_defs / definitions::core_tools。
    ]
}
