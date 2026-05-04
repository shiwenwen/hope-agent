// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::sync::Arc;
use std::time::Duration;

/// Maximum consecutive crash restarts in child mode (panic recovery)
const MAX_CHILD_PANICS: u32 = 3;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Dangerous mode: --dangerously-skip-all-approvals (top-level, process-scoped,
    // NOT persisted). Skips every tool-level approval gate for THIS launch only.
    // Applied before subcommand dispatch so GUI, server, and ACP modes all see it.
    if args.iter().any(|a| a == "--dangerously-skip-all-approvals") {
        ha_core::security::dangerous::set_cli_flag(true);
        eprintln!(
            "[!] DANGEROUS MODE: all tool approvals will be skipped (CLI flag, this launch only)"
        );
    }

    // ACP subcommand: `hope-agent acp` — runs the ACP stdio server
    if args.len() >= 2 && args[1] == "acp" {
        run_acp_server(&args[2..]);
        return;
    }

    // Server subcommand: `hope-agent server` — runs the HTTP/WS server (no GUI)
    if args.len() >= 2 && args[1] == "server" {
        run_server(&args[2..]);
        return;
    }

    // Auth subcommand: `hope-agent auth codex ...` — terminal-only auth flows
    if args.len() >= 2 && args[1] == "auth" {
        app_lib::cli_auth::run(&args[2..]);
        return;
    }

    // Child mode: spawned by Guardian via --child-mode arg or legacy HOPE_AGENT_CHILD env
    if (args.len() >= 2 && args[1] == "--child-mode") || env::var("HOPE_AGENT_CHILD").is_ok() {
        run_child();
    } else if cfg!(debug_assertions) {
        // Dev mode — skip guardian, run app directly
        run_child();
    } else if is_guardian_enabled() {
        run_guardian();
    } else {
        // Guardian disabled by user — run app directly
        run_child();
    }
}

/// Check if the guardian (self-healing) feature is enabled in config.json.
/// Defaults to true if config is missing or unreadable.
fn is_guardian_enabled() -> bool {
    let config_path = match app_lib::paths::config_path() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return true,
    };
    // config.guardian.enabled — defaults to true
    config
        .get("guardian")
        .and_then(|g| g.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

// ── Guardian Mode ──────────────────────────────────────────────────

fn run_guardian() {
    // Desktop guardian: spawn child with HOPE_AGENT_CHILD env var
    ha_core::guardian::run_guardian(
        vec!["--child-mode".to_string()],
        ha_core::guardian::GuardianConfig::default(),
    );
}

// ── Child Mode ─────────────────────────────────────────────────────

fn run_child() {
    let mut crash_count: u32 = 0;

    loop {
        let result = std::panic::catch_unwind(|| {
            app_lib::run();
        });

        match result {
            Ok(_) => {
                // Normal exit (user closed window / quit)
                std::process::exit(0);
            }
            Err(panic_info) => {
                crash_count += 1;
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                eprintln!(
                    "[Child] Panic detected ({}/{}): {}",
                    crash_count, MAX_CHILD_PANICS, msg
                );

                if crash_count >= MAX_CHILD_PANICS {
                    eprintln!(
                        "[Child] Max panic restarts reached ({}), exiting with error.",
                        MAX_CHILD_PANICS
                    );
                    std::process::exit(1);
                }

                // Brief delay before restart to avoid tight crash loops
                std::thread::sleep(Duration::from_secs(1));
                eprintln!("[Child] Restarting after panic...");
            }
        }
    }
}

// ── ACP Server Mode ────────────────────────────────────────────────

fn run_acp_server(args: &[String]) {
    let mut verbose = false;
    let mut agent_id = "default".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--verbose" | "-v" => verbose = true,
            "--agent-id" | "-a" => {
                i += 1;
                if i < args.len() {
                    agent_id = args[i].clone();
                }
            }
            // Already handled at top-level main() — consume silently here.
            "--dangerously-skip-all-approvals" => {}
            "--version" => {
                println!("hope-agent-acp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                println!("Hope Agent ACP Server");
                println!();
                println!("Usage: hope-agent acp [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --verbose, -v                     Enable verbose logging to stderr");
                println!(
                    "  --agent-id, -a ID                 Use specific agent (default: \"default\")"
                );
                println!(
                    "  --dangerously-skip-all-approvals  Skip ALL tool approvals (DANGEROUS, this launch only)"
                );
                println!("  --version                         Print version and exit");
                println!("  --help, -h                        Print help and exit");
                return;
            }
            _ => {
                eprintln!("[acp] Unknown argument: {}", args[i]);
            }
        }
        i += 1;
    }

    if verbose {
        eprintln!(
            "[acp] Starting Hope Agent ACP server v{}",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!("[acp] Agent ID: {}", agent_id);
        eprintln!("[acp] Protocol: NDJSON over stdio");
    }

    // Hard-fail early if onboarding hasn't happened yet. ACP stdio IS the
    // protocol channel — we can't prompt here, so direct the user to the
    // Web GUI / desktop / `server setup` instead of silently running with
    // no provider and producing opaque failures later.
    match ha_core::onboarding::state::get_state() {
        Ok(s) if s.completed_version < ha_core::onboarding::CURRENT_ONBOARDING_VERSION => {
            eprintln!("ERROR: Hope Agent is not configured yet.");
            eprintln!("       Run 'hope-agent server setup' interactively,");
            eprintln!("       or launch the desktop app to finish first-run setup.");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("[acp] Warning: failed to read onboarding state: {}", e);
        }
        _ => {}
    }

    // Initialize core runtime: opens every DB, sets every OnceLock,
    // registers channel plugins, and brings up the ACP control plane.
    // ACP needs this because its tool loop hits memory / cron / subagent
    // / cached-agent / logger paths that all depend on these singletons.
    ha_core::init_runtime("acp");

    let session_db = ha_core::require_session_db()
        .expect("init_runtime contract")
        .clone();

    // Side-channel tokio runtime for the minimal background-task set:
    //   - IM channel approval / ask_user listeners (idempotent if no bus
    //     subscriber)
    //   - one-shot ask_user purge + async_jobs replay
    //   - MCP `init_global` (so MCP-namespaced tools resolve)
    //
    // Intentionally skips daily timers, channel auto-start, dreaming,
    // cron, and the MCP watchdog — see `start_minimal_background_tasks`
    // for the rationale.
    //
    // The ACP main loop itself stays on this thread (synchronous stdin
    // reader; each `session/prompt` builds its own current-thread runtime
    // internally). Sharing one runtime is awkward because of nested
    // `block_on`s, so we keep them strictly separate: bg_rt drops when
    // `run` returns and cancels the listeners cleanly.
    let bg_rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("acp-bg")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!(
                "[acp] Fatal: failed to build background tokio runtime: {}",
                e
            );
            std::process::exit(1);
        }
    };
    bg_rt.spawn(ha_core::start_minimal_background_tasks());

    // Crash-flush signal handlers must run inside a tokio runtime; bg_rt
    // is the only one this mode owns. Drop happens on process exit, but
    // the handler `std::process::exit(0)` runs before the bg_rt teardown
    // path so this is safe.
    bg_rt.spawn(async {
        ha_core::crash_flush::install_signal_handlers();
    });

    // Run the ACP server (blocks on stdin)
    let result = app_lib::acp::server::start(session_db, agent_id, verbose);

    // Tear bg_rt down before exit so its tasks see cancellation.
    drop(bg_rt);

    if let Err(e) = result {
        eprintln!("[acp] Server error: {}", e);
        std::process::exit(1);
    }
}

