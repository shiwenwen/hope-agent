//! Isolated JavaScript renderer for `web_fetch`.
//!
//! This deliberately does not use the active BrowserBackend: that backend may
//! be the user's Chrome Extension and therefore carry cookies, tabs, and
//! credentials.  Each render launches a fresh incognito Chromium process,
//! intercepts every request, applies Hope's SSRF policy, captures the DOM, and
//! tears the process down.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::{
    SetDownloadBehaviorBehavior, SetDownloadBehaviorParams,
};
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams, EventRequestPaused, FailRequestParams, HeaderEntry,
    RequestPattern, RequestStage,
};
use chromiumoxide::cdp::browser_protocol::network::ErrorReason;
use chromiumoxide::cdp::browser_protocol::network::{EventDataReceived, ResourceType};
use chromiumoxide::cdp::browser_protocol::page::{FrameId, StopLoadingParams};
use futures_util::StreamExt;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use ha_core::browser_hooks::{WebFetchRenderRequest, WebFetchRenderResult};

fn render_slots() -> &'static Semaphore {
    static SLOTS: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(1))
}

struct AbortOnDrop(Option<JoinHandle<()>>);

impl AbortOnDrop {
    fn new(task: JoinHandle<()>) -> Self {
        Self(Some(task))
    }

    fn replace(&mut self, task: JoinHandle<()>) {
        self.abort();
        self.0 = Some(task);
    }

    fn abort(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.abort();
    }
}

fn network_url_requires_check(url: &str) -> bool {
    matches!(
        url::Url::parse(url)
            .ok()
            .map(|url| url.scheme().to_string())
            .as_deref(),
        Some("http" | "https")
    )
}

fn local_document_url(url: &str) -> bool {
    matches!(
        url::Url::parse(url)
            .ok()
            .map(|url| url.scheme().to_string())
            .as_deref(),
        Some("about" | "data" | "blob")
    )
}

async fn allow_request(request: &WebFetchRenderRequest, url: &str) -> bool {
    if local_document_url(url) {
        return true;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !network_url_requires_check(url)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return false;
    }
    ha_core::security::ssrf::check_url(parsed.as_str(), request.ssrf_policy, &request.trusted_hosts)
        .await
        .is_ok()
}

fn resource_needed_for_text(resource_type: &ResourceType) -> bool {
    matches!(
        resource_type,
        ResourceType::Document
            | ResourceType::Stylesheet
            | ResourceType::Script
            | ResourceType::Xhr
            | ResourceType::Fetch
            | ResourceType::Preflight
    )
}

fn method_is_read_only(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

fn is_top_level_document(
    resource_type: &ResourceType,
    frame_id: &FrameId,
    main_frame: &FrameId,
) -> bool {
    *resource_type == ResourceType::Document && frame_id == main_frame
}

fn declared_content_length(headers: Option<&[HeaderEntry]>) -> Option<usize> {
    headers?
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-length"))?
        .value
        .trim()
        .parse()
        .ok()
}

fn response_allows_snapshot_cache(headers: Option<&[HeaderEntry]>) -> bool {
    let Some(headers) = headers else {
        return true;
    };
    let sets_cookie = headers
        .iter()
        .any(|header| header.name.eq_ignore_ascii_case("set-cookie"));
    let restricted = headers.iter().any(|header| {
        header.name.eq_ignore_ascii_case("cache-control")
            && header.value.split(',').map(str::trim).any(|directive| {
                directive.eq_ignore_ascii_case("private")
                    || directive.eq_ignore_ascii_case("no-store")
                    || directive.split_once('=').is_some_and(|(name, _)| {
                        name.trim().eq_ignore_ascii_case("private")
                            || name.trim().eq_ignore_ascii_case("no-store")
                    })
            })
    });
    !sets_cookie && !restricted
}

/// Records decoded bytes while capping the counter at `limit + 1`. Returning
/// true tells the caller to stop all pending loads immediately. CDP reports
/// `dataLength` after content decoding, so compressed responses cannot hide
/// expansion behind a small wire length.
fn record_network_bytes(total: &AtomicUsize, chunk: usize, limit: usize) -> bool {
    let ceiling = limit.saturating_add(1);
    let previous = total
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(chunk).min(ceiling))
        })
        .unwrap_or_else(|current| current);
    previous.saturating_add(chunk) > limit
}

