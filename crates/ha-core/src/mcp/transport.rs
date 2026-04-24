//! Transport factories — wire an [`McpTransportSpec`] up to an rmcp client.
//!
//! Phase 2 shipped stdio only. Phase 4 adds Streamable HTTP (the spec's
//! preferred remote transport) plus a best-effort SSE fallback routed
//! through the same client (rmcp 1.5 retired the standalone SSE client
//! in favor of Streamable HTTP's SSE sub-protocol).
//!
//! WebSocket still returns `NotReady` — implementing `rmcp::Transport`
//! over `tokio-tungstenite` is a bigger undertaking scheduled for a
//! follow-up pass.
//!
//! Every networked transport goes through the project SSRF policy
//! (`security::ssrf::check_url`) BEFORE we touch the network, so a
//! misconfigured private-network URL cannot exfiltrate through a rogue
//! `Authorization` header.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use rmcp::service::RunningService;
use rmcp::transport::child_process::ConfigureCommandExt;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use tokio::process::{ChildStderr, Command};

use super::config::{expand_placeholders, McpServerConfig, McpTransportSpec, McpTrustLevel};
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

/// A fully-served MCP client plus any side-channel handles the caller
/// needs to drive after the fact. `stderr` is `Some` only for stdio.
///
/// We construct the rmcp transport **and** call `.serve()` internally
/// here so the concrete reqwest-0.13 `Client` type rmcp uses for
/// Streamable HTTP never escapes this module (ha-core itself depends
/// on reqwest 0.12 through other call sites; mixing the two at the
/// type level causes a trait-resolution conflict).
pub struct ConnectedClient {
    pub running: RunningService<RoleClient, ()>,
    pub stderr: Option<ChildStderr>,
}

/// Spawn the subprocess described by a stdio transport spec and return
/// the connected rmcp client + stderr pipe. Caller must drain the
/// stderr pipe — otherwise a verbose server can fill its buffer and
/// block.
pub async fn build_stdio_client(cfg: &McpServerConfig) -> McpResult<ConnectedClient> {
    let cmd = build_stdio_command(cfg)?;
    let (proc, stderr) = TokioChildProcess::builder(cmd.configure(|_| {}))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| McpError::Transport {
            server: cfg.name.clone(),
            source: format!("spawn failed: {e}"),
        })?;
    let running = ().serve(proc).await.map_err(|e| McpError::Transport {
        server: cfg.name.clone(),
        source: format!("handshake failed: {e}"),
    })?;
    Ok(ConnectedClient { running, stderr })
}

/// Build a Streamable HTTP (or SSE → Streamable HTTP fallback) client
/// transport and complete the initial handshake. Runs the SSRF policy
/// check before constructing the underlying reqwest client so a
/// misconfigured private-network URL never dials out.
pub async fn build_http_client(cfg: &McpServerConfig, url: &str) -> McpResult<ConnectedClient> {
    let transport_label = cfg.transport.kind_label();
    // Expand `${VAR}` in user-provided header values + resolve URL
    // placeholders so the SSRF check sees the real destination.
    let expanded_url = expand_placeholders(url, |name| std::env::var(name).ok());

    // SSRF gate — trusted servers use the default policy, untrusted
    // servers get the strict policy. Any block lands on `McpError::Blocked`
    // which the GUI surfaces as "blocked by security policy".
    let app_cfg = crate::config::cached_config();
    let trusted_hosts = app_cfg.ssrf.trusted_hosts.clone();
    let policy = match cfg.trust_level {
        McpTrustLevel::Trusted => app_cfg.ssrf.default_policy,
        McpTrustLevel::Untrusted => crate::security::ssrf::SsrfPolicy::Strict,
    };
    crate::security::ssrf::check_url(&expanded_url, policy, &trusted_hosts)
        .await
        .map_err(|e| McpError::Blocked {
            server: cfg.name.clone(),
            reason: format!("SSRF policy blocked {transport_label} URL: {e}"),
        })?;

    let mut headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
    for (k, v) in &cfg.headers {
        let expanded = expand_placeholders(v, |name| std::env::var(name).ok());
        let name = HeaderName::from_str(k).map_err(|e| {
            McpError::Config(format!(
                "invalid header name '{k}' for server '{srv}': {e}",
                srv = cfg.name
            ))
        })?;
        let value = HeaderValue::from_str(&expanded).map_err(|e| {
            McpError::Config(format!(
                "invalid header value for '{k}' on server '{srv}': {e}",
                srv = cfg.name
            ))
        })?;
        headers.insert(name, value);
    }

    let http_cfg =
        StreamableHttpClientTransportConfig::with_uri(expanded_url).custom_headers(headers);
    let transport = StreamableHttpClientTransport::from_config(http_cfg);
    let running = ().serve(transport).await.map_err(|e| McpError::Transport {
        server: cfg.name.clone(),
        source: format!("handshake failed: {e}"),
    })?;
    Ok(ConnectedClient {
        running,
        stderr: None,
    })
}

/// Entry point used by `client::do_connect`. Dispatches on the transport
/// kind, runs any gating policy (SSRF), constructs the appropriate
/// rmcp transport, and returns a connected client ready for
/// `list_tools` / `call_tool` round-trips.
pub async fn build_transport_for(cfg: &McpServerConfig) -> McpResult<ConnectedClient> {
    match &cfg.transport {
        McpTransportSpec::Stdio { .. } => build_stdio_client(cfg).await,
        McpTransportSpec::StreamableHttp { url } => build_http_client(cfg, url).await,
        McpTransportSpec::Sse { url } => {
            // rmcp 1.5 retired the standalone SSE client; Streamable HTTP
            // speaks the same SSE sub-protocol on its GET channel, so we
            // route legacy `Sse` entries through that. Servers that
            // strictly require the old SSE-only transport need a rebuild
            // or a newer server version.
            crate::app_warn!(
                "mcp",
                &format!("{}:transport", cfg.name),
                "Legacy SSE transport routed through Streamable HTTP; \
                 update the server to the 2025-03-26 spec if behavior differs"
            );
            build_http_client(cfg, url).await
        }
        McpTransportSpec::WebSocket { .. } => Err(McpError::NotReady {
            server: cfg.name.clone(),
            reason: "WebSocket transport is not yet implemented in this build".into(),
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

    #[tokio::test]
    async fn websocket_still_not_implemented() {
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::WebSocket {
            url: "wss://example.com/mcp".into(),
        };
        match build_transport_for(&cfg).await {
            Err(McpError::NotReady { .. }) => {}
            Err(other) => panic!("unexpected error variant: {other:?}"),
            Ok(_) => panic!("expected NotReady for websocket"),
        }
    }

    #[tokio::test]
    async fn http_transport_honors_ssrf_policy() {
        // Untrusted + private-network URL → Blocked. This guards the
        // regression where the SSRF gate was skipped on MCP dial-out.
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::StreamableHttp {
            url: "http://127.0.0.1:9999/mcp".into(),
        };
        cfg.trust_level = McpTrustLevel::Untrusted;
        match build_transport_for(&cfg).await {
            Err(McpError::Blocked { .. }) => {}
            Err(other) => panic!("expected Blocked, got: {other:?}"),
            Ok(_) => panic!("expected Blocked for private URL under Strict policy"),
        }
    }
}
