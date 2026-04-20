//! Top-level CLI wizard orchestrator — called from `run_server` on a TTY
//! and from the dedicated `hope-agent server setup` subcommand.
//!
//! Each step is a separate module under `steps/`. On success, the wizard
//! marks the onboarding complete so the next launch skips the flow.
//! Interrupting with Ctrl+C leaves the draft untouched — the Rust-side
//! state store only persists at step boundaries, so a partial run still
//! stores useful progress.

use anyhow::Result;

use ha_core::onboarding::state::mark_completed;

use super::prompt::{println_header, println_step};
use super::steps;

/// Total number of steps reported to the user in the `[N/TOTAL]` banner.
/// Must match the number of steps we actually prompt through.
const TOTAL_STEPS: u32 = 9;

/// Run the full wizard. Returns `Ok(())` when every step completed (or
/// was skipped with the user's awareness); propagates I/O / persistence
/// errors when they happen.
pub fn run() -> Result<()> {
    println_header("Hope Agent — First-run setup");
    println!(
        "  Walking through {} short steps. Each step can be skipped",
        TOTAL_STEPS
    );
    println!("  by pressing Enter or following the numbered prompt.");

    steps::language::run(1, TOTAL_STEPS)?;
    let provider_done = steps::provider::run(2, TOTAL_STEPS)?;
    steps::profile::run(3, TOTAL_STEPS)?;
    steps::personality::run(4, TOTAL_STEPS)?;
    steps::safety::run(5, TOTAL_STEPS)?;
    steps::skills::run(6, TOTAL_STEPS)?;
    steps::server::run(7, TOTAL_STEPS)?;
    steps::channels::run(8, TOTAL_STEPS)?;

    println_step(9, TOTAL_STEPS, "All done");
    if provider_done {
        println!("  Provider configured. Starting service…");
    } else {
        println!("  Provider not configured — chat won't work until you set one up");
        println!("  in the Web GUI.");
    }
    mark_completed()?;
    Ok(())
}
