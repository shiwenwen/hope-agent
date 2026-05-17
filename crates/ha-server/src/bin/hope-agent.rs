//! Headless `hope-agent` binary — the entry point shipped in the official
//! Docker image. Mirrors the `hope-agent server start` argv shape from
//! [`src-tauri/src/main.rs`] so documentation and the docker entrypoint
//! script don't need to change between desktop and container builds.
//!
//! Scope:
//! - `hope-agent server start [--bind ADDR] [--api-key KEY]` — same flags
//!   as the desktop binary; runs the HTTP/WS server and blocks until exit.
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

    // Process-scoped flag — applied before subcommand dispatch so it
    // wins even if the user puts it after `server`.
    if args.iter().any(|a| a == "--dangerously-skip-all-approvals") {
        ha_core::security::dangerous::set_cli_flag(true);
        eprintln!(
            "[!] DANGEROUS MODE: all tool approvals will be skipped (CLI flag, this launch only)"
        );
    }

    // Headless auto-approve: same effect as ticking "auto approve tools" on
    // every chat the HTTP route opens — the permission engine still runs
    // (so dangerous-commands, plan-mode ask, protected paths all stay
    // enforced), just the `auto_approve_tools` switch goes through. Narrower
    // than `--dangerously-skip-all-approvals`. Env var lets Docker /
    // systemd users opt in without rewriting the entrypoint.
    let env_enabled = std::env::var(ha_server::auto_approve::ENV_VAR)
        .map(|v| ha_server::auto_approve::env_truthy(&v))
        .unwrap_or(false);
    let cli_enabled = args.iter().any(|a| a == ha_server::auto_approve::FLAG);
    if env_enabled || cli_enabled {
        ha_server::auto_approve::set_cli_flag(true);
        let source = if cli_enabled { "CLI flag" } else { "env" };
        eprintln!(
            "[!] AUTO-APPROVE MODE ({source}): HTTP chat will auto-approve every tool call \
             (engine gates still enforced; this launch only)"
        );
    }

    if args.iter().any(|a| a == "--version") {
        println!("hope-agent {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // `hope-agent server [sub] [opts...]`
    if args.len() >= 2 && args[1] == "server" {
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

fn print_top_help() {
    println!("Hope Agent — headless HTTP/WebSocket server");
    println!();
    println!("This binary ships in the official Docker image. Only the");
    println!("headless `server` subcommand is wired up; the desktop GUI,");
    println!("ACP stdio, and `auth` flows live in the Tauri-built binary.");
    println!();
    println!("Usage: hope-agent server start [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --bind, -b ADDR                   Bind address (default: 127.0.0.1:8420)");
    println!("  --api-key KEY                     Bearer token for HTTP/WS auth");
    println!(
        "  --auto-approve-tools              Auto-approve every tool call on HTTP chat (engine"
    );
    println!(
        "                                    gates still enforced; or set HA_SERVER_AUTO_APPROVE_TOOLS=1)"
    );
    println!("  --dangerously-skip-all-approvals  Skip every tool approval (this launch only)");
    println!("  --version                         Print version and exit");
    println!("  --help, -h                        Print help and exit");
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
    ha_core::set_app_version(env!("CARGO_PKG_VERSION"));
    ha_core::init_runtime("server");
    if let Err(e) = ha_core::agent_loader::ensure_default_agent() {
        eprintln!("[server] Warning: failed to ensure default agent: {e}");
    }

    // Resolve the effective API key. Precedence (highest first):
    //   1. `--api-key` CLI flag (translated from `HA_API_KEY` env by the
    //      Docker entrypoint).
    //   2. `config.server.api_key` written by the browser onboarding
    //      wizard or the Settings → Server panel.
    //   3. `None` — server accepts unauthenticated requests.
    //
    // Without #2 a user who enables auth in the browser, restarts the
    // container without re-exporting `HA_API_KEY`, gets a server that
    // silently downgrades to no-auth while the UI suggests otherwise.
    let api_key = api_key.or_else(|| {
        ha_core::config::cached_config()
            .server
            .api_key
            .clone()
            .filter(|k| !k.is_empty())
            .inspect(|_| {
                eprintln!(
                    "[server] Using API key from saved config (server.api_key); CLI / HA_API_KEY would override."
                );
            })
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
