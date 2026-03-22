// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Exit code for "restart requested" (e.g., after self-fix)
const EXIT_CODE_RESTART: i32 = 42;
/// Maximum consecutive crash restarts in child mode (panic recovery)
const MAX_CHILD_PANICS: u32 = 3;
/// Maximum consecutive crashes before triggering backup + self-diagnosis
const DIAGNOSIS_THRESHOLD: u32 = 5;
/// Maximum consecutive crashes before giving up entirely
const MAX_GUARDIAN_CRASHES: u32 = 8;
/// Time window (seconds) — if no crash occurs within this window, reset counter
const CRASH_WINDOW_SECS: u64 = 600; // 10 minutes

/// Backoff delays for consecutive crashes (seconds)
const BACKOFF_DELAYS: [u64; 5] = [1, 3, 9, 15, 30];

fn main() {
    if env::var("OPENCOMPUTER_CHILD").is_ok() {
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
    let should_exit = Arc::new(AtomicBool::new(false));

    // Install signal handlers to forward signals to child and exit cleanly
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGINT, SIGTERM};
        let exit_flag = should_exit.clone();
        let _ = signal_hook::flag::register(SIGTERM, exit_flag.clone());
        let _ = signal_hook::flag::register(SIGINT, exit_flag);
    }

    let mut crash_count: u32 = 0;
    let mut last_crash_time: Option<Instant> = None;

    // Resolve crash journal path (best-effort, don't fail if home dir is unavailable)
    let journal_path = resolve_journal_path();

    loop {
        // Check if we received a termination signal
        if should_exit.load(Ordering::Relaxed) {
            eprintln!("[Guardian] Received termination signal, exiting cleanly.");
            break;
        }

        // Reset crash counter if enough time has passed since last crash
        if let Some(last_time) = last_crash_time {
            if last_time.elapsed() > Duration::from_secs(CRASH_WINDOW_SECS) {
                crash_count = 0;
            }
        }

        // Spawn the child process
        let exe = match env::current_exe() {
            Ok(e) => e,
            Err(err) => {
                eprintln!("[Guardian] Cannot resolve own executable path: {}", err);
                std::process::exit(1);
            }
        };

        let mut cmd = Command::new(&exe);
        cmd.env("OPENCOMPUTER_CHILD", "1");

        // Pass recovery info to child if this is a crash recovery
        if crash_count > 0 {
            cmd.env("OPENCOMPUTER_RECOVERED", "1");
            cmd.env("OPENCOMPUTER_CRASH_COUNT", crash_count.to_string());
        }

        let child_result = cmd.spawn();
        let mut child = match child_result {
            Ok(c) => c,
            Err(err) => {
                eprintln!("[Guardian] Failed to spawn child process: {}", err);
                std::process::exit(1);
            }
        };

        // Wait for child to exit
        let exit_status: ExitStatus = match child.wait() {
            Ok(s) => s,
            Err(err) => {
                eprintln!("[Guardian] Failed to wait for child: {}", err);
                // If we received a signal while waiting, exit cleanly
                if should_exit.load(Ordering::Relaxed) {
                    break;
                }
                crash_count += 1;
                last_crash_time = Some(Instant::now());
                continue;
            }
        };

        // Check if we received a signal during wait
        if should_exit.load(Ordering::Relaxed) {
            break;
        }

        let exit_code = exit_status.code().unwrap_or(1);

        match exit_code {
            0 => {
                // Normal user-initiated quit
                break;
            }
            EXIT_CODE_RESTART => {
                // Restart requested (e.g., after self-fix)
                eprintln!("[Guardian] Restart requested (exit code {}), restarting immediately.", EXIT_CODE_RESTART);
                crash_count = 0;
                last_crash_time = None;
                continue;
            }
            _ => {
                // Crash detected
                crash_count += 1;
                last_crash_time = Some(Instant::now());

                let signal_info = app_lib::crash_journal::signal_name_from_exit_code(exit_code)
                    .map(|s| format!(" ({})", s))
                    .unwrap_or_default();

                eprintln!(
                    "[Guardian] Crash detected ({}/{}): exit code {}{}",
                    crash_count, MAX_GUARDIAN_CRASHES, exit_code, signal_info
                );

                // Record crash to journal
                if let Some(ref path) = journal_path {
                    let mut journal = app_lib::crash_journal::CrashJournal::load(path);
                    journal.add_crash(exit_code, crash_count);
                    let _ = journal.save(path);
                }

                // Trigger backup + self-diagnosis at threshold
                if crash_count == DIAGNOSIS_THRESHOLD {
                    eprintln!("[Guardian] Crash threshold reached, running backup and self-diagnosis...");
                    run_recovery(journal_path.as_ref());
                }

                // Give up after max crashes
                if crash_count >= MAX_GUARDIAN_CRASHES {
                    eprintln!(
                        "[Guardian] Max crash restarts reached ({}), giving up.",
                        MAX_GUARDIAN_CRASHES
                    );
                    std::process::exit(1);
                }

                // Exponential backoff
                let delay_idx = (crash_count as usize - 1).min(BACKOFF_DELAYS.len() - 1);
                let delay = BACKOFF_DELAYS[delay_idx];
                eprintln!("[Guardian] Restarting in {} second(s)...", delay);
                std::thread::sleep(Duration::from_secs(delay));
            }
        }
    }
}

/// Run backup and self-diagnosis
fn run_recovery(journal_path: Option<&std::path::PathBuf>) {
    // Step 1: Backup settings
    match app_lib::backup::create_backup() {
        Ok(backup_path) => {
            eprintln!("[Guardian] Backup created: {}", backup_path);
            if let Some(path) = journal_path {
                let mut journal = app_lib::crash_journal::CrashJournal::load(path);
                journal.set_last_backup(chrono::Utc::now().to_rfc3339());
                let _ = journal.save(path);
            }
        }
        Err(e) => {
            eprintln!("[Guardian] Backup failed: {}", e);
        }
    }

    // Step 2: Self-diagnosis via LLM
    if let Some(path) = journal_path {
        let journal = app_lib::crash_journal::CrashJournal::load(path);
        match app_lib::self_diagnosis::diagnose(&journal) {
            Ok(result) => {
                eprintln!("[Guardian] Diagnosis complete:");
                eprintln!("  Cause: {}", result.cause);
                eprintln!("  Severity: {}", result.severity);
                for rec in &result.recommendations {
                    eprintln!("  - {}", rec);
                }

                // Step 3: Attempt auto-fix if applicable
                let fixes = app_lib::self_diagnosis::auto_fix(&result);
                let mut final_result = result;
                if !fixes.is_empty() {
                    eprintln!("[Guardian] Auto-fixes applied:");
                    for fix in &fixes {
                        eprintln!("  - {}", fix);
                    }
                    final_result.auto_fix_applied = fixes;
                }

                // Save diagnosis to journal
                let mut journal = app_lib::crash_journal::CrashJournal::load(path);
                journal.set_last_diagnosis(final_result);
                let _ = journal.save(path);
            }
            Err(e) => {
                eprintln!("[Guardian] Diagnosis failed: {}", e);
            }
        }
    }
}

/// Resolve crash journal path (best-effort)
fn resolve_journal_path() -> Option<std::path::PathBuf> {
    app_lib::paths::crash_journal_path().ok()
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
