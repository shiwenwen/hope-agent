//! Terminal-only authentication helpers.
//!
//! These are used by `hope-agent auth ...` and by the server setup wizard.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use ha_core::oauth::{self, TokenData};
use ha_core::provider::ActiveModelUpdate;
use tokio::sync::Mutex;

const DEFAULT_CODEX_MODEL: &str = "gpt-5.4";

#[derive(Debug, Clone)]
pub struct CodexLoginOptions {
    pub open_browser: bool,
    pub active_model: ActiveModelUpdate,
}

impl Default for CodexLoginOptions {
    fn default() -> Self {
        Self {
            open_browser: true,
            active_model: ActiveModelUpdate::Always(DEFAULT_CODEX_MODEL.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodexLoginOutcome {
    pub account_id: String,
    pub auth_path: std::path::PathBuf,
}

pub fn run(args: &[String]) {
    let Some(first) = args.first().map(|s| s.as_str()) else {
        print_help();
        return;
    };

    match first {
        "codex" => run_codex(&args[1..]),
        "--help" | "-h" => print_help(),
        "--version" => println!("hope-agent-auth {}", env!("CARGO_PKG_VERSION")),
        other => {
            eprintln!("[auth] Unknown provider: {}", other);
            print_help();
            std::process::exit(2);
        }
    }
}

fn run_codex(args: &[String]) {
    let action = args.first().map(|s| s.as_str()).unwrap_or("login");
    let result = match action {
        "login" => run_codex_login(&args[1..]),
        "status" => run_codex_status(&args[1..]),
        "logout" => run_codex_logout(&args[1..]),
        "--help" | "-h" => {
            print_codex_help();
            Ok(())
        }
        other => Err(anyhow!("unknown Codex auth action: {}", other)),
    };

    if let Err(e) = result {
        eprintln!("[auth] {}", e);
        std::process::exit(1);
    }
}

fn run_codex_login(args: &[String]) -> Result<()> {
    let mut open_browser = true;
    let mut model = DEFAULT_CODEX_MODEL.to_string();
    let mut make_active = true;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-open" => open_browser = false,
            "--open" => open_browser = true,
            "--model" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err(anyhow!("--model requires a model id"));
                };
                model = value.clone();
            }
            "--no-active" => make_active = false,
            "--help" | "-h" => {
                print_codex_login_help();
                return Ok(());
            }
            other => return Err(anyhow!("unknown login option: {}", other)),
        }
        i += 1;
    }

    if !ha_core::agent::is_valid_codex_model(&model) {
        let known = ha_core::agent::get_codex_models()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "unknown Codex model '{}'. Known models: {}",
            model,
            known
        ));
    }

    let active_model = if make_active {
        ActiveModelUpdate::Always(model)
    } else {
        ActiveModelUpdate::Never
    };
    let outcome = login_codex(CodexLoginOptions {
        open_browser,
        active_model,
    })?;

    println!("Codex login completed.");
    println!("  Account: {}", outcome.account_id);
    println!("  Token:   {}", outcome.auth_path.display());
    Ok(())
}

fn run_codex_status(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_codex_status_help();
        return Ok(());
    }
    if let Some(other) = args.first() {
        return Err(anyhow!("unknown status option: {}", other));
    }

    ha_core::paths::ensure_dirs()?;
    match oauth::load_token()? {
        Some(token) => {
            let account_id = token
                .account_id
                .clone()
                .or_else(|| oauth::extract_account_id(&token.access_token))
                .unwrap_or_else(|| "<unknown>".to_string());
            let state = if oauth::is_token_expired(&token) {
                "expired"
            } else {
                "authenticated"
            };
            println!("Codex OAuth: {}", state);
            println!("  Account: {}", account_id);
            println!("  Token:   {}", ha_core::paths::auth_path()?.display());
            if token.refresh_token.is_some() {
                println!("  Refresh: available");
            } else {
                println!("  Refresh: missing");
            }
        }
        None => {
            println!("Codex OAuth: not authenticated");
            println!("  Run: hope-agent auth codex login");
        }
    }
    Ok(())
}

fn run_codex_logout(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_codex_logout_help();
        return Ok(());
    }
    if let Some(other) = args.first() {
        return Err(anyhow!("unknown logout option: {}", other));
    }

    ha_core::paths::ensure_dirs()?;
    ha_core::provider::delete_providers_by_api_type(ha_core::provider::ApiType::Codex, "cli")?;
    oauth::clear_token()?;
    println!("Codex OAuth token cleared.");
    Ok(())
}

pub fn login_codex(options: CodexLoginOptions) -> Result<CodexLoginOutcome> {
    ha_core::paths::ensure_dirs()?;

    let rt = tokio::runtime::Runtime::new()?;
    let token = rt.block_on(login_codex_async(options.open_browser))?;
    let account_id = token
        .account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token))
        .ok_or_else(|| anyhow!("failed to extract account id from Codex token"))?;

    oauth::save_token(&token)?;
    ha_core::provider::ensure_codex_provider_persisted(options.active_model, "cli-auth")?;

    Ok(CodexLoginOutcome {
        account_id,
        auth_path: ha_core::paths::auth_path()?,
    })
}

async fn login_codex_async(open_browser: bool) -> Result<TokenData> {
    let slot = Arc::new(Mutex::new(None));
    let auth_url = oauth::start_oauth_flow_with_auth_url(slot.clone(), open_browser).await?;

    println!("Codex OAuth login");
    if open_browser {
        println!("  Browser opened. If it did not open, use this URL:");
    } else {
        println!("  Open this URL in a browser:");
    }
    println!();
    println!("{}", auth_url);
    println!();
    println!("Waiting for callback on http://localhost:1455/auth/callback ...");
    println!("Tip for remote SSH: forward the callback with `ssh -L 1455:127.0.0.1:1455 <host>`.");

    loop {
        {
            let mut lock = slot.lock().await;
            if let Some(result) = lock.take() {
                return result;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn print_help() {
    println!("Hope Agent auth");
    println!();
    println!("Usage: hope-agent auth <provider> <command> [OPTIONS]");
    println!();
    println!("Providers:");
    println!("  codex                             ChatGPT/Codex OAuth");
    println!();
    println!("Examples:");
    println!("  hope-agent auth codex login");
    println!("  hope-agent auth codex login --no-open");
    println!("  hope-agent auth codex status");
    println!("  hope-agent auth codex logout");
}

fn print_codex_help() {
    println!("Hope Agent Codex auth");
    println!();
    println!("Usage: hope-agent auth codex <command> [OPTIONS]");
    println!();
    println!("Commands:");
    println!("  login                             Sign in with ChatGPT OAuth");
    println!("  status                            Show saved token status");
    println!("  logout                            Remove saved token and Codex provider");
}

fn print_codex_login_help() {
    println!("Sign in to Codex with ChatGPT OAuth.");
    println!();
    println!("Usage: hope-agent auth codex login [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --no-open                         Print the auth URL without opening a browser");
    println!("  --open                            Open the browser (default)");
    println!("  --model MODEL                     Active Codex model (default: gpt-5.4)");
    println!("  --no-active                       Do not switch the active model to Codex");
    println!("  --help, -h                        Print help and exit");
}

fn print_codex_status_help() {
    println!("Show Codex OAuth status.");
    println!();
    println!("Usage: hope-agent auth codex status");
}

fn print_codex_logout_help() {
    println!("Remove the saved Codex OAuth token and Codex provider.");
    println!();
    println!("Usage: hope-agent auth codex logout");
}
