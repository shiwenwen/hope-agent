use anyhow::Result;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::time::Duration;

use crate::tools::execution::ToolExecContext;
use crate::ttl_cache::TtlCache;

mod bocha;
mod brave;
mod duckduckgo;
mod google;
mod grok;
mod helpers;
mod keyless;
mod kimi;
mod perplexity;
mod searxng;
mod tavily;

const WEB_SEARCH_CACHE_MAX_ENTRIES: usize = 200;

// ── Web Search Provider Config ───────────────────────────────────

// 类型已下沉 ha-config-schema：配置 wire 类型 + serde default helper；
// `default_providers` / `DEFAULT_WEB_SEARCH_TIMEOUT_SECS` 为可见性升级
// 再导出（本 crate 的 backfill_providers / duckduckgo 仍引用）。
pub use ha_config_schema::tools::web_search::{
    default_providers, WebSearchConfig, WebSearchProvider, WebSearchProviderEntry,
    DEFAULT_WEB_SEARCH_TIMEOUT_SECS,
};

/// Ensure all known providers exist in the list (appends any missing ones).
/// This handles the case where a new provider is added but the user's saved config
/// was created before that provider existed.
pub fn backfill_providers(config: &mut WebSearchConfig) {
    let was_enabled = has_enabled_provider(config);
    let had_keyless_search = config
        .providers
        .iter()
        .any(|entry| entry.id == WebSearchProvider::DuckDuckGo && entry.enabled);
    let defaults = default_providers();
    for default_entry in &defaults {
        if !config.providers.iter().any(|p| p.id == default_entry.id) {
            let mut entry = default_entry.clone();
            // Adding a default fallback must not re-enable search when the user
            // has explicitly disabled every provider in an existing config.
            entry.enabled &= was_enabled;
            // Extend the existing free-search fallback, while preserving the
            // opt-out of users who enabled only credentialed/self-hosted search.
            if entry.id == WebSearchProvider::Keyless {
                entry.enabled &= had_keyless_search;
            }
            config.providers.push(entry);
        }
    }
}

/// Check if any web search provider is enabled in the config.
pub fn has_enabled_provider(config: &WebSearchConfig) -> bool {
    config.providers.iter().any(|p| p.enabled)
}

/// Collect enabled providers in order. Explicitly disabling search also closes
/// the execution path, including calls made with a stale tool definition.
fn resolve_providers(config: &WebSearchConfig) -> Vec<&WebSearchProviderEntry> {
    config
        .providers
        .iter()
        .filter(|entry| entry.enabled)
        .collect()
}

fn query_and_count(args: &Value, default_count: usize) -> Result<(&str, usize)> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| anyhow::anyhow!("A non-empty 'query' parameter is required"))?;
    let count = args
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(default_count as u64)
        .clamp(1, 10) as usize;
    Ok((query, count))
}

// ── Tool Entry Point ─────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub(super) struct WebSearchUsageContext {
    session_id: Option<String>,
    agent_id: Option<String>,
}

pub(super) fn record_llm_web_search_usage(
    ctx: &WebSearchUsageContext,
    operation: &'static str,
    provider_id: &'static str,
    provider_name: &'static str,
    model_id: &'static str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    response_body: Option<&Value>,
) {
    let mut event = crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_WEB_SEARCH);
    event.operation = Some(operation.to_string());
    event.source = Some("web_search".to_string());
    event.provider_id = Some(provider_id.to_string());
    event.provider_name = Some(provider_name.to_string());
    event.model_id = Some(model_id.to_string());
    event.session_id = ctx.session_id.clone();
    event.agent_id = ctx.agent_id.clone();
    event.duration_ms = Some(duration_ms);
    event.success = success;
    event.error = error;
    event.metadata = Some(serde_json::json!({ "provider": provider_name }));

    if let Some(usage) = response_body.and_then(|body| body.get("usage")) {
        event.input_tokens = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(|v| v.as_u64());
        event.output_tokens = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(|v| v.as_u64());
        event.cache_read_input_tokens = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
            })
            .and_then(|v| v.as_u64());
    }

    crate::model_usage::record_model_usage_best_effort(event);
}

