//! Transport factories — wire an [`McpTransportSpec`] up to an rmcp client.
//!
//! Phase 2 only wires stdio. Phase 4 extends this with Streamable HTTP,
//! SSE, and the `tokio-tungstenite` WebSocket wrapper. Each transport
//! returns something that implements `rmcp::transport::Transport<RoleClient>`.
//!
//! Env placeholder expansion happens here (not in `client.rs`) so the
//! narrow "how to spawn" knowledge stays isolated; the rest of the
//! subsystem sees only a ready-to-hand transport.

use std::collections::BTreeMap;
use std::process::Stdio;

use rmcp::transport::child_process::ConfigureCommandExt;
use rmcp::transport::TokioChildProcess;
use tokio::process::{ChildStderr, Command};

use super::config::{expand_placeholders, McpServerConfig, McpTransportSpec};
use super::errors::{McpError, McpResult};

/// Minimal list of env vars inherited from the parent process when we
/// spawn a subprocess. Stops surprises like "works on my machine because
/// I have `AWS_PROFILE` in my shell" from making MCP servers behave
/// differently between the desktop GUI and the HTTP server mode.
///
/// Anything the server genuinely needs must be declared in the server's
/// [`McpServerConfig::env`] block.
const INHERITED_ENV_WHITELIST: &[&str] = &[
    "HOME", "USER", "PATH", "LANG", "LC_ALL", "TZ", "TMPDIR", "TEMP", "TMP",
];

/// Build a `tokio::process::Command` from a `Stdio` transport spec, applying:
/// * env placeholder expansion (`${VAR}` / `$VAR`), looking up in the
///   server's own `env` block first, then falling back to the real env.
/// * env whitelisting — only whitelisted vars inherit from the parent,
///   plus the server's explicit `env` entries on top.
/// * optional `cwd`.
fn build_stdio_command(cfg: &McpServerConfig) -> McpResult<Command> {
    let (command, args, cwd) = match &cfg.transport {
        McpTransportSpec::Stdio { command, args, cwd } => (command, args, cwd),
        _ => unreachable!("build_stdio_command called on non-stdio transport"),
    };

    // 1. Expand `${VAR}` in the server's env values using the process
    //    env as fallback. Keys are never expanded (they're identifiers).
    let expanded_env: BTreeMap<String, String> = cfg
        .env
        .iter()
        .map(|(k, v)| {
            let expanded = expand_placeholders(v, |name| std::env::var(name).ok());
            (k.clone(), expanded)
        })
        .collect();

    // 2. Build the final env map: whitelist inherit + expanded overrides.
    //    Explicit server entries win over the inherited defaults.
    let mut final_env: BTreeMap<String, String> = BTreeMap::new();
    for key in INHERITED_ENV_WHITELIST {
        if let Ok(v) = std::env::var(key) {
            final_env.insert((*key).to_string(), v);
        }
    }
    for (k, v) in expanded_env {
        final_env.insert(k, v);
    }

    // 3. Expand `${VAR}` in each argv slot using the *final* env first,
    //    then the process env. This lets users template the real command
    //    on values they just declared in `env`.
    let expanded_args: Vec<String> = args
        .iter()
        .map(|a| {
            expand_placeholders(a, |name| {
                final_env
                    .get(name)
                    .cloned()
                    .or_else(|| std::env::var(name).ok())
            })
        })
        .collect();

    // 4. Expand cwd similarly. Unknown variable → empty substring, which
    //    should produce a visible error from the OS (ENOENT) rather than
    //    silent failure.
    let expanded_cwd = cwd.as_ref().map(|c| {
        expand_placeholders(c, |name| {
            final_env
                .get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
        })
    });

    let mut cmd = Command::new(command);
    cmd.args(&expanded_args).env_clear();
    for (k, v) in final_env {
        cmd.env(k, v);
    }
    if let Some(dir) = expanded_cwd {
        if !dir.is_empty() {
            cmd.current_dir(dir);
        }
    }
    Ok(cmd)
}

