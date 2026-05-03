//! Step 2 — optional OpenClaw → Hope Agent one-shot import.
//!
//! Mirrors the GUI `ImportOpenClawStep` + `OpenClawImportPanel`. The GUI
//! gives users multi-select checkboxes per provider / agent / memory and
//! lets them rename agents on the way in. CLI keeps the surface tight:
//! one yes/no per category, accepting the scan defaults (target_id =
//! source_id, all available files imported, no vibe edits). Power users
//! who need finer control should run the GUI wizard once.
//!
//! Skipped silently when OpenClaw isn't installed on this machine.

use anyhow::Result;

use ha_core::openclaw_import::{
    import_openclaw_full, scan_openclaw_full, ImportAgentRequest, OpenClawImportRequest,
};

use crate::cli_onboarding::prompt::{
    print_error, print_saved, print_skipped, println_step, prompt_confirm,
};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Import from OpenClaw (optional)");

    let preview = match scan_openclaw_full() {
        Ok(p) => p,
        Err(e) => {
            print_skipped(&format!("OpenClaw scan failed: {e}. Continuing."));
            return Ok(());
        }
    };

    if !preview.state_dir_present {
        print_skipped(&format!(
            "No OpenClaw install detected at {}",
            preview.state_dir
        ));
        return Ok(());
    }

    let new_providers = preview
        .providers
        .iter()
        .filter(|p| !p.name_conflicts_existing)
        .count();
    let new_agents = preview
        .agents
        .iter()
        .filter(|a| !a.already_exists)
        .count();
    let agent_md_count = preview.memories.agent_md_counts.len();
    let global_md = preview.memories.global_md_present;

    let nothing_to_import =
        new_providers == 0 && new_agents == 0 && agent_md_count == 0 && !global_md;
    if nothing_to_import {
        print_skipped(&format!(
            "OpenClaw at {} but nothing new to import (already up to date).",
            preview.state_dir
        ));
        return Ok(());
    }

    println!("  Detected OpenClaw install at {}", preview.state_dir);
    println!(
        "  Available: {} provider(s), {} agent(s), {} agent memory file(s), global memory: {}",
        new_providers,
        new_agents,
        agent_md_count,
        if global_md { "yes" } else { "no" }
    );
    for w in &preview.warnings {
        println!("  ⚠ {w}");
    }

    if !prompt_confirm("Import everything above?", true)? {
        print_skipped("OpenClaw import skipped");
        return Ok(());
    }

    let request = OpenClawImportRequest {
        import_provider_keys: preview
            .providers
            .iter()
            .filter(|p| !p.name_conflicts_existing)
            .map(|p| p.source_key.clone())
            .collect(),
        import_agents: preview
            .agents
            .iter()
            .filter(|a| !a.already_exists)
            .map(|a| ImportAgentRequest {
                source_id: a.id.clone(),
                target_id: a.id.clone(),
                name: a.name.clone(),
                emoji: a.emoji.clone(),
                vibe: None,
                sandbox: a.sandbox,
                import_files: a.available_files.clone(),
            })
            .collect(),
        import_global_memory: global_md,
        import_agent_memories: preview
            .memories
            .agent_md_counts
            .iter()
            .map(|(id, _)| id.clone())
            .collect(),
    };

    match import_openclaw_full(&request) {
        Ok(summary) => {
            let agent_ok = summary.agents.iter().filter(|a| a.success).count();
            let agent_fail = summary.agents.len() - agent_ok;
            print_saved(&format!(
                "Imported {} provider(s), {} agent(s){}, {} memory entry/entries",
                summary.providers_added.len(),
                agent_ok,
                if agent_fail > 0 {
                    format!(" ({} failed)", agent_fail)
                } else {
                    String::new()
                },
                summary.memories_added,
            ));
            for w in &summary.warnings {
                println!("  ⚠ {w}");
            }
        }
        Err(e) => {
            print_error(&format!("Import failed: {e}"));
        }
    }
    Ok(())
}
