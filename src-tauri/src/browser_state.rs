use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use futures_util::StreamExt;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;

// ── Element Reference (from snapshot) ────────────────────────────

#[derive(Debug, Clone)]
pub struct ElementRef {
    pub ref_id: u32,
    pub role: String,
    pub text: String,
    /// Unique CSS selector for re-finding the element
    pub selector: String,
    /// Bounding box center X
    pub center_x: f64,
    /// Bounding box center Y
    pub center_y: f64,
    /// Extra attributes (href, value, placeholder, etc.)
    pub attrs: HashMap<String, String>,
}

// ── Browser State ────────────────────────────────────────────────

pub struct BrowserState {
    /// The chromiumoxide Browser handle
    pub browser: Option<Browser>,
    /// Browser event handler task
    handler_task: Option<JoinHandle<()>>,
    /// Cached page handles by target_id
    pub pages: HashMap<String, Page>,
    /// Currently active tab/page target ID
    pub active_page_id: Option<String>,
    /// Element refs from the most recent snapshot
    pub element_refs: Vec<ElementRef>,
    /// URL when the snapshot was taken (for staleness detection)
    pub snapshot_url: Option<String>,
    /// Connection URL (for reconnection)
    pub connection_url: Option<String>,
}

impl BrowserState {
    fn new() -> Self {
        Self {
            browser: None,
            handler_task: None,
            pages: HashMap::new(),
            active_page_id: None,
            element_refs: Vec::new(),
            snapshot_url: None,
            connection_url: None,
        }
    }

    /// Connect to an already-running Chrome instance via CDP
    pub async fn connect(&mut self, debug_url: &str) -> anyhow::Result<()> {
        // First, discover the WebSocket debugger URL from /json/version
        let ws_url = discover_ws_url(debug_url).await?;

        app_info!("browser", "cdp", "Connecting to Chrome at {}", ws_url);

        let (browser, mut handler) = Browser::connect(&ws_url).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Chrome at {}: {}. Make sure Chrome is running with --remote-debugging-port", debug_url, e))?;

        // Spawn the handler task — drives the CDP event loop.
        // CRITICAL: This must keep running for the entire browser session.
        // Do NOT break on errors — only exit when the stream ends (returns None).
        let handle = tokio::spawn(async move {
            loop {
                match handler.next().await {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        app_warn!("browser", "cdp", "CDP handler error (continuing): {}", e);
                        // Don't break — transient errors are normal in the CDP stream
                    }
                    None => {
                        app_info!("browser", "cdp", "CDP handler stream ended");
                        break;
                    }
                }
            }
        });

        self.browser = Some(browser);
        self.handler_task = Some(handle);
        self.connection_url = Some(debug_url.to_string());

        // Brief yield to let the handler task start processing
        tokio::task::yield_now().await;

        // Refresh page list
        self.refresh_pages().await?;

