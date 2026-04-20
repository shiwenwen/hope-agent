//! Step 7 — server exposure (bind address + optional API key).

use anyhow::Result;

use ha_core::onboarding::apply::{apply_server, generate_api_key, ServerStepInput};

use crate::cli_onboarding::prompt::{print_saved, println_step, prompt_confirm, prompt_select};

pub fn run(step: u32, total: u32) -> Result<()> {
    println_step(step, total, "Server exposure");

    let bind_idx = prompt_select(
        "How should the embedded HTTP server listen?",
        &[
            "This device only (127.0.0.1:8420) — safe default",
            "Same local network (0.0.0.0:8420) — accessible from other devices",
        ],
        0,
    )?;
    let bind_addr = if bind_idx == 0 {
        "127.0.0.1:8420".to_string()
    } else {
        "0.0.0.0:8420".to_string()
    };

    // When exposing to LAN, push the API key toggle on by default so we
    // don't accidentally teach users that 0.0.0.0 without auth is normal.
    let default_require_key = bind_idx == 1;
    let want_key = prompt_confirm("Require an API key", default_require_key)?;
    let api_key = if want_key {
        let generated = generate_api_key();
        println!(
            "  {}Generated API key:{} {}",
            crate::cli_onboarding::prompt::color::DIM,
            crate::cli_onboarding::prompt::color::RESET,
            generated
        );
        Some(generated)
    } else {
        Some(String::new()) // empty string clears existing key
    };

    apply_server(ServerStepInput {
        bind_addr: Some(bind_addr.clone()),
        api_key,
    })?;
    print_saved(&format!(
        "Server bind saved ({}){}",
        bind_addr,
        if want_key { ", API key set" } else { "" }
    ));
    println!(
        "  {}Note:{} bind-address changes take effect after the next server restart.",
        crate::cli_onboarding::prompt::color::YELLOW,
        crate::cli_onboarding::prompt::color::RESET
    );
    Ok(())
}
