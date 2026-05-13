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
        match args[i].as_str() {
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
            "--dangerously-skip-all-approvals" => {}
            "--version" => {
                println!("hope-agent {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => return None,
            _ => {
                eprintln!("[server] Unknown argument: {}", args[i]);
            }
        }
        i += 1;
    }
    Some((bind_addr, api_key))
}
