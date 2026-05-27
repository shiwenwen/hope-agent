//! `command` hook handler — runs a shell command, feeds it the hook input on
//! stdin, captures stdout/stderr (design doc §7.2).
//!
//! Deliberately a thin spawner of its own rather than reusing
//! `tools::exec`'s spawner: that path is coupled to the process registry +
//! cancel watchers. We reuse only `exec::get_login_shell_path()` (for PATH).
//!
//! Phase 0.1 limitation: timeout kills the direct child (via `kill_on_drop`);
//! full SIGTERM→SIGKILL of the whole process group (§7.2) is deferred.

use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::super::config::{CommandHookConfig, HookShell};
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};

/// Default command-hook timeout (design §7.2 — note: *not* exec's 1800s).
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 600;
/// Per-stream capture cap (§7.9).
const MAX_CAPTURE_BYTES: usize = 1024 * 1024; // 1 MiB

pub struct CommandHandler {
    config: CommandHookConfig,
}

impl CommandHandler {
    pub fn new(config: CommandHookConfig) -> Self {
        Self { config }
    }

    /// `(program, args)` for the configured shell.
    fn shell_invocation(&self) -> (String, Vec<String>) {
        match self.config.shell {
            // Resolve `bash` via PATH rather than hardcoding `/bin/bash`, which
            // doesn't exist on NixOS/Guix and some BSDs. The child inherits the
            // resolved login-shell PATH (HookEnv), so this finds the user's bash.
            Some(HookShell::Bash) => (
                "bash".to_string(),
                vec!["-c".to_string(), self.config.command.clone()],
            ),
            Some(HookShell::Powershell) => (
                "pwsh".to_string(),
                vec!["-Command".to_string(), self.config.command.clone()],
            ),
            None => {
                #[cfg(unix)]
                {
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                    (shell, vec!["-c".to_string(), self.config.command.clone()])
                }
                #[cfg(windows)]
                {
                    (
                        "powershell".to_string(),
                        vec!["-Command".to_string(), self.config.command.clone()],
                    )
                }
            }
        }
    }

    fn build_command(&self, env: &HookEnv) -> Command {
        let (program, args) = self.shell_invocation();
        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Own process group so a timeout can terminate the whole tree (the
        // shell plus any children it forked), not just the direct child.
        #[cfg(unix)]
        cmd.process_group(0);
        // Run from the session/project cwd so a hook's relative paths
        // (`./scripts/fmt.sh`, `node_modules/.bin/...`) resolve against the
        // project root rather than the hope-agent process cwd. Guard on
        // `is_dir` — a stale/non-existent configured working dir would
        // otherwise make every spawn fail with ENOENT.
        if let Some(dir) = env.cwd.as_ref().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        for (k, v) in env.as_vars() {
            cmd.env(k, v);
        }
        cmd
    }
}

#[async_trait]
impl HookHandler for CommandHandler {
    fn identity(&self) -> String {
        // Include shell/timeout/async so two same-command hooks with different
        // execution semantics aren't collapsed into one by dedup.
        format!(
            "{}|shell={:?}|timeout={:?}|async={:?}",
            self.config.command, self.config.shell, self.config.timeout, self.config.async_run
        )
    }