// ── HTTP/WS Server Mode ───────────────────────────────────────────

fn run_server(args: &[String]) {
    // Handle service sub-subcommands first
    if let Some(subcmd) = args.first().map(|s| s.as_str()) {
        match subcmd {
            "install" => {
                return run_server_install(&args[1..]);
            }
            "uninstall" => {
                match ha_core::service_install::uninstall_service() {
                    Ok(()) => println!("Service uninstalled successfully."),
                    Err(e) => {
                        eprintln!("Failed to uninstall service: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            "status" => {
                match ha_core::service_install::service_status() {
                    Ok(status) => println!("{}", status),
                    Err(e) => {
                        eprintln!("Failed to query service status: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            "stop" => {
                match ha_core::service_install::stop_server() {
                    Ok(()) => println!("Server stopped."),
                    Err(e) => {
                        eprintln!("Failed to stop server: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            "setup" => {
                return run_server_setup(&args[1..]);
            }
            _ => {} // Fall through to normal arg parsing
        }
    }

    let Some((bind_addr, api_key)) = parse_server_args(args, "server") else {
        println!("Hope Agent HTTP/WebSocket Server");
        println!();
        println!("Usage: hope-agent server [COMMAND] [OPTIONS]");
        println!();
        println!("Commands:");
        println!(
            "  install                           Install as a system service (launchd/systemd)"
        );
        println!("  uninstall                         Uninstall the system service");
        println!("  status                            Show service status");
        println!("  stop                              Stop the running server");
        println!("  setup [--reset]                   Run the interactive first-run wizard");
        println!();
        println!("Options:");
        println!("  --bind, -b ADDR                   Bind address (default: 127.0.0.1:8420)");
        println!("  --api-key KEY                     API key for authentication");
        println!("  --dangerously-skip-all-approvals  Skip ALL tool approvals (DANGEROUS, this launch only)");
        println!("  --version                         Print version and exit");
        println!("  --help, -h                        Print help and exit");
        return;
    };

    eprintln!(
        "[server] Starting Hope Agent server v{}",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("[server] Bind address: {}", bind_addr);

    // Initialize core subsystems
    if let Err(e) = ha_core::paths::ensure_dirs() {
        eprintln!("[server] Failed to initialize data directories: {}", e);
        std::process::exit(1);
    }

    // Onboarding status: when the wizard hasn't run yet, walk the user
    // through it interactively on a real TTY. Headless launches (systemd,
    // Docker, piped stdin) fall back to printing a notice that points at
    // the Web GUI — the service still starts with defaults so ops-style
    // deployments aren't blocked on human input.
    match ha_core::onboarding::state::get_state() {
        Ok(state) if state.completed_version < ha_core::onboarding::CURRENT_ONBOARDING_VERSION => {
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                if let Err(e) = app_lib::cli_onboarding::run_wizard() {
                    eprintln!("[server] Wizard aborted: {}. Continuing with defaults.", e);
                }
            } else {
                ha_server::banner::print_unconfigured_notice(&bind_addr);
            }
        }
        Err(e) => {
            eprintln!("[server] Warning: failed to read onboarding state: {}", e);
        }
        _ => {}
    }
    if let Err(e) = ha_core::agent_loader::ensure_default_agent() {
        eprintln!("[server] Warning: failed to ensure default agent: {}", e);
    }

    // Initialize core runtime: opens every DB, sets every OnceLock,
    // bootstraps a default EventBus, registers channel plugins, and brings
    // up the ACP control plane. Server mode wants the same singleton set
    // as desktop — only the GUI-specific pieces (Tauri webview, embedded
    // HTTP server, EventBus → frontend bridge) differ.
    ha_core::init_runtime("server");

    let session_db = ha_core::require_session_db()
        .expect("init_runtime contract")
        .clone();
    let project_db = ha_core::require_project_db()
        .expect("init_runtime contract")
        .clone();
    let event_bus = ha_core::get_event_bus()
        .expect("init_runtime contract")
        .clone();

    // Build server context
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

    // Write PID file
    let pid_path = ha_core::paths::root_dir()
        .map(|d| d.join("server.pid"))
        .ok();
    if let Some(ref p) = pid_path {
        let _ = std::fs::write(p, std::process::id().to_string());
    }

    // Run the tokio runtime. Inside it: kick off the long-running
    // background-task set (channel listeners, cron scheduler, channel
    // auto-start, dreaming, MCP + watchdog, retention loops, …) before
    // serving HTTP. Server mode is the daemon equivalent of desktop —
    // it gets the full set, not the ACP-shaped minimal one.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        tokio::spawn(ha_core::start_background_tasks());
        ha_core::crash_flush::install_signal_handlers();
        if let Err(e) = ha_server::start_server(config, ctx).await {
            eprintln!("[server] Server error: {}", e);
            std::process::exit(1);
        }
    });

    // Clean up PID file
    if let Some(ref p) = pid_path {
        let _ = std::fs::remove_file(p);
    }
}

/// Shared server arg parser for --bind and --api-key.
/// Returns None if --help was requested (already printed).
fn parse_server_args(args: &[String], context: &str) -> Option<(String, Option<String>)> {
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
            // Already handled at top-level main() — consume silently here.
            "--dangerously-skip-all-approvals" => {}
            "--version" => {
                println!("hope-agent-server {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => return None,
            _ => {
                eprintln!("[{}] Unknown argument: {}", context, args[i]);
            }
        }
        i += 1;
    }
    Some((bind_addr, api_key))
}

/// Handle `hope-agent server install [--bind ADDR] [--api-key KEY]`
fn run_server_install(args: &[String]) {
    let Some((bind_addr, api_key)) = parse_server_args(args, "server install") else {
        println!("Install Hope Agent server as a system service");
        println!();
        println!("Usage: hope-agent server install [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --bind, -b ADDR                   Bind address (default: 127.0.0.1:8420)");
        println!("  --api-key KEY                     API key for authentication");
        println!("  --dangerously-skip-all-approvals  Skip ALL tool approvals (DANGEROUS, this launch only)");
        println!("  --help, -h                        Print help and exit");
        return;
    };

    match ha_core::service_install::install_service(&bind_addr, api_key.as_deref()) {
        Ok(msg) => println!("{}", msg),
        Err(e) => {
            eprintln!("Failed to install service: {}", e);
            std::process::exit(1);
        }
    }
}

/// Handle `hope-agent server setup [--reset]` — runs the interactive
/// first-run wizard without starting the HTTP server afterwards. Useful
/// for admins preparing a fresh install before flipping on `server start`.
fn run_server_setup(args: &[String]) {
    let mut reset = false;
    for a in args {
        match a.as_str() {
            "--reset" => reset = true,
            "--help" | "-h" => {
                println!("Run the first-run onboarding wizard interactively.");
                println!();
                println!("Usage: hope-agent server setup [--reset]");
                println!();
                println!("Options:");
                println!("  --reset    Clear the existing onboarding state first");
                println!("             (providers / user config are NOT deleted).");
                return;
            }
            _ => {
                eprintln!("[server setup] Unknown argument: {}", a);
            }
        }
    }

    if let Err(e) = ha_core::paths::ensure_dirs() {
        eprintln!(
            "[server setup] Failed to initialize data directories: {}",
            e
        );
        std::process::exit(1);
    }

    if reset {
        if let Err(e) = ha_core::onboarding::state::reset() {
            eprintln!("[server setup] Failed to reset onboarding state: {}", e);
            std::process::exit(1);
        }
        println!("  Onboarding state cleared — running the wizard again.");
    }

    if let Err(e) = app_lib::cli_onboarding::run_wizard() {
        eprintln!("[server setup] Wizard failed: {}", e);
        std::process::exit(1);
    }
}
