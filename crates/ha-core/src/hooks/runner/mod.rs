//! Hook handler execution layer (design doc §7).
//!
//! Every handler type (`command` / `http` / `mcp_tool` / `prompt` / `agent`)
//! implements [`HookHandler`] and produces a [`RawHookResult`] that the output
//! parser (`parse.rs`) turns into a `HookContribution`. This phase only ships
//! the `command` runner (`command.rs`).

use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::env::HookEnv;
use super::types::HookInput;

pub mod command;

/// Raw output of running one handler, before protocol parsing.
#[derive(Debug, Clone)]
pub struct RawHookResult {
    /// `None` when the handler has no exit-code concept (http).
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub timed_out: bool,
}

impl RawHookResult {
    /// A clean "nothing to report" result (exit 0, empty) — used for
    /// fire-and-forget `async` handlers and as a safe default.
    pub fn noop() -> Self {
        Self {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::ZERO,
            timed_out: false,
        }
    }

    /// A non-blocking error (exit 1) carrying a stderr message.
    pub fn non_blocking_error(stderr: impl Into<String>) -> Self {
        Self {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: stderr.into(),
            duration: Duration::ZERO,
            timed_out: false,
        }
    }
}

/// A runnable hook handler.
#[async_trait]
pub trait HookHandler: Send + Sync {
    /// Stable identity for dedup (design doc §7.7): command string, URL,
    /// prompt hash, etc.
    fn identity(&self) -> String;

    /// `"command" | "http" | "mcp_tool" | "prompt" | "agent"`.
    fn handler_type(&self) -> &'static str;

    /// Default timeout when the handler config doesn't override it.
    fn default_timeout(&self) -> Duration;

    /// Execute the handler. Must respect `deadline`.
    async fn run(&self, input: &HookInput, env: &HookEnv, deadline: Instant) -> RawHookResult;
}