    fn handler_type(&self) -> &'static str {
        "command"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS))
    }

    async fn run(&self, input: &HookInput, env: &HookEnv, deadline: Instant) -> RawHookResult {
        let start = Instant::now();
        let stdin_data = match serde_json::to_string(input) {
            Ok(s) => format!("{s}\n"), // trailing newline for `read`/`jq` friendliness
            Err(e) => {
                return RawHookResult::non_blocking_error(format!("serialize hook input: {e}"))
            }
        };

        let mut cmd = self.build_command(env);

        // `async: true` — fire-and-forget, doesn't affect the decision (§7.1).
        if self.config.async_run == Some(true) {
            // `asyncRewake`: capture output (pipe, don't /dev/null) so an
            // `exit 2` can inject the hook's stderr into the next turn (§7.1).
            if self.config.async_rewake == Some(true) {
                let session_id = input.common().session_id.clone();
                if let Ok(mut child) = cmd.spawn() {
                    let stdin = child.stdin.take();
                    let task = async move {
                        let write = async move {
                            if let Some(mut s) = stdin {
                                let _ = s.write_all(stdin_data.as_bytes()).await;
                            }
                        };
                        let (_, output) = tokio::join!(write, child.wait_with_output());
                        // Only exit 2 (the block code) rewakes; any other exit
                        // is plain fire-and-forget.
                        if let Ok(output) = output {
                            if output.status.code() == Some(2) {
                                let stderr = cap_utf8(&output.stderr);
                                crate::hooks::rewake_inject(&session_id, &stderr).await;
                            }
                        }
                    };
                    if let Some(rt) = crate::hooks::fire_and_forget_runtime() {
                        rt.spawn(task);
                    } else if tokio::runtime::Handle::try_current().is_ok() {
                        tokio::spawn(task);
                    }
                }
                return RawHookResult::noop();
            }

            // Plain async: the output is discarded anyway, so send stdout/stderr
            // to /dev/null rather than piping: the kernel drops it, so a chatty
            // or long-running hook (`yes`, a daemon, verbose logs) can neither
            // deadlock on a full ~64 KiB stdout pipe nor grow hope-agent's
            // memory unboundedly (which buffering the output to discard it
            // would). Overrides the piped stdout/stderr `build_command` set.
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            if let Ok(mut child) = cmd.spawn() {
                let stdin = child.stdin.take();
                let task = async move {
                    // Write stdin CONCURRENTLY with the wait so a large hook
                    // input can't deadlock the stdin pipe against a hook that
                    // reads it lazily.
                    let write = async move {
                        if let Some(mut s) = stdin {
                            let _ = s.write_all(stdin_data.as_bytes()).await;
                            // `s` dropped here → EOF on the child's stdin.
                        }
                    };
                    let _ = tokio::join!(write, child.wait());
                };
                // Spawn on the process-lived runtime so the detached child
                // survives a short-lived caller runtime (e.g. ACP's per-turn
                // current-thread rt, which would otherwise abort it on drop).
                if let Some(rt) = crate::hooks::fire_and_forget_runtime() {
                    rt.spawn(task);
                } else if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::spawn(task);
                }
            }
            return RawHookResult::noop();
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return RawHookResult::non_blocking_error(format!("spawn hook: {e}")),
        };
        let child_pid = child.id();

        // Write stdin CONCURRENTLY with draining stdout/stderr. Writing the
        // whole payload before reading deadlocks once the child fills its
        // stdout pipe (~64 KiB) and stops reading stdin — common for hooks
        // that echo their input (`cat`, `jq .`, `tee`).
        let stdin = child.stdin.take();
        let write_stdin = async move {
            if let Some(mut s) = stdin {
                let _ = s.write_all(stdin_data.as_bytes()).await;
                // `s` dropped here → child sees EOF on stdin.
            }
        };

        let timeout = deadline.saturating_duration_since(Instant::now());
        let combined = async {
            let (_, output) = tokio::join!(write_stdin, child.wait_with_output());
            output
        };
        match tokio::time::timeout(timeout, combined).await {
            Ok(Ok(output)) => RawHookResult {
                exit_code: output.status.code(),
                stdout: cap_utf8(&output.stdout),
                stderr: cap_utf8(&output.stderr),
                duration: start.elapsed(),
                timed_out: false,
            },
            Ok(Err(e)) => RawHookResult::non_blocking_error(format!("hook io error: {e}")),
            Err(_) => {
                // Timed out. `kill_on_drop` reaps only the direct child when
                // the cancelled future drops; explicitly kill the whole process
                // group so forked grandchildren don't leak.
                if let Some(pid) = child_pid {
                    crate::platform::terminate_process_tree(pid);
                }
                RawHookResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("hook timed out after {}s", timeout.as_secs()),
                    duration: start.elapsed(),
                    timed_out: true,
                }
            }
        }
    }
}

