//! Goal agent-tool handlers. Names and schemas remain kernel contracts.

pub mod goal;

use ha_core::tools::registry::BuiltinToolEntry;

pub fn goal_dispatch_entries() -> Vec<BuiltinToolEntry> {
    use ha_core::tools::registry::tool_handler;
    vec![
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_STATUS,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(goal::tool_goal_status(args, ctx).await)),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_PREPARE_CONTRACT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(
                goal::tool_goal_prepare_contract(args, ctx).await
            )),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_CHECKPOINT,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(goal::tool_goal_checkpoint(args, ctx).await)),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_RECORD_EVIDENCE,
            aliases: &[],
            handler: tool_handler!(
                |args, ctx| Ok(goal::tool_goal_record_evidence(args, ctx).await)
            ),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_EVALUATE,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(goal::tool_goal_evaluate(args, ctx).await)),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_FINISH_REQUEST,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(goal::tool_goal_finish_request(args, ctx).await)),
        },
        BuiltinToolEntry {
            name: ha_core::tools::TOOL_GOAL_BLOCK_REQUEST,
            aliases: &[],
            handler: tool_handler!(|args, ctx| Ok(goal::tool_goal_block_request(args, ctx).await)),
        },
    ]
}
