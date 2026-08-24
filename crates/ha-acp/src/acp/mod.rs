//! ACP (Agent Client Protocol) module for Hope Agent.
//!
//! Provides a native Rust ACP server that IDE clients (Zed, VS Code, etc.)
//! can connect to via stdio + NDJSON (newline-delimited JSON-RPC 2.0).
//!
//! This is a native protocol adapter (not a process bridge): ACP requests enter
//! the shared TurnKernel and durable chat engine in-process.

pub mod agent;
pub mod event_mapper;
pub mod protocol;
pub mod server;
pub mod session;
pub mod types;

pub use agent::AcpAgent;
