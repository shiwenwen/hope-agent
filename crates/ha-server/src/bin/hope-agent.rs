//! Headless `hope-agent` binary — the entry point shipped in the official
//! Docker image. Mirrors the `hope-agent server start` argv shape from
//! [`src-tauri/src/main.rs`] so documentation and the docker entrypoint
//! script don't need to change between desktop and container builds.
//!
//! Scope:
//! - `hope-agent server start [--bind ADDR] [--api-key KEY]` — same flags
//!   as the desktop binary; runs the HTTP/WS server and blocks until exit.
//! - `hope-agent knowledge-mcp` — stdio MCP wrapper for Knowledge Space.
//! - `--version` / `--help`.
//! - `hope-agent server {install,uninstall,status,stop,setup}` — print a
//!   pointer at the orchestrator (compose / k8s / browser onboarding) and
//!   exit non-zero. These actions belong outside the container.
//!
//! Out of scope: desktop GUI, ACP stdio, `auth` CLI flows. Those depend on
//! `app_lib` (the Tauri-side library) and stay exclusive to the desktop
//! binary in `src-tauri`.

use std::env;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = env::args().collect();

    if matches!(
        args.get(1).map(String::as_str),
        Some("--version") | Some("-V")
    ) {
        println!("hope-agent {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.len() >= 2 && args[1] == "knowledge-mcp" {
        return run_knowledge_mcp(&args[2..]);
    }

    if args.len() >= 2 && args[1] == "mcp" {
        return run_mcp(&args[2..]);
    }

    // `hope-agent server [sub] [opts...]`
    if args.len() >= 2 && args[1] == "server" {
        // Flag detection lives inside the `server` branch so plain
        // `hope-agent --help` (or anything without `server`) never prints
        // the auto-approve / dangerous-mode banner, even when the
        // matching env var is exported.
        apply_server_process_flags(&args);
        let sub = args.get(2).map(|s| s.as_str()).unwrap_or("");
        match sub {
            // No sub or explicit `start` → run the server. Flags either
            // way are forwarded straight to `parse_server_args`.
            "" => return run_server(&[]),
            "start" => return run_server(&args[3..]),
            // Sub starts with `-` → caller used `hope-agent server --bind …`
            // shorthand. Treat the whole tail as flags.
            s if s.starts_with('-') => return run_server(&args[2..]),
            "install" | "uninstall" | "status" | "stop" | "setup" => {
                print_unsupported_subcommand(sub);
                std::process::exit(1);
            }
            other => {
                eprintln!("[server] Unknown subcommand: {other}");
                print_top_help();
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_top_help();
        return;
    }

    if args.len() > 1 {
        eprintln!("[hope-agent] Unknown arguments: {:?}", &args[1..]);
        print_top_help();
        std::process::exit(1);
    }

    print_top_help();
}

/// Detect the two process-scoped permission flags
/// (`--dangerously-skip-all-approvals` / `--auto-approve-tools` +
/// `HA_SERVER_AUTO_APPROVE_TOOLS`) and emit their stderr banners. Called
/// only from inside the `server` subcommand branch so plain `--help` /
/// `--version` invocations stay quiet even if the env var is exported.
fn apply_server_process_flags(args: &[String]) {
    if args.iter().any(|a| a == "--dangerously-skip-all-approvals") {
        ha_core::security::dangerous::set_cli_flag(true);
        eprintln!(
            "[!] DANGEROUS MODE: all tool approvals will be skipped (CLI flag, this launch only)"
        );
    }

    // Headless auto-approve: same effect as ticking "auto approve tools"
    // on every IM account — sets `ChatEngineParams.auto_approve_tools=true`
    // for every chat the HTTP route opens, which bypasses ALL permission
    // gates including dangerous-commands, protected-paths, and edit-command
    // audits. `--dangerously-skip-all-approvals` is a strict superset: it
    // silences dispatcher-level `app_warn!` audit logs too. Env var lets
    // Docker / systemd users opt in without rewriting the entrypoint.
    let env_enabled = std::env::var(ha_server::auto_approve::ENV_VAR)
        .map(|v| ha_server::auto_approve::env_truthy(&v))
        .unwrap_or(false);
    let cli_enabled = args.iter().any(|a| a == ha_server::auto_approve::FLAG);
    if env_enabled || cli_enabled {
        ha_server::auto_approve::set_active(true);
        let source = if cli_enabled { "CLI flag" } else { "env" };
        eprintln!(
            "[!] AUTO-APPROVE MODE ({source}): every HTTP chat tool call auto-allowed, \
             including dangerous-commands / protected-paths (this launch only)"
        );
        // The stderr banner reaches `docker logs` / journalctl, but it
        // doesn't reach `~/.hope-agent/logs.db` — the canonical surface
        // for agent self-diagnosis. Logging here would race with
        // `init_runtime`; see `run_server` for the post-init log call.
    }
}

fn print_top_help() {
    println!("Hope Agent — headless HTTP/WebSocket server");
    println!();
    println!("This binary ships in the official Docker image. Only the headless");
    println!("`server`, `knowledge-mcp` and `mcp` subcommands are wired up; the");
    println!("desktop GUI, ACP stdio, and `auth` flows live in the Tauri-built binary.");
    println!();
    println!("Usage:");
    println!("  hope-agent server start [OPTIONS]");
    println!("  hope-agent knowledge-mcp [OPTIONS]");
    println!("  hope-agent mcp [OPTIONS]");
    println!();
    println!("Server options:");
    println!("  --bind, -b ADDR                   Bind address (default: 127.0.0.1:8420)");
    println!("  --api-key KEY                     Bearer token for HTTP/WS auth");
    println!(
        "  --auto-approve-tools              Auto-approve every tool call on HTTP chat — including"
    );
    println!("                                    dangerous-commands / protected-paths (or set");
    println!("                                    HA_SERVER_AUTO_APPROVE_TOOLS=1)");
    println!("  --dangerously-skip-all-approvals  Skip every tool approval (this launch only)");
    println!("  --version                         Print version and exit");
    println!("  --help, -h                        Print help and exit");
}

fn run_knowledge_mcp(args: &[String]) {
    let Some(options) = parse_knowledge_mcp_args(args) else {
        print_knowledge_mcp_help();
        return;
    };

    if let Err(e) = ha_core::paths::ensure_dirs() {
        eprintln!("[knowledge-mcp] Failed to initialize data directories: {e}");
        std::process::exit(1);
    }
    ha_core::set_app_version(env!("CARGO_PKG_VERSION"));
    ha_core::init_runtime("knowledge-mcp");

    if let Err(e) = ha_core::knowledge::agent_mcp::run_stdio(options) {
        eprintln!("[knowledge-mcp] Server error: {e}");
        std::process::exit(1);
    }
}

fn run_mcp(args: &[String]) {
    let mut options = ha_core::mcp_server::McpServerOptions::default();
    for arg in args {
        match arg.as_str() {
            "--allow-writes" => options.allow_writes = true,
            "--version" => {
                println!("hope-agent-mcp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("Hope Agent MCP Server (platform) — Design Space over stdio MCP.");
                println!();
                println!("Usage: hope-agent mcp [--allow-writes]");
                println!("  --allow-writes  Expose write tools (generate / edit / comment); default read-only");
                println!("(Knowledge Space tools remain under `hope-agent knowledge-mcp`.)");
                return;
            }
            other => {
                eprintln!("[mcp] Unknown argument: {other}");
                return;
            }
        }
    }
    if let Err(e) = ha_core::paths::ensure_dirs() {
        eprintln!("[mcp] Failed to initialize data directories: {e}");
        std::process::exit(1);
    }
    ha_core::set_app_version(env!("CARGO_PKG_VERSION"));
    ha_core::init_runtime("mcp");

    let providers: Vec<Box<dyn ha_core::mcp_server::ToolProvider>> =
        vec![Box::new(ha_core::design::mcp_provider::DesignToolProvider)];
    if let Err(e) = ha_core::mcp_server::run_stdio(options, providers) {
        eprintln!("[mcp] Server error: {e}");
        std::process::exit(1);
    }
}

fn parse_knowledge_mcp_args(
    args: &[String],
) -> Option<ha_core::knowledge::agent_mcp::KnowledgeMcpOptions> {
    let mut options = ha_core::knowledge::agent_mcp::KnowledgeMcpOptions::default();
    for arg in args {
        match arg.as_str() {
            "--allow-proposals" => options.allow_proposals = true,
            "--version" => {
                println!("hope-agent-knowledge-mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => return None,
            other => {
                eprintln!("[knowledge-mcp] Unknown argument: {other}");
                return None;
            }
        }
    }
    Some(options)
}

fn print_knowledge_mcp_help() {
    println!("Hope Agent Knowledge MCP Server");
    println!();
    println!("Usage: hope-agent knowledge-mcp [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --allow-proposals  Also expose knowledge_compile_propose (review proposal only)");
    println!("  --version          Print version and exit");
    println!("  --help, -h         Print help and exit");
}

fn print_unsupported_subcommand(sub: &str) {
    eprintln!("`hope-agent server {sub}` is not supported in this build.");
    match sub {
        "install" | "uninstall" | "status" | "stop" => eprintln!(
            "  Service lifecycle belongs to your orchestrator. Use `docker compose up/down/logs`, your kubernetes manifest, or whatever supervisor wraps the container."
        ),
        "setup" => eprintln!(
            "  Use the browser onboarding wizard at the server's bind address (http://<bind>/) on first launch."
        ),
        _ => {}
    }
}

fn run_server(args: &[String]) {
    let Some((bind_addr, api_key)) = parse_server_args(args) else {
        print_top_help();
        return;
    };
    let model_eval_mode = ha_core::eval_context::model_eval_mode_enabled();
    if model_eval_mode {
        if let Err(error) = ha_core::eval_context::install_supervisor_watchdog_from_env() {
            eprintln!("[model-eval] failed to install Supervisor watchdog: {error}");
            std::process::exit(2);
        }
        if let Err(error) = ha_core::platform::prevent_process_dumping() {
            eprintln!("[model-eval] failed to harden Provider secret process: {error}");
            std::process::exit(2);
        }
    }
    let model_eval_server_token = if model_eval_mode {
        if api_key.is_some() {
            eprintln!(
                "[model-eval] --api-key is forbidden; pass HA_MODEL_EVAL_SERVER_TOKEN so it can be consumed before tool processes start"
            );
            std::process::exit(2);
        }
        let token = std::env::var("HA_MODEL_EVAL_SERVER_TOKEN")
            .unwrap_or_default()
            .trim()
            .to_string();
        std::env::remove_var("HA_MODEL_EVAL_SERVER_TOKEN");
        if token.len() < 24 || token.len() > 4_096 {
            eprintln!("[model-eval] HA_MODEL_EVAL_SERVER_TOKEN is missing or invalid");
            std::process::exit(2);
        }
        Some(token)
    } else {
        None
    };

    eprintln!(
        "[server] Starting Hope Agent server v{}",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("[server] Bind address: {bind_addr}");

    if let Err(e) = ha_core::paths::ensure_dirs() {
        eprintln!("[server] Failed to initialize data directories: {e}");
        std::process::exit(1);
    }

    // Browser onboarding handles the same flow as the desktop TTY wizard;
    // the TTY wizard implementation lives in `src-tauri/src/lib.rs`
    // (`app_lib::cli_onboarding`) so we skip it here. The banner points
    // the operator at the bind address with a clear next step.
    match ha_core::onboarding::state::get_state() {
        Ok(state) if state.completed_version < ha_core::onboarding::CURRENT_ONBOARDING_VERSION => {
            ha_server::banner::print_unconfigured_notice(&bind_addr);
        }
        Err(e) => {
            eprintln!("[server] Warning: failed to read onboarding state: {e}");
        }
        _ => {}
    }

    // Same init order as src-tauri/src/main.rs::run_server: set_app_version
    // and init_runtime("server") MUST run before ensure_default_agent —
    // the legacy "default" → "ha-main" agent-id rename inside init_runtime
    // would otherwise race with the pre-create and orphan user data.
    //
    // In isolated model-eval mode this eager read also consumes the protected
    // Provider secret overlay before init_runtime snapshots the login-shell
    // environment or starts any background worker. Agent/tool subprocesses
    // therefore cannot inherit the one-shot secret environment variable.
    let model_eval_codex_token = if model_eval_mode {
        match ha_core::config::initialize_model_eval_provider_secrets() {
            Ok(token) => token,
            Err(error) => {
                eprintln!("[model-eval] failed to initialize isolated Provider config: {error:#}");
                std::process::exit(2);
            }
        }
    } else {
        let _ = ha_core::config::cached_config();
        None
    };
    ha_core::set_app_version(env!("CARGO_PKG_VERSION"));
    ha_core::init_runtime("server");
    if let Some(codex_token) = model_eval_codex_token {
        let cache = match ha_core::require_codex_token_cache() {
            Ok(cache) => cache,
            Err(error) => {
                eprintln!("[model-eval] Codex token cache is unavailable: {error:#}");
                std::process::exit(2);
            }
        };
        *cache.blocking_lock() = Some((codex_token.access_token, codex_token.account_id));
    }
    if let Err(e) = ha_core::agent_loader::ensure_default_agent() {
        eprintln!("[server] Warning: failed to ensure default agent: {e}");
    }

    // Mirror the startup banner into logs.db so agent self-diagnosis (and
    // any operator grepping `logs.db` after the fact) can see that
    // auto-approve was active for this launch. The stderr banner above
    // already reached docker / journalctl; this one persists into the
    // application log surface.
    if ha_server::auto_approve::is_active() {
        ha_core::app_warn!(
            "permission",
            "server_startup",
            "HTTP auto-approve mode active for this launch — every chat tool call auto-allowed, including dangerous-commands / protected-paths / edit-command audits (equivalent to an IM account with auto_approve_tools=true)"
        );
    }

    // Resolve the effective API key. Precedence (highest first):
    //   1. One-shot `HA_MODEL_EVAL_SERVER_TOKEN` in isolated evaluation mode;
    //      it has already been removed from the server environment.
    //   2. `--api-key` CLI flag (translated from `HA_API_KEY` env by the
    //      Docker entrypoint).
    //   3. `config.server.api_key` written by the browser onboarding
    //      wizard or the Settings → Server panel.
    //   4. `None` — server accepts unauthenticated requests.
    //
    // Without #2 a user who enables auth in the browser, restarts the
    // container without re-exporting `HA_API_KEY`, gets a server that
    // silently downgrades to no-auth while the UI suggests otherwise.
    let saved_server_config = ha_core::config::cached_config().server.clone();
    let api_key = model_eval_server_token.or(api_key).or_else(|| {
        saved_server_config
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .inspect(|_| {
                eprintln!(
                    "[server] Using API key from saved config (server.api_key); CLI / HA_API_KEY would override."
                );
            })
    });
    let knowledge_agent_read_token = env::var("HA_KNOWLEDGE_AGENT_READ_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            saved_server_config
                .knowledge_agent_read_token
                .clone()
                .filter(|k| !k.is_empty())
        });

    let session_db = ha_core::require_session_db()
        .expect("init_runtime contract")
        .clone();
    let project_db = ha_core::require_project_db()
        .expect("init_runtime contract")
        .clone();
    let event_bus = ha_core::get_event_bus()
        .expect("init_runtime contract")
        .clone();

    let ctx = Arc::new(ha_server::AppContext {
        session_db,
        project_db,
        event_bus,
        chat_cancels: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        api_key: api_key.clone(),
    });
    let config = ha_server::ServerConfig {
        bind_addr,
        api_key,
        knowledge_agent_read_token,
        cors_origins: Vec::new(),
    };

    // Write PID file. The Docker entrypoint clears any stale file from a
    // SIGKILL'd previous container before invoking the binary, so the
    // freshly-created PID is always trustworthy.
    let pid_path = ha_core::paths::root_dir()
        .map(|d| d.join("server.pid"))
        .ok();
    if let Some(ref p) = pid_path {
        let _ = std::fs::write(p, std::process::id().to_string());
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        tokio::spawn(ha_core::start_background_tasks());
        ha_core::crash_flush::install_signal_handlers();
        if let Err(e) = ha_server::start_server(config, ctx).await {
            eprintln!("[server] Server error: {e}");
            std::process::exit(1);
        }
    });

    if let Some(ref p) = pid_path {
        let _ = std::fs::remove_file(p);
    }
}

/// Argv parsing for `server start` flags. `None` means `--help` was
/// requested; the caller prints help and returns.
fn parse_server_args(args: &[String]) -> Option<(String, Option<String>)> {
    let mut bind_addr = "127.0.0.1:8420".to_string();
    let mut api_key: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--bind" | "-b" => {
                i += 1;
                if i < args.len() {
                    bind_addr = args[i].clone();
                }
            }
            "--api-key" => {
                i += 1;
                if i < args.len() {
                    api_key = Some(args[i].clone());
                }
            }
            // Also honor `--bind=ADDR` / `--api-key=VALUE` so a user
            // dropping into `docker run ... hope-agent server start
            // --api-key=KEY` doesn't fall into the unknown-arg branch
            // (which would echo the full token to stderr → docker logs).
            s if s.starts_with("--bind=") => {
                bind_addr = s["--bind=".len()..].to_string();
            }
            s if s.starts_with("--api-key=") => {
                api_key = Some(s["--api-key=".len()..].to_string());
            }
            "--dangerously-skip-all-approvals" => {}
            // Already consumed in `main()`; ignore here so it doesn't fall
            // through to the unknown-arg branch and stripe the help text.
            s if s == ha_server::auto_approve::FLAG => {}
            "--version" => {
                println!("hope-agent {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => return None,
            _ => {
                // Last-resort redaction: anything that even *looks*
                // like a secret-bearing arg is logged with the value
                // stripped, so a misspelled `--apikey=...` / typo in a
                // future flag can't leak the token via stderr.
                eprintln!("[server] Unknown argument: {}", redact_arg_for_log(arg));
            }
        }
        i += 1;
    }
    Some((bind_addr, api_key))
}

/// Mask the value portion of any `--…key…=value` / `--token=value` /
/// `--secret=value` style argument before it hits stderr. Plain flags
/// without `=` are returned unchanged.
fn redact_arg_for_log(arg: &str) -> String {
    if let Some((flag, _value)) = arg.split_once('=') {
        let lower = flag.to_ascii_lowercase();
        if lower.contains("key")
            || lower.contains("token")
            || lower.contains("secret")
            || lower.contains("pass")
        {
            return format!("{}=[REDACTED]", flag);
        }
    }
    arg.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_value_after_secret_flag() {
        assert_eq!(
            redact_arg_for_log("--api-key=abc123"),
            "--api-key=[REDACTED]"
        );
        assert_eq!(redact_arg_for_log("--apikey=xxx"), "--apikey=[REDACTED]");
        assert_eq!(
            redact_arg_for_log("--auth-token=yyy"),
            "--auth-token=[REDACTED]"
        );
        assert_eq!(
            redact_arg_for_log("--Some-Secret=zzz"),
            "--Some-Secret=[REDACTED]"
        );
        assert_eq!(redact_arg_for_log("--password=p"), "--password=[REDACTED]");
    }

    #[test]
    fn redact_passes_through_non_secret_args() {
        assert_eq!(
            redact_arg_for_log("--bind=0.0.0.0:8420"),
            "--bind=0.0.0.0:8420"
        );
        assert_eq!(redact_arg_for_log("--unknown-flag"), "--unknown-flag");
        assert_eq!(redact_arg_for_log("plain"), "plain");
    }
}