pub(crate) async fn tool_web_search(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let mut config = crate::config::cached_config().web_search.clone();
    // Saved configurations from older releases need the same effective
    // provider list as the settings UI, even before the user saves it again.
    backfill_providers(&mut config);
    let usage_ctx = WebSearchUsageContext {
        session_id: ctx.session_id.clone(),
        agent_id: ctx.agent_id.clone(),
    };

    let (query, count) = query_and_count(args, config.default_result_count)?;

    let params = SearchParams {
        country: args
            .get("country")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_country.clone()),
        language: args
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_language.clone()),
        freshness: args
            .get("freshness")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_freshness.clone()),
    };

    let providers = resolve_providers(&config);
    if providers.is_empty() {
        anyhow::bail!("Web search is disabled; enable a provider in Settings → Web Search");
    }
    let timeout = config.timeout_seconds;

    // Try each enabled provider in order; fallback to next on error or 0 results
    let mut results = Vec::new();
    let mut used_provider = String::new();
    let mut last_error: Option<anyhow::Error> = None;
    let mut no_result_providers: Vec<String> = Vec::new();
    let mut provider_errors: Vec<(String, String)> = Vec::new();

    for entry in &providers {
        let provider_id = &entry.id;

        app_info!(
            "tool",
            "web_search",
            "Web search [{}] started (count: {}, country: {:?}, lang: {:?}, freshness: {:?})",
            provider_id,
            count,
            params.country,
            params.language,
            params.freshness
        );

        // Check cache
        let ck = search_cache_key(&provider_id.to_string(), query, count, &params);
        if let Some(cached) = read_search_cache(&ck, config.cache_ttl_minutes) {
            app_info!("tool", "web_search", "Cache hit for [{}]", provider_id);
            return Ok(cached);
        }

        let attempt = match provider_id {
            WebSearchProvider::Keyless => {
                keyless::search_keyless(query, count, &params, timeout, &usage_ctx).await
            }
            WebSearchProvider::DuckDuckGo => {
                duckduckgo::search_duckduckgo(query, count, timeout).await
            }
            WebSearchProvider::Searxng => {
                let url = entry.base_url.as_deref().unwrap_or("http://127.0.0.1:8080");
                // The constructed URL and every redirect are validated before
                // send; failures remain inside this provider's fallback result.
                searxng::search_searxng(url, query, count, &params, timeout).await
            }
            WebSearchProvider::Brave => {
                let key = entry.api_key.as_deref().unwrap_or("");
                brave::search_brave(key, query, count, &params, timeout).await
            }
            WebSearchProvider::Bocha => {
                let key = entry.api_key.as_deref().unwrap_or("");
                bocha::search_bocha(key, query, count, &params, timeout).await
            }
            WebSearchProvider::Perplexity => {
                let key = entry.api_key.as_deref().unwrap_or("");
                perplexity::search_perplexity(key, query, count, &params, timeout, &usage_ctx).await
            }
            WebSearchProvider::Google => {
                let key = entry.api_key.as_deref().unwrap_or("");
                let cx = entry.api_key2.as_deref().unwrap_or("");
                google::search_google(key, cx, query, count, &params, timeout).await
            }
            WebSearchProvider::Grok => {
                let key = entry.api_key.as_deref().unwrap_or("");
                grok::search_grok(key, query, count, timeout, &usage_ctx).await
            }
            WebSearchProvider::Kimi => {
                let key = entry.api_key.as_deref().unwrap_or("");
                kimi::search_kimi(key, query, count, timeout, &usage_ctx).await
            }
            WebSearchProvider::Tavily => {
                let key = entry.api_key.as_deref().unwrap_or("");
                tavily::search_tavily(key, query, count, &params, timeout).await
            }
        };

        match attempt {
            Ok(r) if !r.is_empty() => {
                used_provider = provider_id.to_string();
                results = r;
                break;
            }
            Ok(_) => {
                app_warn!(
                    "tool",
                    "web_search",
                    "Provider [{}] returned 0 results, trying next provider",
                    provider_id
                );
                no_result_providers.push(provider_id.to_string());
            }
            Err(e) => {
                let error_message = e.to_string();
                app_warn!(
                    "tool",
                    "web_search",
                    "Provider [{}] error: {}, trying next provider",
                    provider_id,
                    error_message
                );
                provider_errors.push((provider_id.to_string(), error_message));
                last_error = Some(e);
            }
        }
    }

    if results.is_empty() {
        if let Some(e) = last_error {
            app_warn!(
                "tool",
                "web_search",
                "All providers failed, last error: {}",
                e
            );
        }
        return Ok(format_empty_search_result(
            &no_result_providers,
            &provider_errors,
        ));
    }

    let mut output = format_search_result_header(&used_provider);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   Source: {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.source,
            result.snippet
        ));
    }
    let cache_output = output.clone();
    append_provider_diagnostics(&mut output, &no_result_providers, &provider_errors);

    // Write to cache
    let ck = search_cache_key(&used_provider, query, count, &params);
    write_search_cache(ck, cache_output, config.cache_ttl_minutes);

    Ok(output)
}

fn format_search_result_header(provider: &str) -> String {
    format!("Search results (via {})\n\n", provider)
}

fn format_empty_search_result(
    no_result_providers: &[String],
    provider_errors: &[(String, String)],
) -> String {
    let mut output = if provider_errors.is_empty() {
        "No results found.\n".to_string()
    } else if no_result_providers.is_empty() {
        "Search failed.\n\nNo configured search provider returned results.\n".to_string()
    } else {
        "No results found by available providers.\n".to_string()
    };

    append_provider_diagnostics(&mut output, no_result_providers, provider_errors);

    if !provider_errors.is_empty() {
        output.push_str(
            "\nProvider failures or rate limits are not the same as the web having no results.\n",
        );
    }

    output
}

