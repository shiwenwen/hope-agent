//! Shared utility for managing external child processes (signal-cli, imsg, etc.).

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// A managed child process with line-based stdout/stderr readers.
pub struct ManagedProcess {
    child: Child,
    /// Channel that receives lines from stdout.
    pub stdout_rx: mpsc::Receiver<String>,
    /// Channel that receives lines from stderr.
    pub stderr_rx: mpsc::Receiver<String>,
}

impl ManagedProcess {
    /// Spawn a child process with the given command and args.
    /// stdout and stderr are captured and forwarded as lines.
    pub fn spawn(program: &str, args: &[&str]) -> Result<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to spawn '{}'. Is it installed and in your PATH?",
                    program
                )
            })?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (stdout_tx, stdout_rx) = mpsc::channel(256);
        let (stderr_tx, stderr_rx) = mpsc::channel(256);

        // Spawn stdout reader
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if stdout_tx.send(line).await.is_err() {
                    break;
                }
            }
        });

        // Spawn stderr reader
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if stderr_tx.send(line).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            stdout_rx,
            stderr_rx,
        })
    }

    /// Get a mutable reference to the child's stdin for writing.
    pub fn stdin(&mut self) -> Option<&mut tokio::process::ChildStdin> {
        self.child.stdin.as_mut()
    }

    /// Check if the process is still running.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// Graceful shutdown: ask the child to stop, wait up to `timeout`,
    /// then force-kill. Unix sends `SIGTERM` to the pid; Windows calls
    /// `taskkill /PID` which delivers a WM_CLOSE / CTRL_BREAK depending
    /// on the child type.
    pub async fn shutdown(&mut self, timeout: std::time::Duration) {
        if let Some(pid) = self.child.id() {
            crate::platform::send_graceful_stop(pid);
        }

        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = self.child.kill().await;
            }
        }
    }
}

/// Check if a binary is available in PATH.
pub fn find_binary(name: &str) -> Option<std::path::PathBuf> {
    which::which(name).ok()
}
