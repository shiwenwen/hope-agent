//! Step 6 — bundled skills allow-list.

use anyhow::Result;

use ha_core::config::load_config;
use ha_core::onboarding::apply::apply_skills;
use ha_core::skills::discovery::load_all_skills_with_extra;

use crate::cli_onboarding::prompt::{print_saved, print_skipped, println_step, prompt_multiselect};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Bundled skills");

    let extra = load_config()?.extra_skills_dirs.clone();
    let skills = load_all_skills_with_extra(&extra);
    // Keep only bundled, non-`always:true` skills. Core skills like
    // `ha-settings` / `ha-skill-creator` set `requires.always = true` so
    // the model never loses config-management. Filtering by name here
    // would miss those — frontmatter is the source of truth.
    let bundled: Vec<_> = skills
        .into_iter()
        .filter(|s| s.source == "bundled" && !s.requires.always)
        .collect();

    if bundled.is_empty() {
        print_skipped("No optional bundled skills detected");
        return Ok(());
    }

    let disabled_prev = load_config()?.disabled_skills.clone();
    let disabled_set: std::collections::HashSet<&str> =
        disabled_prev.iter().map(|s| s.as_str()).collect();

    let labels: Vec<String> = bundled.iter().map(|s| s.name.clone()).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let defaults: Vec<bool> = bundled
        .iter()
        .map(|s| !disabled_set.contains(s.name.as_str()))
        .collect();

    let selection = prompt_multiselect(
        "Toggle bundled skills on/off (checked = enabled):",
        &label_refs,
        &defaults,
    )?;

    let new_disabled: Vec<String> = bundled
        .iter()
        .zip(selection.iter())
        .filter(|(_, enabled)| !**enabled)
        .map(|(s, _)| s.name.clone())
        .collect();

    apply_skills(new_disabled.clone())?;
    print_saved(&format!("{} bundled skill(s) disabled", new_disabled.len()));
    Ok(())
}
