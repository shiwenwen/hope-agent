//! 飞书业务工具 adapter。名字常量在 kernel 的
//! [`ha_core::tool_defs::feishu_names`]（`permission::engine` 要按名字裁决），
//! 此处只放 schema 与 handler。

pub mod feishu;

use ha_core::tools::registry::BuiltinToolEntry;

/// 35 条分发条目。与 [`feishu::get_feishu_tools`] 的 schema 一一对应——
/// 少一边就是「能跑但不受 `tools.allow/deny` 约束」或「有 schema 无 handler」。
pub fn feishu_dispatch_entries() -> Vec<BuiltinToolEntry> {
    use ha_core::tools::registry::tool_handler;
    vec![
        BuiltinToolEntry {
            name: feishu::TOOL_DOCX_CREATE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::docx::execute_create(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DOCX_GET_BLOCKS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::docx::execute_get_blocks(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DOCX_APPEND_BLOCK,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::docx::execute_append_block(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DOCX_UPDATE_BLOCK_TEXT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::docx::execute_update_block_text(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_LIST_RECORDS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_list_records(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_SEARCH_RECORDS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_search_records(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_CREATE_RECORD,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_create_record(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_BATCH_UPDATE_RECORDS,
            aliases: &[],
            handler: tool_handler!(
                |args, ctx| feishu::bitable::execute_batch_update_records(args).await
            ),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_LIST_VIEWS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_list_views(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_GET_VIEW,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_get_view(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_BITABLE_LIST_DASHBOARDS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::bitable::execute_list_dashboards(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DRIVE_LIST_FILES,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::drive::execute_list_files(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DRIVE_UPLOAD_MEDIA,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::drive::execute_upload_media(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_DRIVE_DOWNLOAD_MEDIA,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::drive::execute_download_media(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_WIKI_GET_NODE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::wiki::execute_get_node(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_APPROVAL_CREATE_INSTANCE,
            aliases: &[],
            handler: tool_handler!(
                |args, ctx| feishu::approval::execute_create_instance(args).await
            ),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_APPROVAL_GET_INSTANCE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::approval::execute_get_instance(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_APPROVAL_CANCEL_INSTANCE,
            aliases: &[],
            handler: tool_handler!(
                |args, ctx| feishu::approval::execute_cancel_instance(args).await
            ),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_APPROVAL_LIST_INSTANCES,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::approval::execute_list_instances(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_APPROVAL_SUBSCRIBE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::approval::execute_subscribe(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_LIST,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::calendar::execute_list(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_CREATE_EVENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::calendar::execute_create_event(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_LIST_EVENTS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::calendar::execute_list_events(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_UPDATE_EVENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::calendar::execute_update_event(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_DELETE_EVENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::calendar::execute_delete_event(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CALENDAR_ATTENDEES_CREATE,
            aliases: &[],
            handler: tool_handler!(
                |args, ctx| feishu::calendar::execute_attendees_create(args).await
            ),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CONTACT_GET_USER,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::contact::execute_get_user(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CONTACT_BATCH_GET_USERS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::contact::execute_batch_get_users(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CONTACT_GET_DEPARTMENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::contact::execute_get_department(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_CONTACT_SEARCH_USERS_BY_DEPARTMENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| {
                feishu::contact::execute_search_users_by_department(args).await
            }),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_HIRE_LIST_JOBS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::hire::execute_list_jobs(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_HIRE_GET_JOB,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::hire::execute_get_job(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_HIRE_LIST_TALENTS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::hire::execute_list_talents(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_HIRE_GET_TALENT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::hire::execute_get_talent(args).await),
        },
        BuiltinToolEntry {
            name: feishu::TOOL_HIRE_LIST_APPLICATIONS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| feishu::hire::execute_list_applications(args).await),
        },
    ]
}
