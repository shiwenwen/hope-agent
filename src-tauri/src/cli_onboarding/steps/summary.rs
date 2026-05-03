//! Final step — read every persisted setting back and print a recap so
//! the operator sees what just got saved. Mirrors the GUI `SummaryStep`
//! recap card; the GUI also shows a clickable Web GUI URL with optional
//! `?token=` for sharing — we do the same in stdout.

use anyhow::Result;

use ha_core::agent_loader::{load_agent, DEFAULT_AGENT_ID};
use ha_core::config::cached_config;
use ha_core::user_config::load_user_config;

use crate::cli_onboarding::prompt::{print_saved, println_step};

pub fn run(step: u32, total: u32, provider_done: bool) -> Result<()> {
    println_step(step, total, "Summary");

    let cfg = cached_config();
    let user = load_user_config().unwrap_or_default();

    let language = if cfg.language.is_empty() {
        "auto".to_string()
    } else {
        cfg.language.clone()
    };

    let provider_label = if provider_done {
        let active = cfg
            .active_model
            .as_ref()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "(no active model)".to_string());
        format!("Configured · active model: {active}")
    } else {
        "Not configured — chat will not work until you set one up".to_string()
    };

    let profile_bits: Vec<String> = [user.name.clone(), user.ai_experience.clone()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let profile_label = if profile_bits.is_empty() {
        "—".to_string()
    } else {
        profile_bits.join(" · ")
    };

    let personality_label = read_personality_label();

    let approvals_on = cfg.permission.approval_timeout_secs > 0;
    let safety_label = if approvals_on {
        format!(
            "Approvals on (timeout {}s)",
            cfg.permission.approval_timeout_secs
        )
    } else {
        "Approvals off — tools auto-proceed".to_string()
    };

    let skills_label = format!("{} bundled skill(s) disabled", cfg.disabled_skills.len());

    let server_label = match cfg.server.api_key.as_deref() {
        Some(k) if !k.is_empty() => format!("bind {} · API key set", cfg.server.bind_addr),
        _ => format!("bind {} · no API key", cfg.server.bind_addr),
    };

    println!("  Language     : {language}");
    println!("  Provider     : {provider_label}");
    println!("  Profile      : {profile_label}");
    println!("  Personality  : {personality_label}");
    println!("  Safety       : {safety_label}");
    println!("  Skills       : {skills_label}");
    println!("  Server       : {server_label}");

    println!();
    println!("  Web GUI URL(s):");
    let urls = build_web_urls(&cfg.server.bind_addr, cfg.server.api_key.as_deref());
    for url in &urls {
        println!("    {url}");
    }
    println!();
    println!(
        "  Start the service with:  {}hope-agent server{}",
        crate::cli_onboarding::prompt::color::BOLD,
        crate::cli_onboarding::prompt::color::RESET
    );
    println!();
    print_saved("Onboarding complete");
    Ok(())
}

/// Best-effort label for the default agent's personality. The persisted
/// `PersonalityConfig` doesn't keep the wizard preset id around — it
/// stores the expanded role / vibe / tone — so we surface `role` as the
/// most operator-meaningful field, falling back to a flat "default"
/// when role is unset (matches the Default preset's empty config).
fn read_personality_label() -> String {
    match load_agent(DEFAULT_AGENT_ID) {
        Ok(def) => match def.config.personality.role {
            Some(r) if !r.is_empty() => r,
            _ => "default".to_string(),
        },
        Err(_) => "—".to_string(),
    }
}

/// Build the user-facing Web GUI URLs by delegating host/port expansion
/// to `ha_server::banner::display_host_urls` (same path used by the
/// `print_launch_banner` so the wizard recap and the eventual server
/// boot banner show identical URLs), then appending the token suffix.
fn build_web_urls(bind_addr: &str, api_key: Option<&str>) -> Vec<String> {
    let token_suffix = api_key
        .filter(|k| !k.is_empty())
        .map(|k| format!("/?token={}", k))
        .unwrap_or_else(|| "/".to_string());

    ha_server::banner::display_host_urls(bind_addr)
        .into_iter()
        .map(|base| format!("{}{}", base, token_suffix))
        .collect()
}