/// Output of a successful transport build: the rmcp transport plus any
/// side-channel handles the caller needs to drive (e.g. the stdio
/// `ChildStderr` handle, owned by `client.rs` so it can spawn a tailer
/// task with the right server-name prefix).
pub struct BuiltTransport {
    pub inner: TokioChildProcess,
    pub stderr: Option<ChildStderr>,
}

/// Spawn the subprocess described by `cfg.transport` (stdio only for
/// Phase 2) and return the rmcp child-process transport along with a
/// captured `stderr` handle for log tailing. The transport stores the
/// child handle internally and cleans up on drop.
///
/// The caller is responsible for feeding the returned transport to
/// `rmcp::ServiceExt::serve()` with a `ClientHandler` (we use `()`), and
/// for draining `stderr` — otherwise a verbose server can fill its
/// stderr pipe and block.
pub fn build_stdio_transport(cfg: &McpServerConfig) -> McpResult<BuiltTransport> {
    let cmd = build_stdio_command(cfg)?;
    let (proc, stderr) = TokioChildProcess::builder(cmd.configure(|_| {}))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| McpError::Transport {
            server: cfg.name.clone(),
            source: format!("spawn failed: {e}"),
        })?;
    Ok(BuiltTransport {
        inner: proc,
        stderr,
    })
}

/// Convenience entry: dispatch on the transport kind and build whatever
/// transport the spec calls for. In Phase 2 anything non-stdio returns
/// an explicit `NotReady` with a helpful message so the GUI can render
/// "not implemented yet" instead of a generic failure.
///
/// Phase 4 replaces the non-stdio branches with real implementations.
pub fn build_transport_for(cfg: &McpServerConfig) -> McpResult<BuiltTransport> {
    match &cfg.transport {
        McpTransportSpec::Stdio { .. } => build_stdio_transport(cfg),
        other => Err(McpError::NotReady {
            server: cfg.name.clone(),
            reason: format!(
                "{} transport is not implemented in this build (phase 2 is stdio-only)",
                other.kind_label()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};

    fn stdio_cfg(command: &str) -> McpServerConfig {
        McpServerConfig {
            id: "id-1".into(),
            name: "t".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: command.into(),
                args: vec!["-x".into(), "${FOO}".into()],
                cwd: None,
            },
            env: [("FOO".into(), "from-env".into())].into_iter().collect(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    #[test]
    fn build_stdio_command_expands_args_from_env_block() {
        let cfg = stdio_cfg("echo");
        let cmd = build_stdio_command(&cfg).unwrap();
        // `std::process::Command` (via tokio wrapper) doesn't expose
        // its argv publicly; we use `get_args()` on the std command.
        let std_cmd: &std::process::Command = cmd.as_std();
        let args: Vec<_> = std_cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["-x".to_string(), "from-env".to_string()]);
    }

    #[test]
    fn build_stdio_command_whitelists_env() {
        let cfg = stdio_cfg("echo");
        let cmd = build_stdio_command(&cfg).unwrap();
        let std_cmd: &std::process::Command = cmd.as_std();
        let envs: std::collections::HashMap<String, Option<String>> = std_cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|s| s.to_string_lossy().into_owned()),
                )
            })
            .collect();
        // FOO must have been passed through.
        assert!(envs.contains_key("FOO"));
        // A variable that is NOT in the whitelist and NOT in cfg.env
        // should not have been forwarded. We use `PWD` as a probe since
        // it's almost always present in the parent env but we deliberately
        // left it off the whitelist.
        assert!(!envs.contains_key("PWD"));
    }

    #[test]
    fn non_stdio_not_implemented_in_phase_2() {
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::StreamableHttp {
            url: "https://example.com/mcp".into(),
        };
        // `TokioChildProcess` doesn't impl `Debug`, so `unwrap_err()` won't
        // compile — match the Result explicitly.
        match build_transport_for(&cfg) {
            Err(McpError::NotReady { .. }) => {}
            Err(other) => panic!("unexpected error variant: {other:?}"),
            Ok(_) => panic!("expected NotReady, got a transport"),
        }
    }
}