        Ok(())
    }

    /// Launch a new managed Chrome instance
    pub async fn launch(&mut self, executable_path: Option<&str>, headless: bool) -> anyhow::Result<()> {
        let mut config = BrowserConfig::builder();

        if let Some(path) = executable_path {
            config = config.chrome_executable(path);
        }

        if headless {
            config = config.arg("--headless=new");
        }

        // Common args for stability
        config = config
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking");

        let config = config.build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {}", e))?;

        let (browser, mut handler) = Browser::launch(config).await
            .map_err(|e| anyhow::anyhow!("Failed to launch Chrome: {}. Make sure Chrome/Chromium is installed.", e))?;

        let handle = tokio::spawn(async move {
            loop {
                match handler.next().await {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        app_warn!("browser", "cdp", "CDP handler error (continuing): {}", e);
                    }
                    None => {
                        app_info!("browser", "cdp", "CDP handler stream ended (launch)");
                        break;
                    }
                }
            }
        });

        self.browser = Some(browser);
        self.handler_task = Some(handle);
        self.connection_url = None;

        tokio::task::yield_now().await;

        // Refresh page list
        self.refresh_pages().await?;

        Ok(())
    }

    /// Disconnect from the browser and clean up resources
    pub async fn disconnect(&mut self) {
        self.pages.clear();
        self.active_page_id = None;
        self.element_refs.clear();
        self.snapshot_url = None;
        self.connection_url = None;

        // Drop the browser (closes the CDP connection)
        self.browser.take();

        // Abort the handler task
        if let Some(handle) = self.handler_task.take() {
            handle.abort();
        }

        app_info!("browser", "cdp", "Browser disconnected");
    }

    /// Check if connected to a browser (browser exists AND handler is still running)
    pub fn is_connected(&self) -> bool {
        self.browser.is_some()
            && self.handler_task.as_ref().map_or(false, |h| !h.is_finished())
    }

    /// Refresh the page list from the browser
    pub async fn refresh_pages(&mut self) -> anyhow::Result<()> {
        let browser = self.browser.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to browser"))?;

        let pages = browser.pages().await
            .map_err(|e| anyhow::anyhow!("Failed to list pages: {}", e))?;

        self.pages.clear();
        for page in pages {
            let target_id = page.target_id().as_ref().to_string();
            self.pages.insert(target_id.clone(), page);

            // Auto-select first page if none selected
            if self.active_page_id.is_none() {
                self.active_page_id = Some(target_id);
            }
        }

        Ok(())
    }

    /// Get the active page handle
    pub fn get_active_page(&self) -> anyhow::Result<&Page> {
        let page_id = self.active_page_id.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No active page. Use 'new_page' or 'select_page' first."))?;

        self.pages.get(page_id)
            .ok_or_else(|| anyhow::anyhow!("Active page {} no longer exists. Use 'list_pages' to see available pages.", page_id))
    }

    /// Find an element ref by ref_id
    pub fn find_ref(&self, ref_id: u32) -> anyhow::Result<&ElementRef> {
        self.element_refs.iter()
            .find(|r| r.ref_id == ref_id)
            .ok_or_else(|| {
                let available: Vec<u32> = self.element_refs.iter().map(|r| r.ref_id).collect();
                anyhow::anyhow!(
                    "Element ref={} not found. Available refs: {}. Use 'take_snapshot' to refresh element references.",
                    ref_id,
                    if available.len() > 20 { format!("{:?}...({})", &available[..20], available.len()) }
                    else { format!("{:?}", available) }
                )
            })
    }
}

// ── Global Singleton ─────────────────────────────────────────────

static BROWSER_STATE: OnceLock<Mutex<BrowserState>> = OnceLock::new();

pub fn get_browser_state() -> &'static Mutex<BrowserState> {
    BROWSER_STATE.get_or_init(|| Mutex::new(BrowserState::new()))
}

/// Auto-connect to Chrome if not already connected (tries localhost:9222)
pub async fn ensure_connected() -> anyhow::Result<()> {
    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        return Ok(());
    }

    // Clean up stale connection if handler died
    if state.browser.is_some() {
        app_info!("browser", "cdp", "Cleaning up stale browser connection (handler died)");
        state.disconnect().await;
    }

    state.connect("http://localhost:9222").await.map_err(|_| {
        anyhow::anyhow!(
            "Browser not connected. Please either:\n\
             1. Launch Chrome with: chrome --remote-debugging-port=9222\n\
             2. Use action=\"launch\" to start a managed Chrome instance\n\
             3. Use action=\"connect\" with a custom URL"
        )
    })
}

// ── Helper: Discover WebSocket URL ───────────────────────────────

/// Fetch the WebSocket debugger URL from Chrome's /json/version endpoint
async fn discover_ws_url(base_url: &str) -> anyhow::Result<String> {
    let version_url = format!("{}/json/version", base_url.trim_end_matches('/'));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let resp = client.get(&version_url).send().await
        .map_err(|e| anyhow::anyhow!(
            "Cannot reach Chrome at {}. Is Chrome running with --remote-debugging-port? Error: {}",
            base_url, e
        ))?;

    let body: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Invalid response from Chrome: {}", e))?;

    body.get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("Chrome did not return webSocketDebuggerUrl. Response: {}", body))
}
