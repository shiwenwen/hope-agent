//! Step 5 — tool approvals toggle.

use anyhow::Result;

use ha_core::onboarding::apply::{apply_safety, SafetyStepInput};

use crate::cli_onboarding::prompt::{print_saved, println_step, prompt_confirm};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Safety & tool approvals");
    println!("  By default Hope Agent asks for your approval before running");
    println!("  shell commands, editing files, or calling other sensitive tools.");
    println!();
    let enabled = prompt_confirm("Require approvals (recommended)", true)?;
    apply_safety(SafetyStepInput {
        approvals_enabled: enabled,
    })?;
    let msg = if enabled {
        "Tool approvals enabled"
    } else {
        "Tool approvals disabled — tools will auto-proceed"
    };
    print_saved(msg);
    Ok(())
}
