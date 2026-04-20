//! Step 4 — personality preset.

use anyhow::Result;

use ha_core::onboarding::{apply::apply_personality_preset, personality_preset_by_id};

use crate::cli_onboarding::prompt::{print_saved, print_skipped, println_step, prompt_select};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Personality preset");
    let idx = prompt_select(
        "Pick a personality starting point:",
        &[
            "Skip (use current settings)",
            "Default assistant — balanced & neutral",
            "Professional engineer — rigorous & technical",
            "Creative partner — exploratory & vivid",
            "Friendly companion — warm & conversational",
        ],
        0,
    )?;
    let preset_id = match idx {
        1 => "default",
        2 => "engineer",
        3 => "creative",
        4 => "companion",
        _ => {
            print_skipped("Personality step skipped");
            return Ok(());
        }
    };
    let preset = personality_preset_by_id(preset_id)
        .ok_or_else(|| anyhow::anyhow!("unknown preset: {}", preset_id))?;
    apply_personality_preset(preset)?;
    print_saved(&format!("Personality preset '{}' applied", preset_id));
    Ok(())
}
