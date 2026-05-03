//! Final step — read every persisted setting back and print a recap so
//! the operator sees what just got saved. Mirrors the GUI `SummaryStep`
//! recap card; the GUI also shows a clickable Web GUI URL with optional
//! `?token=` for sharing — we do the same in stdout.
//!
//! Replaces the old hard-coded "All done" banner inside `wizard.rs`.

use anyhow::Result;

use ha_core::agent_loader::DEFAULT_AGENT_ID;
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

    let personality_label = read_personality_preset_label();

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

fn read_personality_preset_label() -> String {
    let path = match ha_core::paths::agent_dir(DEFAULT_AGENT_ID) {
        Ok(p) => p.join("agent.json"),
        Err(_) => return "—".to_string(),
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return "—".to_string(),
    };
    let v: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return "—".to_string(),
    };
    v.get("personality")
        .and_then(|p| p.get("preset"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string())
}

/// Build the user-facing Web GUI URLs. Mirrors GUI `SummaryStep` logic:
/// localhost first, then up to N LAN IPs when bind is `0.0.0.0`. Token
/// gets appended as `?token=...` so the URL is share-ready.
fn build_web_urls(bind_addr: &str, api_key: Option<&str>) -> Vec<String> {
    let port = bind_addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8420);
    let bind_host = bind_addr
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(bind_addr);

    let mut hosts: Vec<String> = vec!["localhost".to_string()];
    if bind_host == "0.0.0.0" {
        for ip in ha_server::banner::local_ipv4_addresses() {
            if !hosts.contains(&ip) {
                hosts.push(ip);
            }
        }
    } else if !["127.0.0.1", "localhost"].contains(&bind_host) {
        hosts.insert(0, bind_host.to_string());
    }

    let token_suffix = match api_key {
        Some(k) if !k.is_empty() => format!("/?token={}", k),
        _ => "/".to_string(),
    };

    hosts
        .into_iter()
        .map(|h| format!("http://{}:{}{}", h, port, token_suffix))
        .collect()
}
