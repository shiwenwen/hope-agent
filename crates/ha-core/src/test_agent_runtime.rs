//! Test-only same-source view of the feature-owned Provider runtime.
//!
//! `chat_engine` retains kernel contract tests that exercise failover and
//! durability against local HTTP fixtures. Reusing the feature source keeps
//! those tests representative without compiling Provider implementations into
//! the production `ha-core` target.

mod api_types {
    pub use crate::agent::api_types::*;
}
mod config {
    pub use crate::agent::config::*;
}
mod content {
    pub use crate::agent::content::*;
}
mod context {
    pub use crate::agent::context::*;
}
mod errors {
    pub use crate::agent::errors::*;
}
mod events {
    pub use crate::agent::events::*;
}
mod streaming_adapter {
    pub use crate::agent::streaming_adapter::*;
}
mod token_manifest {
    pub use crate::agent::token_manifest::*;
}
mod types {
    pub use crate::agent::types::*;
}

#[path = "../../ha-agent-runtime/src/chat_dispatch.rs"]
mod chat_dispatch;
#[path = "../../ha-agent-runtime/src/provider_adapters/mod.rs"]
mod provider_adapters;
#[path = "../../ha-agent-runtime/src/streaming_loop.rs"]
mod streaming_loop;
#[path = "../../ha-agent-runtime/src/vision_bridge.rs"]
mod vision_bridge;

pub(crate) use chat_dispatch::run_agent_chat;