fn append_provider_diagnostics(
    output: &mut String,
    no_result_providers: &[String],
    provider_errors: &[(String, String)],
) {
    if no_result_providers.is_empty() && provider_errors.is_empty() {
        return;
    }

    output.push('\n');

    if !no_result_providers.is_empty() {
        output.push_str("Providers with no results:\n");
        for provider in no_result_providers {
            output.push_str(&format!("- {}\n", provider));
        }
    }

    if !provider_errors.is_empty() {
        output.push_str("Providers unavailable or failed:\n");
        for (provider, error) in provider_errors {
            output.push_str(&format!("- {}: {}\n", provider, error));
        }
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    /// Which search engine/provider produced this result
    source: String,
}

// ── Search Params & Helpers ─────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct SearchParams {
    country: Option<String>,
    language: Option<String>,
    freshness: Option<String>,
}

// ── Search Result Cache ─────────────────────────────────────────

static WEB_SEARCH_CACHE: Lazy<TtlCache<String, String>> =
    Lazy::new(|| TtlCache::new(WEB_SEARCH_CACHE_MAX_ENTRIES));

fn search_cache_key(provider: &str, query: &str, count: usize, params: &SearchParams) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        provider,
        query.to_lowercase().trim(),
        count,
        params.country.as_deref().unwrap_or(""),
        params.language.as_deref().unwrap_or(""),
        params.freshness.as_deref().unwrap_or(""),
    )
}

fn read_search_cache(key: &str, ttl_minutes: u64) -> Option<String> {
    if ttl_minutes == 0 {
        return None;
    }
    WEB_SEARCH_CACHE.get(key, Duration::from_secs(ttl_minutes * 60))
}

fn write_search_cache(key: String, response: String, ttl_minutes: u64) {
    if ttl_minutes == 0 {
        return;
    }
    WEB_SEARCH_CACHE.put(key, response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyless_defaults_and_backfill_preserve_search_disabled_state() {
        let defaults = WebSearchConfig::default();
        assert_eq!(defaults.providers[0].id, WebSearchProvider::Keyless);
        assert!(defaults.providers[0].enabled);
        for enabled in [false, true] {
            let mut config = WebSearchConfig {
                providers: vec![WebSearchProviderEntry {
                    id: WebSearchProvider::DuckDuckGo,
                    enabled,
                    api_key: None,
                    api_key2: None,
                    base_url: None,
                }],
                ..defaults.clone()
            };
            backfill_providers(&mut config);
            assert_eq!(config.providers[0].id, WebSearchProvider::DuckDuckGo);
            assert_eq!(
                config
                    .providers
                    .iter()
                    .find(|entry| entry.id == WebSearchProvider::Keyless)
                    .unwrap()
                    .enabled,
                enabled
            );
            assert_eq!(has_enabled_provider(&config), enabled);
            assert_eq!(resolve_providers(&config).is_empty(), !enabled);
            let count = config.providers.len();
            backfill_providers(&mut config);
            assert_eq!(config.providers.len(), count);
        }
    }

    #[test]
    fn keyless_backfill_preserves_an_explicit_free_search_opt_out() {
        let mut config = WebSearchConfig::default();
        config
            .providers
            .retain(|entry| entry.id != WebSearchProvider::Keyless);
        for entry in &mut config.providers {
            entry.enabled = entry.id == WebSearchProvider::Brave;
            if entry.id == WebSearchProvider::Brave {
                entry.api_key = Some("TEST_ONLY_KEY".into());
            }
        }
        let existing = serde_json::to_value(&config.providers).unwrap();
        backfill_providers(&mut config);
        let added = config.providers.last().unwrap();
        assert_eq!(added.id, WebSearchProvider::Keyless);
        assert!(!added.enabled);
        assert_eq!(
            serde_json::to_value(&config.providers[..config.providers.len() - 1]).unwrap(),
            existing
        );
    }

    #[test]
    fn search_arguments_reject_blank_queries_and_bound_result_count() {
        for query in [serde_json::json!(null), serde_json::json!(" \n\t")] {
            assert!(query_and_count(&serde_json::json!({ "query": query }), 5).is_err());
        }
        assert!(query_and_count(&serde_json::json!({}), 5).is_err());
        for (requested, expected) in [(0, 1), (1, 1), (5, 5), (10, 10), (u64::MAX, 10)] {
            let args = serde_json::json!({ "query": " 中文 Rust ", "count": requested });
            assert_eq!(query_and_count(&args, 5).unwrap(), ("中文 Rust", expected));
        }
        assert_eq!(
            query_and_count(&serde_json::json!({ "query": "Rust" }), 0)
                .unwrap()
                .1,
            1
        );
    }

    #[test]
    fn user_facing_search_diagnostics_only_expose_provider_details() {
        let header = format_search_result_header("Brave");
        let empty = format_empty_search_result(
            &["DuckDuckGo".to_string()],
            &[(
                "Brave".to_string(),
                "request failed with HTTP 429".to_string(),
            )],
        );

        assert_eq!(header, "Search results (via Brave)\n\n");
        assert_eq!(
            empty,
            "No results found by available providers.\n\nProviders with no results:\n- DuckDuckGo\nProviders unavailable or failed:\n- Brave: request failed with HTTP 429\n\nProvider failures or rate limits are not the same as the web having no results.\n"
        );
    }
}
