//! ACP Control Plane — Client-side ACP runtime management.
//!
//! Enables Hope Agent's agent to spawn and control external ACP-compatible
//! agents (Claude Code, Codex CLI, Gemini CLI, etc.) as child processes.
//!
//! This is the **client** counterpart to `crate::acp` (the server).

pub mod config;
pub mod events;
pub mod health;
pub mod registry;
pub mod runtime_stdio;
pub mod session_manager;
pub mod types;

pub use config::{AcpControlConfig, AgentAcpConfig};
pub use registry::AcpRuntimeRegistry;
pub use session_manager::AcpSessionManager;
