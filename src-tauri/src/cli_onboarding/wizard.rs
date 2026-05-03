//! Top-level CLI wizard orchestrator — called from `run_server` on a TTY
//! and from the dedicated `hope-agent server setup` subcommand.
//!
//! Each step is a separate module under `steps/`. On success, the wizard
//! marks the onboarding complete so the next launch skips the flow.
//! Interrupting with Ctrl+C leaves the draft untouched — the Rust-side
//! state store only persists at step boundaries, so a partial run still
//! stores useful progress.
//!
//! Step order mirrors the GUI wizard (see `src/components/onboarding/
//! types.ts::ONBOARDING_STEPS`): language → import-openclaw → mode →
//! [if local: provider → profile → personality → safety → skills →
//! server → channels] → summary. "remote" mode short-circuits after the
//! mode step — when this install just points at someone else's server
//! there's nothing local to configure, exactly like
//! `stepsForMode("remote")` in the GUI.

use anyhow::Result;

use ha_core::onboarding::state::mark_completed;

use super::prompt::{print_saved, println_header, println_step};
use super::steps;
use super::steps::mode::ModeOutcome;

/// Total number of steps reported to the user in the `[N/TOTAL]` banner
/// for the local-install path. Remote mode shows `[step/REMOTE_TOTAL]`
/// instead — see `REMOTE_TOTAL`. Must stay in sync with the actual
/// step count below.
const LOCAL_TOTAL: u32 = 11;
const REMOTE_TOTAL: u32 = 4;

/// Run the full wizard. Returns `Ok(())` when every step completed (or
/// was skipped with the user's awareness); propagates I/O / persistence
/// errors when they happen.
pub fn run() -> Result<()> {
    println_header("Hope Agent — First-run setup");
    println!(
        "  Walking through up to {} short steps. Each can be skipped",
        LOCAL_TOTAL
    );
    println!("  by pressing Enter or following the numbered prompt.");
    println!("  Picking 'Remote' on step 3 finishes the wizard early.");

    steps::language::run(1, LOCAL_TOTAL)?;
    steps::import_openclaw::run(2, LOCAL_TOTAL)?;
    let mode = steps::mode::run(3, LOCAL_TOTAL)?;

    if mode == ModeOutcome::Remote {
        println_step(4, REMOTE_TOTAL, "All done");
        print_saved("Remote target saved.");
        println!("  Launch any Hope Agent client (web GUI / desktop app) and it");
        println!("  will route through the remote server you just configured.");
        mark_completed()?;
        return Ok(());
    }

    let provider_done = steps::provider::run(4, LOCAL_TOTAL)?;
    steps::profile::run(5, LOCAL_TOTAL)?;
    steps::personality::run(6, LOCAL_TOTAL)?;
    steps::safety::run(7, LOCAL_TOTAL)?;
    steps::skills::run(8, LOCAL_TOTAL)?;
    steps::server::run(9, LOCAL_TOTAL)?;
    steps::channels::run(10, LOCAL_TOTAL)?;
    steps::summary::run(11, LOCAL_TOTAL, provider_done)?;

    mark_completed()?;
    Ok(())
}
