//! Native keyless search, inspired by DDGS's multi-engine fallback.
//! This does not run or bundle the Python DDGS package.

use anyhow::{bail, Result};
use scraper::{Html, Selector};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::helpers::{
    brave_freshness, build_search_client_for_url, read_text_capped, status_error,
    HTML_RESPONSE_BYTE_CAP,
};
use super::{SearchParams, SearchResult, WebSearchUsageContext};

static BRAVE_COOLDOWN: AtomicU64 = AtomicU64::new(0);
static SO_COOLDOWN: AtomicU64 = AtomicU64::new(0);
const COOLDOWN_SECS: u64 = 60;

#[derive(Clone, Copy)]
enum Engine {
    Brave,
    So,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Self::Brave => "Brave Web",
            Self::So => "360 Search",
        }
    }

    fn cooldown(self) -> &'static AtomicU64 {
        match self {
            Self::Brave => &BRAVE_COOLDOWN,
            Self::So => &SO_COOLDOWN,
        }
    }
}

pub(super) async fn search_keyless(
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
    usage_ctx: &WebSearchUsageContext,
) -> Result<Vec<SearchResult>> {
    try_engines(
        timeout_secs,
        eligible_engines(params),
        |engine, budget| async move {
            let started = Instant::now();
            let result = within_timeout(
                budget,
                search_engine(engine, query, count, params, budget.as_secs().max(1)),
            )
            .await;
            let mut event =
                crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_WEB_SEARCH);
            event.operation = Some("keyless_search".into());
            event.source = Some("web_search".into());
            event.provider_id = Some("keyless".into());
            event.provider_name = Some(engine.name().into());
            event.session_id = usage_ctx.session_id.clone();
            event.agent_id = usage_ctx.agent_id.clone();
            event.duration_ms = Some(started.elapsed().as_millis() as u64);
            event.success = result.as_ref().is_ok_and(|results| !results.is_empty());
            event.error = result.as_ref().err().map(ToString::to_string);
            // No query text, upstream content, credentials, or estimated tokens.
            event.metadata = Some(serde_json::json!({ "engine": engine.name() }));
            crate::model_usage::record_model_usage_best_effort(event);
            result
        },
    )
    .await
}

async fn within_timeout<Fut>(budget: Duration, attempt: Fut) -> Result<Vec<SearchResult>>
where
    Fut: Future<Output = Result<Vec<SearchResult>>>,
{
    tokio::time::timeout(budget, attempt)
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("request timed out")))
}

fn eligible_engines(params: &SearchParams) -> &'static [Engine] {
    if params.freshness.is_some() {
        &[Engine::Brave]
    } else {
        &[Engine::Brave, Engine::So]
    }
}

async fn try_engines<F, Fut>(
    timeout_secs: u64,
    engines: &[Engine],
    mut attempt: F,
) -> Result<Vec<SearchResult>>
where
    F: FnMut(Engine, Duration) -> Fut,
    Fut: Future<Output = Result<Vec<SearchResult>>>,
{
    let started = Instant::now();
    let total = Duration::from_secs(timeout_secs.max(1));
    let mut failures = Vec::new();
    for (index, &engine) in engines.iter().enumerate() {
        let remaining = total.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            failures.push("search timeout budget exhausted".to_string());
            break;
        }
        // Reserve time for the next engine rather than letting the first use
        // the entire provider timeout. A successful first engine stops here.
        let budget = remaining / (engines.len() - index) as u32;
        match attempt(engine, budget).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => failures.push(format!("{}: no results", engine.name())),
            Err(error) => {
                app_warn!(
                    "tool",
                    "web_search",
                    "Keyless engine [{}] failed: {}, trying next engine",
                    engine.name(),
                    error
                );
                failures.push(format!("{}: {}", engine.name(), error));
            }
        }
    }
    bail!("Keyless search unavailable: {}", failures.join("; "))
}

