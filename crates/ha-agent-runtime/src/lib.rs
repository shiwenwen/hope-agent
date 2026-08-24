//! Main-turn execution feature wired above `ha-core`.
//!
//! Owns the admitted-turn engine, Provider protocol adapters, one-shot network
//! runtime, Hope round/tool driver and vision bridge. `ha-core` retains turn
//! admission, policy, permission, durability and terminal ledgers; this crate
//! consumes those capabilities only through the registered stable ports.

#[macro_use]
extern crate ha_core;

use std::sync::Once;

mod chat_dispatch;
mod engine;
mod one_shot;
mod provider_adapters;
mod streaming_loop;
mod vision_bridge;

mod api_types {
    pub use ha_core::agent::api_types::*;
}
mod config {
    pub use ha_core::agent::config::*;
    pub use ha_core::config::{cached_config, mutate_config, AppConfig};
}
mod content {
    pub use ha_core::agent::content::*;
}
mod context {
    pub use ha_core::agent::context::*;
}
mod cache_routing {
    pub use ha_core::{audit_fingerprint, keyed_digest};
}
mod errors {
    pub use ha_core::agent::errors::*;
}
mod events {
    pub use ha_core::agent::events::*;
}
mod streaming_adapter {
    pub use ha_core::agent::streaming_adapter::*;
}
mod token_manifest {
    pub use ha_core::agent::token_manifest::*;
}
mod types {
    pub use ha_core::agent::types::*;
}

pub use ha_core::{
    agent, app_debug, app_info, app_warn, async_jobs, attachments, awareness, blocking,
    chat_engine, context_compact, eval_context, failover, get_logger, get_session_db, hooks, lsp,
    mcp, model_usage, provider, recovery_control, security, session, skills, subagent,
    system_prompt, token_accounting, tool_defs, tools, truncate_utf8, ttl_cache, turn_durability,
    util,
};

static WIRE: Once = Once::new();
static ONE_SHOT_RUNTIME: one_shot::Runtime = one_shot::Runtime;
pub(crate) use chat_dispatch::run_agent_chat;

async fn execute_inner(
    turn: ha_core::turn_kernel::AdmittedTurn,
) -> Result<ha_core::turn_kernel::AgentTurnOutput, ha_core::chat_engine::TurnFailure> {
    let result = engine::execute_admitted_params(turn.into_runtime_params()).await?;
    Ok(ha_core::turn_kernel::AgentTurnOutput {
        response: result.response,
        model_used: result.model_used,
        usage: result.usage,
        terminal: result.terminal,
    })
}

fn execute(turn: ha_core::turn_kernel::AdmittedTurn) -> ha_core::turn_kernel::AgentTurnFuture {
    Box::pin(execute_inner(turn))
}

/// Register the required main-turn executor. Repeated shell wiring is safe;
/// competing implementations still fail via the kernel's single-assignment
/// registry.
pub fn wire() {
    WIRE.call_once(|| {
        ha_core::agent::llm_adapter::register_one_shot_runtime(&ONE_SHOT_RUNTIME)
            .expect("ha-agent-runtime one-shot Provider runtime must register exactly once");
        ha_core::turn_kernel::register_agent_turn_executor(
            ha_core::turn_kernel::AgentTurnExecutor { execute },
        )
        .expect("ha-agent-runtime executor must be registered exactly once");
    });
}
