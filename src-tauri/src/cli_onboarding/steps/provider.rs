//! Step 2 — provider + active-model setup.
//!
//! CLI flow:
//!   1. Pick a template (OpenAI / Anthropic / DeepSeek / Ollama / Custom)
//!   2. Enter Base URL + API Key
//!   3. Enter a primary model id (or accept the template default)
//!   4. Persist a `ProviderConfig` + set it as `active_model`
//!
//! We don't ping the provider from CLI — a failing request would only
//! produce a confusing error mid-wizard. The validated connection test
//! lives in the full Web GUI.

use anyhow::Result;

use ha_core::config::{load_config, save_config};
use ha_core::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};

use crate::cli_onboarding::prompt::{
    print_saved, print_skipped, println_step, prompt_input, prompt_password, prompt_select,
};

struct Template {
    name: &'static str,
    api_type: ApiType,
    base_url: &'static str,
    model_id: &'static str,
    is_local: bool,
}

fn templates() -> Vec<Template> {
    vec![
        Template {
            name: "OpenAI",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.openai.com/v1",
            model_id: "gpt-4o",
            is_local: false,
        },
        Template {
            name: "Anthropic",
            api_type: ApiType::Anthropic,
            base_url: "https://api.anthropic.com",
            model_id: "claude-sonnet-4-5",
            is_local: false,
        },
        Template {
            name: "DeepSeek",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.deepseek.com/v1",
            model_id: "deepseek-chat",
            is_local: false,
        },
        Template {
            name: "Moonshot (Kimi)",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.moonshot.cn/v1",
            model_id: "moonshot-v1-32k",
            is_local: false,
        },
        Template {
            name: "Ollama (local)",
            api_type: ApiType::OpenaiChat,
            base_url: "http://127.0.0.1:11434/v1",
            model_id: "llama3",
            is_local: true,
        },
        Template {
            name: "Custom",
            api_type: ApiType::OpenaiChat,
            base_url: "https://api.example.com/v1",
            model_id: "custom-model",
            is_local: false,
        },
    ]
}

pub fn run(step: u32, total: u32) -> Result<bool> {
    println_step(step, total, "Model provider");

    let tpls = templates();
    let labels: Vec<&str> = tpls.iter().map(|t| t.name).collect();
    let idx = prompt_select("Pick a provider template:", &labels, 0)?;
    let tpl = &tpls[idx];

    let provider_name = prompt_input("Provider display name", Some(tpl.name))?;
    let base_url = prompt_input("Base URL", Some(tpl.base_url))?;
    let api_key = if tpl.is_local {
        "ollama".to_string()
    } else {
        prompt_password("API Key")?
    };
    if api_key.is_empty() {
        print_skipped("API Key blank — skipping provider step");
        return Ok(false);
    }
    let model_id = prompt_input("Primary model id", Some(tpl.model_id))?;

    let mut config = load_config()?;
    let mut provider = ProviderConfig::new(provider_name, tpl.api_type.clone(), base_url, api_key);
    provider.models = vec![ModelConfig {
        id: model_id.clone(),
        name: model_id.clone(),
        input_types: vec!["text".to_string()],
        context_window: 200_000,
        max_tokens: 8192,
        reasoning: false,
        cost_input: 0.0,
        cost_output: 0.0,
    }];
    let provider_id = provider.id.clone();
    config.providers.push(provider);
    config.active_model = Some(ActiveModel {
        provider_id,
        model_id,
    });
    save_config(&config)?;

    print_saved("Provider saved and set as active model");
    Ok(true)
}