async fn search_engine(
    engine: Engine,
    query: &str,
    count: usize,
    params: &SearchParams,
    timeout_secs: u64,
) -> Result<Vec<SearchResult>> {
    if epoch_secs() < engine.cooldown().load(Ordering::Relaxed) {
        bail!("rate-limit cooldown active");
    }
    let url = search_url(engine, query, params)?;
    let cfg = crate::config::cached_config();
    let response = crate::security::http_redirect::checked_get_with_client_factory(
        url.as_str(),
        cfg.ssrf.default_policy,
        &cfg.ssrf.trusted_hosts,
        5,
        |target| build_search_client_for_url(target.as_str(), timeout_secs),
    )
    .await?
    .response;
    let status = response.status();
    if matches!(status.as_u16(), 403 | 429) {
        engine
            .cooldown()
            .store(epoch_secs() + COOLDOWN_SECS, Ordering::Relaxed);
    }
    if !status.is_success() {
        return Err(status_error(engine.name(), status));
    }
    let body = read_text_capped(response, HTML_RESPONSE_BYTE_CAP)
        .await
        .map_err(|_| anyhow::anyhow!("response body could not be read"))?;
    match engine {
        Engine::Brave => parse_brave(&body, count),
        Engine::So => parse_so(&body, count),
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn search_url(engine: Engine, query: &str, params: &SearchParams) -> Result<url::Url> {
    let mut url = url::Url::parse(match engine {
        Engine::Brave => "https://search.brave.com/search",
        Engine::So => "https://www.so.com/s",
    })?;
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("q", query);
    match engine {
        Engine::Brave => {
            pairs.append_pair("source", "web");
            if let Some(country) = &params.country {
                pairs.append_pair("country", country);
            }
            if let Some(language) = &params.language {
                pairs.append_pair("search_lang", language);
            }
            if let Some(freshness) = &params.freshness {
                pairs.append_pair("tf", brave_freshness(freshness));
            }
        }
        Engine::So => {
            // This HTML endpoint has no verified freshness-filter contract.
            // Do not silently relax an explicit time filter during fallback.
            if params.freshness.is_some() {
                bail!("360 Search does not support freshness filters");
            }
        }
    }
    drop(pairs);
    Ok(url)
}

fn valid_result_url(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn parse_brave(body: &str, count: usize) -> Result<Vec<SearchResult>> {
    let html = Html::parse_document(body);
    let items = Selector::parse("div[data-type='web']").unwrap();
    let links = Selector::parse("a").unwrap();
    let titles = Selector::parse("div.title").unwrap();
    let snippets = Selector::parse(".snippet .content, .content").unwrap();
    let mut seen = std::collections::HashSet::new();
    let results: Vec<_> = html
        .select(&items)
        .filter_map(|item| {
            let link = item
                .select(&links)
                .find(|link| link.select(&titles).next().is_some())?;
            let url = link.value().attr("href")?;
            let title = link.select(&titles).next()?.text().collect::<String>();
            if title.trim().is_empty() || !valid_result_url(url) || !seen.insert(url.to_string()) {
                return None;
            }
            Some(SearchResult {
                title: title.trim().to_string(),
                url: url.to_string(),
                snippet: item
                    .select(&snippets)
                    .next()
                    .map(|node| node.text().collect::<String>())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                source: "Brave Web".into(),
            })
        })
        .take(count)
        .collect();
    if results.is_empty() {
        bail!("no usable results (page may be blocked or its format changed)");
    }
    Ok(results)
}

fn parse_so(body: &str, count: usize) -> Result<Vec<SearchResult>> {
    let html = Html::parse_document(body);
    let items = Selector::parse("li.res-list").unwrap();
    let links = Selector::parse("h3.res-title a").unwrap();
    let summaries = Selector::parse(".res-list-summary").unwrap();
    let mut seen = std::collections::HashSet::new();
    let results: Vec<_> = html
        .select(&items)
        .filter_map(|item| {
            let link = item.select(&links).next()?;
            // Prefer the actual destination supplied by the search page.
            // Never expose opaque so.com tracking links as source URLs.
            let url = link
                .value()
                .attr("data-mdurl")
                .or_else(|| link.value().attr("href"))?;
            let title = link.text().collect::<String>();
            if title.trim().is_empty()
                || !valid_result_url(url)
                || url::Url::parse(url)
                    .ok()?
                    .host_str()
                    .is_some_and(|host| host == "so.com" || host.ends_with(".so.com"))
                || !seen.insert(url.to_string())
            {
                return None;
            }
            Some(SearchResult {
                title: title.trim().to_string(),
                url: url.to_string(),
                snippet: item
                    .select(&summaries)
                    .next()
                    .map(|node| node.text().collect::<String>())
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                source: "360 Search".into(),
            })
        })
        .take(count)
        .collect();
    if results.is_empty() {
        bail!("no usable results (page may be blocked or its format changed)");
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brave_extracts_web_results_and_rejects_unsafe_links() {
        let body = r#"<div data-type="web"><a href="https://rust-lang.org/"><div class="title">Rust &amp; tools</div></a><div class="content">Build <b>reliable</b> software</div></div>
            <div data-type="web"><a href="javascript:alert(1)"><div class="title">Bad</div></a></div>
            <div data-type="web"><a href="https://rust-lang.org/"><div class="title">Duplicate</div></a></div>"#;
        let results = parse_brave(body, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust & tools");
        assert_eq!(results[0].snippet, "Build reliable software");
        assert!(parse_brave("<html>Challenge</html>", 5).is_err());
    }

    #[test]
    fn so_uses_destination_urls_and_deduplicates() {
        let body = r#"<li class="res-list"><h3 class="res-title"><a href="https://www.so.com/link?m=tracking" data-mdurl="https://example.com/?a=1&amp;b=2">Rust &amp; 中文</a></h3><span class="res-list-summary">Build <b>software</b></span></li>
            <li class="res-list"><h3 class="res-title"><a data-mdurl="https://example.com/?a=1&amp;b=2">Duplicate</a></h3></li>
            <li class="res-list"><h3 class="res-title"><a data-mdurl="https://secret@example.com/">Credentials</a></h3></li>
            <li class="res-list"><h3 class="res-title"><a href="https://www.so.com/link?m=tracking">Opaque link</a></h3></li>"#;
        let results = parse_so(body, 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust & 中文");
        assert_eq!(results[0].url, "https://example.com/?a=1&b=2");
        assert_eq!(results[0].snippet, "Build software");
        assert!(parse_so("<html>Challenge</html>", 5).is_err());
    }

    #[tokio::test]
    async fn failed_first_engine_falls_back_with_reserved_budget() {
        let results = try_engines(
            4,
            eligible_engines(&SearchParams::default()),
            |engine, budget| async move {
                match engine {
                    Engine::Brave => {
                        assert!(budget <= Duration::from_secs(2));
                        bail!("HTTP 429")
                    }
                    Engine::So => Ok(vec![SearchResult {
                        title: "Result".into(),
                        url: "https://example.com".into(),
                        snippet: String::new(),
                        source: engine.name().into(),
                    }]),
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(results[0].source, "360 Search");
        let error = try_engines(
            4,
            eligible_engines(&SearchParams::default()),
            |_, _| async { Ok(Vec::new()) },
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Brave Web: no results"));
        assert!(error.to_string().contains("360 Search: no results"));
    }

    #[tokio::test]
    async fn timed_out_first_engine_leaves_time_for_fallback() {
        let results = try_engines(
            1,
            eligible_engines(&SearchParams::default()),
            |engine, budget| async move {
                within_timeout(budget, async move {
                    match engine {
                        Engine::Brave => std::future::pending().await,
                        Engine::So => Ok(vec![SearchResult {
                            title: "Fallback".into(),
                            url: "https://example.com".into(),
                            snippet: String::new(),
                            source: engine.name().into(),
                        }]),
                    }
                })
                .await
            },
        )
        .await
        .unwrap();
        assert_eq!(results[0].source, "360 Search");
    }

    #[tokio::test]
    async fn success_stops_before_contacting_the_other_engine() {
        let results = try_engines(
            1,
            eligible_engines(&SearchParams::default()),
            |engine, _| async move {
                assert!(matches!(engine, Engine::Brave));
                Ok(vec![SearchResult {
                    title: "Primary".into(),
                    url: "https://example.com".into(),
                    snippet: String::new(),
                    source: engine.name().into(),
                }])
            },
        )
        .await
        .unwrap();
        assert_eq!(results[0].source, "Brave Web");
    }

    #[tokio::test]
    async fn freshness_filters_reserve_the_full_budget_for_brave() {
        let params = SearchParams {
            freshness: Some("week".into()),
            ..SearchParams::default()
        };
        let error = try_engines(4, eligible_engines(&params), |engine, budget| async move {
            assert!(
                matches!(engine, Engine::Brave),
                "Ineligible fallback was attempted"
            );
            assert!(
                budget > Duration::from_secs(3),
                "Eligible engine lost half its budget"
            );
            bail!("HTTP 429")
        })
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Brave Web: HTTP 429"));
        assert!(!error.to_string().contains("360 Search"));
    }

    #[test]
    fn only_http_links_without_credentials_are_returned() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,secret",
            "file:///etc/passwd",
            "//example.com/",
            "https://secret:token@example.com/",
            "",
        ] {
            assert!(!valid_result_url(url), "{url}");
        }
        for url in ["https://example.com/中文?q=1#section", "http://example.com"] {
            assert!(valid_result_url(url));
        }
        let html = r#"<div data-type="web"><a href="https://example.com"><div class="title">First</div></a></div>
            <div data-type="web"><a href="https://example.org"><div class="title">Second</div></a></div>"#;
        assert_eq!(parse_brave(html, 1).unwrap().len(), 1);
        assert!(parse_so("<html>No results</html>", 5).is_err());
    }

    #[test]
    fn query_is_encoded_and_fallback_preserves_explicit_time_filters() {
        let params = SearchParams {
            freshness: Some("week".into()),
            ..SearchParams::default()
        };
        let url = search_url(Engine::Brave, "中文 & source=other", &params).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs.get("q").unwrap(), "中文 & source=other");
        assert_eq!(pairs.get("source").unwrap(), "web");
        assert_eq!(pairs.get("tf").unwrap(), "pw");
        assert!(search_url(Engine::So, "query", &params).is_err());
    }

    #[tokio::test]
    #[ignore = "Public network smoke test; run explicitly with isolated HA_DATA_DIR"]
    async fn live_keyless_search() {
        assert!(std::env::var_os("HA_DATA_DIR").is_some());
        let output = super::super::tool_web_search(
            &serde_json::json!({ "query": "Rust programming language", "count": 5 }),
            &crate::tool_defs::ToolExecContext::default(),
        )
        .await
        .unwrap();
        assert!(output.starts_with("Search results (via Keyless)"));
        assert!(output.contains("rust-lang.org"));
        let count = output.matches("   URL: ").count();
        assert!((1..=5).contains(&count));
        let source = output
            .lines()
            .find(|line| line.starts_with("   Source: "))
            .unwrap();
        println!("{count} results; {}", source.trim());
    }

    #[tokio::test]
    #[ignore = "Bilingual public network probes; run explicitly with isolated HA_DATA_DIR"]
    async fn live_keyless_search_matrix() {
        assert!(std::env::var_os("HA_DATA_DIR").is_some());
        for (label, query, count, country, language, expected_domain) in [
            (
                "rust",
                "Rust programming language",
                1,
                "US",
                "en",
                "rust-lang.org",
            ),
            (
                "python",
                "Python programming language official",
                10,
                "US",
                "en",
                "python.org",
            ),
            (
                "chinese",
                "北京 故宫博物院 官方网站",
                5,
                "CN",
                "zh",
                "dpm.org.cn",
            ),
            (
                "site-filter",
                "site:docs.rs tokio timeout",
                5,
                "US",
                "en",
                "docs.rs",
            ),
            (
                "symbols",
                "site:cppreference.com C++ std::vector",
                5,
                "US",
                "en",
                "cppreference.com",
            ),
        ] {
            let started = Instant::now();
            let output = super::super::tool_web_search(
                &serde_json::json!({
                    "query": query, "count": count, "country": country, "language": language,
                }),
                &crate::tool_defs::ToolExecContext::default(),
            )
            .await
            .unwrap();
            let actual_count = output.matches("   URL: ").count();
            assert!((1..=count).contains(&actual_count), "{label}: no results");
            assert!(
                output
                    .lines()
                    .filter_map(|line| line.strip_prefix("   URL: "))
                    .filter_map(|raw| url::Url::parse(raw).ok())
                    .any(|url| url.host_str().is_some_and(|host| {
                        host == expected_domain || host.ends_with(&format!(".{expected_domain}"))
                    })),
                "{label}: expected relevant source missing"
            );
            let source = output
                .lines()
                .find(|line| line.starts_with("   Source: "))
                .unwrap();
            println!(
                "{label}: {actual_count} results; {}; {} ms",
                source.trim(),
                started.elapsed().as_millis()
            );
        }
    }
}
