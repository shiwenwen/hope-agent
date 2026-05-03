//! Step 3 — install mode: local install vs connect to a remote hope-agent.
//!
//! Mirrors the GUI `ModeStep`. "local" continues the wizard normally
//! (provider / profile / ... / channels). "remote" prompts URL + optional
//! API key, probes `<url>/api/health` (10 s timeout, optional Bearer),
//! persists the three remote fields into `user_config`, and signals the
//! wizard to short-circuit — there is nothing local-side to configure
//! once we point at someone else's server.

use std::time::Duration;

use anyhow::Result;

use ha_core::onboarding::apply::{apply_remote_mode, RemoteModeInput};

use crate::cli_onboarding::prompt::{
    print_error, print_saved, print_skipped, println_step, prompt_input, prompt_optional,
    prompt_select,
};

/// Outcome of the mode step. `Local` continues the wizard; `Remote`
/// tells the wizard to print the remote summary and exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeOutcome {
    Local,
    Remote,
}

pub fn run(step: u32, total: u32) -> Result<ModeOutcome> {
    println_step(step, total, "Install mode");
    println!("  Pick how this Hope Agent install should run.");
    println!();

    let choice = prompt_select(
        "Choose:",
        &[
            "Local install — configure providers, agents, channels on this machine",
            "Remote — connect to an existing hope-agent server",
        ],
        0,
    )?;

    if choice == 0 {
        print_saved("Local install — continuing wizard");
        return Ok(ModeOutcome::Local);
    }

    // ── Remote branch ──────────────────────────────────────────────
    println!();
    println!("  Remote mode points this machine at a hope-agent server you");
    println!("  already run elsewhere (LAN box, VPS, etc.). The provider,");
    println!("  agents and channels live there — nothing more to configure here.");
    println!();

    let url_raw = prompt_input("Server URL (e.g. http://192.168.1.10:8420)", None)?;
    let url = url_raw.trim().trim_end_matches('/').to_string();
    if !is_acceptable_url(&url) {
        print_error("URL must start with http:// or https://. Skipping remote setup.");
        return Ok(ModeOutcome::Local);
    }

    let api_key = prompt_optional("API key (Bearer token, blank = none)", None)?;

    match probe_remote(&url, api_key.as_deref()) {
        Ok(status) => {
            print_saved(&format!("Reachable: {}", status));
        }
        Err(e) => {
            print_error(&format!("Probe failed: {}", e));
            // Save anyway — user might be configuring ahead of bringing
            // the remote up. Mirrors the GUI's "Connect" button only
            // gating on probe success, but CLI users running this on a
            // headless box might want to commit the config blind. We
            // still warn so they know.
            print_skipped("Saving the remote target anyway. Verify before relying on it.");
        }
    }

    apply_remote_mode(RemoteModeInput {
        url: url.clone(),
        api_key,
    })?;
    print_saved(&format!("Remote target saved: {}", url));
    Ok(ModeOutcome::Remote)
}

fn is_acceptable_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Synchronous `/api/health` probe with 10 s timeout. We use the blocking
/// reqwest client so this step doesn't have to spin up a tokio runtime
/// just for one HTTP call.
fn probe_remote(url: &str, api_key: Option<&str>) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut req = client.get(format!("{}/api/health", url));
    if let Some(key) = api_key {
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
    }
    let resp = req.send()?;
    let status = resp.status();
    if status.is_success() {
        Ok(format!("{} OK", status.as_u16()))
    } else {
        let body = resp.text().unwrap_or_default();
        let snippet = body.chars().take(120).collect::<String>();
        Err(anyhow::anyhow!("{} {}", status.as_u16(), snippet))
    }
}
