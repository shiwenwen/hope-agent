use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};

/// Exit code for "restart requested" (e.g., after self-fix)
const EXIT_CODE_RESTART: i32 = 42;

// ── Config Persistence ────────────────────────────────────────────

/// Read `guardian.enabled` from `~/.opencomputer/config.json`. Defaults to
/// `true` when the file, the `guardian` key, or the `enabled` field is missing.
pub fn get_enabled_from_config() -> Result<bool> {
    let config_path = crate::paths::config_path()?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let config: Value = serde_json::from_str(&content).unwrap_or_default();
    Ok(config
        .get("guardian")
        .and_then(|g| g.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true))
}

/// Set `guardian.enabled` in `~/.opencomputer/config.json` via read-modify-write.
/// Other top-level fields in the file are preserved.
///
/// `guardian` is not part of `AppConfig` (it's a loose JSON field on the raw
/// config doc) so we can't route through `config::save_config`. We still take
/// an autosave snapshot directly so the toggle remains rollback-able, matching
/// the AGENTS.md "all config writes are backed up" invariant.
pub fn set_enabled_in_config(enabled: bool) -> Result<()> {
    let config_path = crate::paths::config_path()?;
    let content = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut config: Value = serde_json::from_str(&content).unwrap_or(json!({}));
    config["guardian"] = json!({ "enabled": enabled });
    let json_str = serde_json::to_string_pretty(&config)?;
    // Snapshot pre-change state so this bypass write still honors the rollback
    // contract. Labels the entry for visibility in list_settings_backups.
    {
        let _g = crate::backup::scope_save_reason("guardian", "guardian");
        crate::backup::snapshot_before_write(&config_path, "config");
    }
    std::fs::write(&config_path, json_str)?;
    Ok(())
}

// ── Configuration ─────────────────────────────────────────────────

/// Configuration for the Guardian process supervisor.
pub struct GuardianConfig {
    /// Maximum consecutive crashes before giving up entirely.
    pub max_crashes: u32,
    /// Number of crashes that triggers backup + self-diagnosis.
    pub diagnosis_threshold: u32,
    /// Time window (seconds) — if no crash occurs within this window, reset counter.
    pub crash_window_secs: u64,
    /// Backoff delays for consecutive crashes (seconds). Index is `crash_count - 1`.
    pub backoff_delays: Vec<u64>,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            max_crashes: 8,
            diagnosis_threshold: 5,
            crash_window_secs: 600,
            backoff_delays: vec![1, 3, 9, 15, 30],
        }
    }
}

// ── Guardian Loop ─────────────────────────────────────────────────