fn cap_utf8(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    crate::truncate_utf8(&s, MAX_CAPTURE_BYTES).to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::hooks::types::{CommonHookInput, PermissionMode};
    use std::path::PathBuf;

    fn dummy_input() -> HookInput {
        HookInput::Notification {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/tmp"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "Notification".into(),
                agent_id: None,
                agent_type: None,
            },
            notification_type: "idle_prompt".into(),
            message: "hi".into(),
            title: None,
        }
    }

    fn deadline(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    #[tokio::test]
    async fn echo_stdout_exit_zero() {
        let h = CommandHandler::new(CommandHookConfig {
            command: "printf '%s' hello".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
});
        let r = h.run(&dummy_input(), &HookEnv::empty(), deadline(10)).await;
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.stdout, "hello");
        assert!(!r.timed_out);
    }

    #[tokio::test]
    async fn nonzero_exit_is_captured() {
        let h = CommandHandler::new(CommandHookConfig {
            command: "echo oops 1>&2; exit 2".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
});
        let r = h.run(&dummy_input(), &HookEnv::empty(), deadline(10)).await;
        assert_eq!(r.exit_code, Some(2));
        assert!(r.stderr.contains("oops"));
    }

    #[tokio::test]
    async fn stdin_receives_hook_input_json() {
        // The hook echoes back its stdin; we assert the JSON arrived.
        let h = CommandHandler::new(CommandHookConfig {
            command: "cat".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
});
        let r = h.run(&dummy_input(), &HookEnv::empty(), deadline(10)).await;
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout.contains("\"hook_event_name\":\"Notification\""));
        assert!(r.stdout.contains("\"session_id\":\"s1\""));
    }

    #[tokio::test]
    async fn timeout_marks_timed_out() {
        let h = CommandHandler::new(CommandHookConfig {
            command: "sleep 5".into(),
            shell: Some(HookShell::Bash),
            timeout: Some(1),
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
});
        // 1s deadline against a 5s sleep.
        let r = h.run(&dummy_input(), &HookEnv::empty(), deadline(1)).await;
        assert!(r.timed_out);
        assert_eq!(r.exit_code, None);
    }

    #[tokio::test]
    async fn runs_in_configured_cwd() {
        // Unique subdir under the temp root so the assertion isn't fooled by
        // /tmp→/private/tmp symlink canonicalization (we match the unique leaf,
        // not the full path).
        let leaf = format!("ha-hook-cwd-{}", std::process::id());
        let dir = std::env::temp_dir().join(&leaf);
        std::fs::create_dir_all(&dir).unwrap();

        let env = HookEnv {
            vars: Default::default(),
            cwd: Some(dir.clone()),
        };
        let h = CommandHandler::new(CommandHookConfig {
            command: "pwd".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
});
        let r = h.run(&dummy_input(), &env, deadline(10)).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(r.exit_code, Some(0));
        assert!(
            r.stdout.contains(&leaf),
            "expected pwd to land in {leaf}, got {:?}",
            r.stdout
        );
    }

    #[test]
    fn identity_distinguishes_shell_and_timeout() {
        let base = CommandHookConfig {
            command: "x".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: None,
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: None,
};
        let mut diff_shell = base.clone();
        diff_shell.shell = Some(HookShell::Powershell);
        let mut diff_timeout = base.clone();
        diff_timeout.timeout = Some(30);

        let id_base = CommandHandler::new(base.clone()).identity();
        assert_ne!(id_base, CommandHandler::new(diff_shell).identity());
        assert_ne!(id_base, CommandHandler::new(diff_timeout).identity());
        // Same config → same identity (dedup still collapses true duplicates).
        assert_eq!(id_base, CommandHandler::new(base).identity());
    }

    #[tokio::test]
    async fn async_rewake_is_fire_and_forget() {
        // An `asyncRewake` command is still fire-and-forget: `run` returns a
        // no-op immediately (exit code None) without blocking on the child or
        // the injection. The detached task captures output for the rewake path,
        // but with no real session the injection is a harmless no-op.
        let h = CommandHandler::new(CommandHookConfig {
            command: "exit 2".into(),
            shell: Some(HookShell::Bash),
            timeout: None,
            async_run: Some(true),
            status_message: None,
            if_rule: None,
            once: None,
            async_rewake: Some(true),
        });
        let r = h.run(&dummy_input(), &HookEnv::empty(), deadline(10)).await;
        // Fire-and-forget → immediate no-op (exit 0, no captured output, not a
        // timeout); the child + injection run detached.
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout.is_empty() && r.stderr.is_empty());
        assert!(!r.timed_out);
    }
}
