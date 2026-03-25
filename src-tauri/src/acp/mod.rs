//! ACP (Agent Client Protocol) module for OpenComputer.
//!
//! Provides a native Rust ACP server that IDE clients (Zed, VS Code, etc.)
//! can connect to via stdio + NDJSON (newline-delimited JSON-RPC 2.0).
//!
//! This is a direct implementation (not a bridge) — ACP requests drive
//! the local AssistantAgent directly, with zero intermediary latency.

pub mod agent;
pub mod event_mapper;
pub mod protocol;
pub mod server;
pub mod session;
pub mod types;

pub use agent::AcpAgent;
