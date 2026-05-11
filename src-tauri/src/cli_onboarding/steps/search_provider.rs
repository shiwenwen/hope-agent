//! Web search provider setup.

use anyhow::Result;

use ha_core::config::load_config;
use ha_core::onboarding::apply::apply_web_search;
use ha_core::tools::web_search::{WebSearchConfig, WebSearchProvider, WebSearchProviderEntry};

use crate::cli_onboarding::prompt::{
    print_saved, print_skipped, println_step, prompt_input, prompt_optional, prompt_password,
    prompt_select,
};

struct SearchTemplate {
    provider: WebSearchProvider,
    label: &'static str,
}

fn templates() -> Vec<SearchTemplate> {
    vec![
        SearchTemplate {
            provider: WebSearchProvider::DuckDuckGo,
            label: "DuckDuckGo (free, no API key)",
        },
        SearchTemplate {
            provider: WebSearchProvider::Searxng,
            label: "SearXNG (self-hosted)",
        },
        SearchTemplate {
            provider: WebSearchProvider::Tavily,
            label: "Tavily",
        },
        SearchTemplate {
            provider: WebSearchProvider::Bocha,
            label: "Bocha AI Search",
        },
        SearchTemplate {
            provider: WebSearchProvider::Brave,
            label: "Brave Search",
        },
        SearchTemplate {
            provider: WebSearchProvider::Perplexity,
            label: "Perplexity",
        },
        SearchTemplate {
            provider: WebSearchProvider::Google,
            label: "Google Custom Search",
        },
        SearchTemplate {
            provider: WebSearchProvider::Grok,
            label: "Grok (X.AI)",
        },
        SearchTemplate {
            provider: WebSearchProvider::Kimi,
            label: "Kimi (Moonshot)",
        },
    ]
}

pub fn run(step: u32, total: u32) -> Result<bool> {
    println_step(step, total, "Web search provider");
    println!("  Configure the provider used by the web_search tool.");
    println!("  DuckDuckGo works without a key; API providers can be added now or later.");
    println!();

    let mut config = load_config()?.web_search;
    ha_core::tools::web_search::backfill_providers(&mut config);

    let templates = templates();
    let mut labels: Vec<&str> = templates.iter().map(|t| t.label).collect();
    labels.push("Skip for now");

    let default_idx = first_enabled_template_idx(&config, &templates).unwrap_or(0);
    let selected = prompt_select("Pick a search provider:", &labels, default_idx)?;
    if selected >= templates.len() {
        print_skipped("Search provider unchanged");
        return Ok(false);
    }

    let template = &templates[selected];
    let Some(mut entry) = take_provider(&mut config, &template.provider) else {
        print_skipped("Selected provider is not available in this build");
        return Ok(false);
    };

    if !configure_entry(&mut entry)? {
        print_skipped("Missing required search provider credentials - step skipped");
        return Ok(false);
    }

    entry.enabled = true;
    config.providers.insert(0, entry);
    apply_web_search(config)?;
    print_saved(&format!("Search provider saved: {}", template.label));
    Ok(true)
}

fn first_enabled_template_idx(
    config: &WebSearchConfig,
    templates: &[SearchTemplate],
) -> Option<usize> {
    let first = config.providers.iter().find(|entry| entry.enabled)?;
    templates
        .iter()
        .position(|template| &template.provider == &first.id)
}

fn take_provider(
    config: &mut WebSearchConfig,
    provider: &WebSearchProvider,
) -> Option<WebSearchProviderEntry> {
    let pos = config
        .providers
        .iter()
        .position(|entry| &entry.id == provider)?;
    Some(config.providers.remove(pos))
}

fn configure_entry(entry: &mut WebSearchProviderEntry) -> Result<bool> {
    match entry.id.clone() {
        WebSearchProvider::DuckDuckGo => Ok(true),
        WebSearchProvider::Searxng => {
            let default_url = entry
                .base_url
                .as_deref()
                .filter(|url| !url.is_empty())
                .unwrap_or("http://127.0.0.1:8080");
            let url = prompt_input("SearXNG instance URL", Some(default_url))?;
            if url.trim().is_empty() {
                return Ok(false);
            }
            entry.base_url = Some(url);
            Ok(true)
        }
        WebSearchProvider::Google => {
            if !prompt_api_key(entry, "Google API Key")? {
                return Ok(false);
            }
            let cx = prompt_optional("Google Search Engine ID (CX)", entry.api_key2.as_deref())?;
            match cx {
                Some(value) if !value.trim().is_empty() => entry.api_key2 = Some(value),
                _ if entry.api_key2.as_deref().unwrap_or("").trim().is_empty() => return Ok(false),
                _ => {}
            }
            Ok(true)
        }
        WebSearchProvider::Bocha
        | WebSearchProvider::Brave
        | WebSearchProvider::Perplexity
        | WebSearchProvider::Grok
        | WebSearchProvider::Kimi
        | WebSearchProvider::Tavily => prompt_api_key(entry, "API Key"),
    }
}

fn prompt_api_key(entry: &mut WebSearchProviderEntry, label: &str) -> Result<bool> {
    let has_existing = entry
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    if has_existing {
        println!("  Existing API key found; leave blank to keep it.");
    }
    let value = prompt_password(label)?;
    if value.trim().is_empty() {
        return Ok(has_existing);
    }
    entry.api_key = Some(value);
    Ok(true)
}
