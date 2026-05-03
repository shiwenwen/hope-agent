//! Step 2 — provider + active-model setup.
//!
//! CLI flow:
//!   1. Pick a template (OpenAI / Codex OAuth / Anthropic / DeepSeek / Ollama / Custom)
//!   2. For Codex, complete terminal OAuth; otherwise enter Base URL + API Key
//!   3. Enter a primary model id (or accept the template default)
//!   4. Persist a `ProviderConfig` + set it as `active_model`
//!
//! We don't ping the provider from CLI — a failing request would only
//! produce a confusing error mid-wizard. The validated connection test
//! lives in the full Web GUI.

use anyhow::Result;

use ha_core::provider::{ApiType, ModelConfig, ProviderConfig};

use crate::cli_onboarding::prompt::{
    print_saved, print_skipped, println_step, prompt_input, prompt_password, prompt_select,
};

enum TemplateKind {
    ApiKey,
    Local,
    CodexOAuth,
}

struct Template {
    name: &'static str,
    api_type: ApiType,
    base_url: &'static str,
    model_id: &'static str,
    kind: TemplateKind,
}

fn templates() -> Vec<Template> {
    vec![
        Template {
            name: "OpenAI",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.openai.com/v1",
            model_id: "gpt-4o",
            kind: TemplateKind::ApiKey,
        },
        Template {
            name: "Codex (ChatGPT OAuth)",
            api_type: ApiType::Codex,
            base_url: "https://chatgpt.com/backend-api/codex",
            model_id: "gpt-5.4",
            kind: TemplateKind::CodexOAuth,
        },
        Template {
            name: "Anthropic",
            api_type: ApiType::Anthropic,
            base_url: "https://api.anthropic.com",
            model_id: "claude-sonnet-4-5",
            kind: TemplateKind::ApiKey,
        },
        Template {
            name: "DeepSeek",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.deepseek.com/v1",
            model_id: "deepseek-chat",
            kind: TemplateKind::ApiKey,
        },
        Template {
            name: "Moonshot (Kimi)",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.moonshot.cn/v1",
            model_id: "moonshot-v1-32k",
            kind: TemplateKind::ApiKey,
        },
        Template {
            name: "Ollama (local)",
            api_type: ApiType::OpenaiChat,
            base_url: "http://127.0.0.1:11434/v1",
            model_id: "llama3",
            kind: TemplateKind::Local,
        },
        Template {
            name: "Custom",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.example.com/v1",
            model_id: "custom-model",
            kind: TemplateKind::ApiKey,
        },
    ]
}

pub fn run(step: u32, total: u32) -> Result<bool> {
    println_step(step, total, "Model provider");

    let tpls = templates();
    let labels: Vec<&str> = tpls.iter().map(|t| t.name).collect();
    let idx = prompt_select("Pick a provider template:", &labels, 0)?;
    let tpl = &tpls[idx];

    if matches!(tpl.kind, TemplateKind::CodexOAuth) {
        let outcome = crate::cli_auth::login_codex(crate::cli_auth::CodexLoginOptions::default())?;
        print_saved(&format!(
            "Codex OAuth saved for account {} and set as active model",
            outcome.account_id
        ));
        return Ok(true);
    }

    let provider_name = prompt_input("Provider display name", Some(tpl.name))?;
    let base_url = prompt_input("Base URL", Some(tpl.base_url))?;
    let api_key = if matches!(tpl.kind, TemplateKind::Local) {
        "ollama".to_string()
    } else {
        prompt_password("API Key")?
    };
    if api_key.is_empty() {
        print_skipped("API Key blank — skipping provider step");
        return Ok(false);
    }
    let model_id = prompt_input("Primary model id", Some(tpl.model_id))?;

    let mut provider = ProviderConfig::new(provider_name, tpl.api_type.clone(), base_url, api_key);
    provider.models = vec![ModelConfig {
        id: model_id.clone(),
        name: model_id.clone(),
        input_types: vec!["text".to_string()],
        context_window: 200_000,
        max_tokens: 8192,
        reasoning: false,
        thinking_style: None,
        cost_input: 0.0,
        cost_output: 0.0,
    }];
    ha_core::provider::add_and_activate_provider(provider, model_id, "cli-onboarding")?;

    print_saved("Provider saved and set as active model");
    Ok(true)
}
