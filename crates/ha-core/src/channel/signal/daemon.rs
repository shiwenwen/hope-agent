use anyhow::{Context, Result};
use std::net::TcpListener;
use std::time::Duration;

use crate::channel::process_manager::ManagedProcess;

/// Manages a `signal-cli daemon` child process.
pub struct SignalDaemon {
    process: Option<ManagedProcess>,
    port: u16,
}

impl SignalDaemon {
    /// Start the signal-cli daemon process.
    ///
    /// - `account`: E.164 phone number (e.g. "+1234567890")
    /// - `cli_path`: path to signal-cli binary, or None to use "signal-cli" from PATH
    /// - `port`: HTTP port to listen on, or None to auto-select a free port
    pub fn start(account: &str, cli_path: Option<&str>, port: Option<u16>) -> Result<Self> {
        // Windows: `signal-cli` is a Java app typically delivered as a
        // `.bat` launcher under an install path like
        // `C:\Program Files\signal-cli\bin\signal-cli.bat`. Bail with a
        // clear message instead of the generic "No such file or directory"
        // you get from spawning a bare `signal-cli` on Windows.
        #[cfg(windows)]
        if cli_path.is_none() {
            anyhow::bail!(
                "signal-cli is not on PATH. On Windows, set channels.signal.cli_path \
                 in config.json to the signal-cli.bat launcher (e.g. \
                 C:\\Program Files\\signal-cli\\bin\\signal-cli.bat)."
            );
        }

        let program = cli_path.unwrap_or("signal-cli");

        let port = match port {
            Some(p) => p,
            None => find_free_port()?,
        };

        // 用 127.0.0.1 而非 localhost：避免 IPv6/IPv4 双栈解析不一致——
        // signal-cli 内部 JVM 默认 IPv6 优先，client 侧 reqwest 也走 localhost
        // 时大多数环境 OK，但容器内 /etc/hosts 缺 ::1 会出现 daemon 监听
        // [::1]:port 而 client 走 127.0.0.1 失败的场景。统一 127.0.0.1。
        let http_addr = format!("127.0.0.1:{}", port);

        // signal-cli 的 --http 参数必须用 `=` 连接（picocli 框架对空格分隔
        // 敏感，部分版本会把 host:port 当下一个独立参数）。改成 `--http=...`
        // 单 token 形式，与 signal-cli-jsonrpc.5.adoc 文档一致。
        let http_arg = format!("--http={}", http_addr);
        let args = vec!["-a", account, "daemon", &http_arg, "--no-receive-stdout"];

        app_info!(
            "channel",
            "signal-daemon",
            "Starting signal-cli daemon for {} on port {}",
            account,
            port
        );

        let process = ManagedProcess::spawn(program, &args)
            .with_context(|| format!("Failed to start signal-cli daemon for {}", account))?;

        Ok(Self {
            process: Some(process),
            port,
        })
    }

    /// Return the HTTP port the daemon is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Check if the daemon process is still running.
    pub fn is_running(&mut self) -> bool {
        match self.process {
            Some(ref mut p) => match p.try_wait() {
                Ok(None) => true,     // still running
                Ok(Some(_)) => false, // exited
                Err(_) => false,
            },
            None => false,
        }
    }

    /// Graceful shutdown: SIGTERM, wait up to 5s, then SIGKILL.
    pub async fn stop(&mut self) {
        if let Some(ref mut process) = self.process {
            app_info!(
                "channel",
                "signal-daemon",
                "Stopping signal-cli daemon on port {}",
                self.port
            );
            process.shutdown(Duration::from_secs(5)).await;
        }
        self.process = None;
    }
}

/// Find a free TCP port by binding to port 0.
fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("Failed to bind to ephemeral port for signal-cli daemon")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}