async fn wait_for_selector(
    page: &chromiumoxide::Page,
    selector: &str,
    deadline: Instant,
) -> Result<()> {
    let selector_json = serde_json::to_string(selector)?;
    loop {
        let found = page
            .evaluate(format!("Boolean(document.querySelector({selector_json}))"))
            .await
            .ok()
            .and_then(|value| value.into_value::<bool>().ok())
            .unwrap_or(false);
        if found {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("render selector did not appear before timeout"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn render(request: WebFetchRenderRequest) -> Result<WebFetchRenderResult> {
    let _slot = render_slots()
        .acquire()
        .await
        .map_err(|_| anyhow!("web fetch render pool unavailable"))?;
    let started = Instant::now();
    let timeout = Duration::from_millis(request.timeout_ms.clamp(1_000, 120_000));
    let managed = super::profile::resolve_profile(super::profile::BUILTIN_MANAGED)?;
    let executable = super::spawn::resolve_chrome_executable_for(
        managed.executable.as_deref(),
        "web_fetch renderer",
    )?;

    let mut builder = BrowserConfig::builder()
        .chrome_executable(executable)
        .incognito()
        .new_headless_mode()
        .enable_request_intercept()
        .disable_cache()
        .respect_https_errors()
        .launch_timeout(timeout)
        .request_timeout(timeout)
        .arg("--disable-background-networking")
        .arg("--disable-breakpad")
        .arg("--disable-component-update")
        .arg("--disable-default-apps")
        .arg("--disable-features=AutofillServerCommunication,BackgroundFetch,BackgroundSync,MediaRouter,Notifications,OptimizationHints,PeriodicBackgroundSync,PushMessaging,ServiceWorker")
        .arg("--disable-notifications")
        .arg("--disable-sync")
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    if super::profile::deployment_is_docker() {
        builder = builder.no_sandbox();
    }
    let config = builder
        .build()
        .map_err(|error| anyhow!("build isolated renderer config: {error}"))?;
    let (mut browser, mut handler) = Browser::launch(config)
        .await
        .map_err(|error| anyhow!("launch isolated renderer: {error}"))?;
    let mut handler_task = AbortOnDrop::new(tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    }));

    // These guards abort background tasks even when the caller cancels this
    // future while navigation or DOM extraction is still in progress.
    let mut interception_task = AbortOnDrop(None);
    let mut network_budget_task = AbortOnDrop(None);
    let rendered = async {
        browser
            .execute(SetDownloadBehaviorParams::new(
                SetDownloadBehaviorBehavior::Deny,
            ))
            .await?;
        let page = Arc::new(browser.new_page("about:blank").await?);
        page.add_init_script(
            "Object.defineProperty(window, 'open', { configurable: false, writable: false, value: () => null });",
        )
        .await?;
        let main_frame = page
            .mainframe()
            .await?
            .ok_or_else(|| anyhow!("isolated renderer has no main frame"))?;
        let mut paused = page.event_listener::<EventRequestPaused>().await?;
        let mut received = page.event_listener::<EventDataReceived>().await?;
        page.execute(
            EnableParams::builder()
                .patterns([
                    RequestPattern::builder()
                        .url_pattern("*")
                        .request_stage(RequestStage::Request)
                        .build(),
                    RequestPattern::builder()
                        .url_pattern("*")
                        .request_stage(RequestStage::Response)
                        .build(),
                ])
                .handle_auth_requests(false)
                .build(),
        )
        .await?;
        let intercept_page = page.clone();
        let intercept_request = request.clone();
        let response_byte_limit = request.max_html_bytes;
        let document_status = Arc::new(AtomicU16::new(0));
        let intercept_status = document_status.clone();
        let cacheable = Arc::new(AtomicBool::new(true));
        let intercept_cacheable = cacheable.clone();
        let admitted_bytes = Arc::new(AtomicUsize::new(0));
        let budget_exceeded = Arc::new(AtomicBool::new(false));
        let budget_page = page.clone();
        let budget_bytes = admitted_bytes.clone();
        let budget_flag = budget_exceeded.clone();
        network_budget_task.replace(tokio::spawn(async move {
            while let Some(event) = received.next().await {
                let decoded = usize::try_from(event.data_length.max(0)).unwrap_or(usize::MAX);
                let encoded =
                    usize::try_from(event.encoded_data_length.max(0)).unwrap_or(usize::MAX);
                if record_network_bytes(&budget_bytes, decoded.max(encoded), response_byte_limit) {
                    budget_flag.store(true, Ordering::Relaxed);
                    let _ = budget_page.execute(StopLoadingParams::default()).await;
                    break;
                }
            }
        }));
        let intercept_budget_flag = budget_exceeded.clone();
        interception_task.replace(tokio::spawn(async move {
            while let Some(event) = paused.next().await {
                if event.response_status_code.is_some() {
                    if intercept_budget_flag.load(Ordering::Relaxed)
                        || declared_content_length(event.response_headers.as_deref())
                            .is_some_and(|length| length > response_byte_limit)
                    {
                        intercept_budget_flag.store(true, Ordering::Relaxed);
                        let _ = intercept_page
                            .execute(FailRequestParams::new(
                                event.request_id.clone(),
                                ErrorReason::Aborted,
                            ))
                            .await;
                        let _ = intercept_page.execute(StopLoadingParams::default()).await;
                        continue;
                    }
                    if is_top_level_document(&event.resource_type, &event.frame_id, &main_frame) {
                        if let Some(status) = event.response_status_code {
                            intercept_status.store(status as u16, Ordering::Relaxed);
                        }
                        if !response_allows_snapshot_cache(event.response_headers.as_deref()) {
                            intercept_cacheable.store(false, Ordering::Relaxed);
                        }
                    }
                    let _ = intercept_page
                        .execute(ContinueRequestParams::new(event.request_id.clone()))
                        .await;
                    continue;
                }
                if !intercept_budget_flag.load(Ordering::Relaxed)
                    && method_is_read_only(&event.request.method)
                    && resource_needed_for_text(&event.resource_type)
                    && allow_request(&intercept_request, &event.request.url).await
                {
                    let _ = intercept_page
                        .execute(ContinueRequestParams::new(event.request_id.clone()))
                        .await;
                } else {
                    let _ = intercept_page
                        .execute(FailRequestParams::new(
                            event.request_id.clone(),
                            ErrorReason::Aborted,
                        ))
                        .await;
                }
            }
        }));

        let navigation = page.goto(request.url.as_str()).await;
        if budget_exceeded.load(Ordering::Relaxed) {
            return Err(anyhow!(
                "rendered responses exceed the configured cumulative byte limit"
            ));
        }
        navigation?;
        let deadline = Instant::now() + timeout.saturating_sub(Duration::from_millis(250));
        if let Some(selector) = request.selector.as_deref() {
            wait_for_selector(&page, selector, deadline).await?;
        } else {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if budget_exceeded.load(Ordering::Relaxed) {
            return Err(anyhow!(
                "rendered responses exceed the configured cumulative byte limit"
            ));
        }

        let final_url = page.url().await?.unwrap_or_else(|| request.url.clone());
        ha_core::security::ssrf::check_url(&final_url, request.ssrf_policy, &request.trusted_hosts)
            .await?;
        let title = page
            .evaluate("document.title || ''")
            .await
            .ok()
            .and_then(|value| value.into_value::<String>().ok())
            .filter(|value| !value.trim().is_empty());
        let dom_bytes = page
            .evaluate(
                "new TextEncoder().encode(document.documentElement?.outerHTML || '').byteLength",
            )
            .await?
            .into_value::<usize>()?;
        if dom_bytes > request.max_html_bytes {
            return Err(anyhow!("rendered DOM exceeds the configured byte limit"));
        }
        let html = page.content().await?;
        if html.len() > request.max_html_bytes {
            return Err(anyhow!("rendered DOM exceeds the configured byte limit"));
        }
        if budget_exceeded.load(Ordering::Relaxed) {
            return Err(anyhow!(
                "rendered responses exceed the configured cumulative byte limit"
            ));
        }
        Ok(WebFetchRenderResult {
            final_url,
            status: match document_status.load(Ordering::Relaxed) {
                0 => None,
                status => Some(status),
            },
            title,
            html,
            received_bytes: admitted_bytes.load(Ordering::Relaxed),
            cacheable: cacheable.load(Ordering::Relaxed),
            took_ms: started.elapsed().as_millis() as u64,
        })
    };

    let result = match tokio::time::timeout(timeout, rendered).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!("isolated renderer timed out")),
    };
    interception_task.abort();
    network_budget_task.abort();
    let _ = browser.close().await;
    handler_task.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_only_allows_network_and_local_document_schemes() {
        assert!(network_url_requires_check("https://example.com"));
        assert!(network_url_requires_check("http://example.com"));
        assert!(local_document_url("about:blank"));
        assert!(local_document_url("data:text/plain,hello"));
        assert!(!network_url_requires_check("file:///etc/passwd"));
        assert!(!local_document_url("file:///etc/passwd"));
    }

    #[test]
    fn renderer_blocks_non_text_page_resources() {
        for resource in [
            ResourceType::Document,
            ResourceType::Stylesheet,
            ResourceType::Script,
            ResourceType::Xhr,
            ResourceType::Fetch,
            ResourceType::Preflight,
        ] {
            assert!(resource_needed_for_text(&resource));
        }
        for resource in [
            ResourceType::Image,
            ResourceType::Media,
            ResourceType::Font,
            ResourceType::WebSocket,
        ] {
            assert!(!resource_needed_for_text(&resource));
        }
    }

    #[test]
    fn renderer_allows_only_read_only_http_methods() {
        for method in ["GET", "get", "HEAD", "OPTIONS"] {
            assert!(method_is_read_only(method));
        }
        for method in ["POST", "PUT", "PATCH", "DELETE", "CONNECT"] {
            assert!(!method_is_read_only(method));
        }
    }

    #[test]
    fn renderer_tracks_status_only_for_the_main_frame_document() {
        let main = FrameId::new("main");
        let iframe = FrameId::new("iframe");
        assert!(is_top_level_document(&ResourceType::Document, &main, &main));
        assert!(!is_top_level_document(
            &ResourceType::Document,
            &iframe,
            &main
        ));
        assert!(!is_top_level_document(&ResourceType::Script, &main, &main));
    }

    #[test]
    fn renderer_rejects_declared_oversized_responses() {
        let headers = [HeaderEntry::new("Content-Length", "2048")];
        assert_eq!(declared_content_length(Some(&headers)), Some(2_048));
        assert!(declared_content_length(Some(&headers)).is_some_and(|length| length > 1_024));
    }

    #[test]
    fn renderer_enforces_cumulative_decoded_response_budget() {
        let total = AtomicUsize::new(0);
        assert!(!record_network_bytes(&total, 400, 1_024));
        assert!(!record_network_bytes(&total, 600, 1_024));
        assert!(record_network_bytes(&total, 25, 1_024));
        assert_eq!(total.load(Ordering::Relaxed), 1_025);
    }

    #[test]
    fn renderer_detects_private_top_level_responses() {
        assert!(!response_allows_snapshot_cache(Some(&[HeaderEntry::new(
            "Cache-Control",
            "public, no-store"
        )])));
        assert!(!response_allows_snapshot_cache(Some(&[HeaderEntry::new(
            "Set-Cookie",
            "session=private"
        )])));
        assert!(response_allows_snapshot_cache(Some(&[HeaderEntry::new(
            "Cache-Control",
            "public, max-age=60"
        )])));
    }

    #[tokio::test]
    async fn renderer_rejects_url_credentials_before_navigation() {
        let request = WebFetchRenderRequest {
            url: "https://example.com".to_string(),
            selector: None,
            timeout_ms: 1_000,
            max_html_bytes: 1_024,
            ssrf_policy: ha_core::security::ssrf::SsrfPolicy::Strict,
            trusted_hosts: vec!["example.com".to_string()],
        };
        assert!(allow_request(&request, "https://example.com/public").await);
        assert!(!allow_request(&request, "https://user:password@example.com/private").await);
    }
}