/// Run the guardian process supervisor.
///
/// Spawns the current executable with `child_args` as arguments, monitors exit codes,
/// and auto-restarts on crashes.
///
/// Exit code conventions:
/// - `0`: user quit — don't restart
/// - `42`: restart requested (self-fix) — restart immediately
/// - SIGINT/SIGTERM: signal handler sets should_exit — don't restart
/// - anything else: crash — restart with backoff
///
/// Additional environment variables passed to the child:
/// - `OPENCOMPUTER_RECOVERED=1` and `OPENCOMPUTER_CRASH_COUNT=N` on recovery restarts
///
/// This function never returns normally — it either exits the process or loops forever.
pub fn run_guardian(child_args: Vec<String>, config: GuardianConfig) -> ! {
    let should_exit = Arc::new(AtomicBool::new(false));

    // Install signal handlers to forward signals and exit cleanly
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
    let journal_path = crate::paths::crash_journal_path().ok();

    // Resolve own executable once (doesn't change during process lifetime)
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(err) => {
            eprintln!("[Guardian] Cannot resolve own executable path: {}", err);
            std::process::exit(1);
        }
    };

    loop {
        // Check if we received a termination signal
        if should_exit.load(Ordering::Relaxed) {
            eprintln!("[Guardian] Received termination signal, exiting cleanly.");
            std::process::exit(0);
        }

        // Reset crash counter if enough time has passed since last crash
        if let Some(last_time) = last_crash_time {
            if last_time.elapsed() > Duration::from_secs(config.crash_window_secs) {
                crash_count = 0;
            }
        }

        // Build the child command
        let mut cmd = Command::new(&exe);
        cmd.args(&child_args);

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
                    std::process::exit(0);
                }
                crash_count += 1;
                last_crash_time = Some(Instant::now());
                continue;
            }
        };

        // Check if we received a signal during wait
        if should_exit.load(Ordering::Relaxed) {
            std::process::exit(0);
        }

        let exit_code = exit_status.code().unwrap_or(1);

        match exit_code {
            0 => {
                // Normal user-initiated quit
                std::process::exit(0);
            }
            EXIT_CODE_RESTART => {
                // Restart requested (e.g., after self-fix)
                eprintln!(
                    "[Guardian] Restart requested (exit code {}), restarting immediately.",
                    EXIT_CODE_RESTART
                );
                crash_count = 0;
                last_crash_time = None;
                continue;
            }
            _ => {
                // Crash detected
                crash_count += 1;
                last_crash_time = Some(Instant::now());

                let signal_info = crate::crash_journal::signal_name_from_exit_code(exit_code)
                    .map(|s| format!(" ({})", s))
                    .unwrap_or_default();

                eprintln!(
                    "[Guardian] Crash detected ({}/{}): exit code {}{}",
                    crash_count, config.max_crashes, exit_code, signal_info
                );

                // Record crash to journal
                if let Some(ref path) = journal_path {
                    let mut journal = crate::crash_journal::CrashJournal::load(path);
                    journal.add_crash(exit_code, crash_count);
                    let _ = journal.save(path);
                }

                // Trigger backup + self-diagnosis at threshold
                if crash_count == config.diagnosis_threshold {
                    eprintln!(
                        "[Guardian] Crash threshold reached, running backup and self-diagnosis..."
                    );
                    run_recovery(journal_path.as_ref());
                }

                // Give up after max crashes
                if crash_count >= config.max_crashes {
                    eprintln!(
                        "[Guardian] Max crash restarts reached ({}), giving up.",
                        config.max_crashes
                    );
                    std::process::exit(1);
                }

                // Exponential backoff
                let delay_idx =
                    (crash_count as usize - 1).min(config.backoff_delays.len().saturating_sub(1));
                let delay = config.backoff_delays.get(delay_idx).copied().unwrap_or(30);
                eprintln!("[Guardian] Restarting in {} second(s)...", delay);
                std::thread::sleep(Duration::from_secs(delay));
            }
        }
    }
}

// ── Recovery ──────────────────────────────────────────────────────

/// Run backup and self-diagnosis (called when crash_count hits the diagnosis threshold).
fn run_recovery(journal_path: Option<&std::path::PathBuf>) {
    // Step 1: Backup settings
    match crate::backup::create_backup() {
        Ok(backup_path) => {
            eprintln!("[Guardian] Backup created: {}", backup_path);
            if let Some(path) = journal_path {
                let mut journal = crate::crash_journal::CrashJournal::load(path);
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
        let journal = crate::crash_journal::CrashJournal::load(path);
        match crate::self_diagnosis::diagnose(&journal) {
            Ok(result) => {
                eprintln!("[Guardian] Diagnosis complete:");
                eprintln!("  Cause: {}", result.cause);
                eprintln!("  Severity: {}", result.severity);
                for rec in &result.recommendations {
                    eprintln!("  - {}", rec);
                }

                // Step 3: Attempt auto-fix if applicable
                let fixes = crate::self_diagnosis::auto_fix(&result);
                let mut final_result = result;
                if !fixes.is_empty() {
                    eprintln!("[Guardian] Auto-fixes applied:");
                    for fix in &fixes {
                        eprintln!("  - {}", fix);
                    }
                    final_result.auto_fix_applied = fixes;
                }

                // Save diagnosis to journal
                let mut journal = crate::crash_journal::CrashJournal::load(path);
                journal.set_last_diagnosis(final_result);
                let _ = journal.save(path);
            }
            Err(e) => {
                eprintln!("[Guardian] Diagnosis failed: {}", e);
            }
        }
    }
}
