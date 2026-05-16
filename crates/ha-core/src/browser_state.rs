use chromiumoxide::browser::Browser;
use chromiumoxide::Page;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::browser::spawn::LaunchSpec;

// ── Element Reference (from snapshot) ────────────────────────────

#[derive(Debug, Clone)]
pub struct ElementRef {
    pub ref_id: u32,
    pub role: String,
    pub text: String,
    /// Unique CSS selector for re-finding the element
    pub selector: String,
    /// Bounding box center X
    #[allow(dead_code)]
    pub center_x: f64,
    /// Bounding box center Y
    #[allow(dead_code)]
    pub center_y: f64,
    /// Extra attributes (href, value, placeholder, etc.)
    #[allow(dead_code)]
    pub attrs: HashMap<String, String>,
}

// ── Browser State ────────────────────────────────────────────────

pub struct BrowserState {
    /// The chromiumoxide Browser handle (CDP client side). `Arc`-wrapped so
    /// the heartbeat task can clone a reference and call `version()` without
    /// holding the global state mutex — `Browser::version` only touches
    /// `self.sender` (a `futures::channel::mpsc::Sender`), no `Child`
    /// dependency.
    pub browser: Option<Arc<Browser>>,
    /// Browser event handler task.
    handler_task: Option<JoinHandle<()>>,
    /// The Chrome process we spawned. Owned so we can kill / wait on
    /// disconnect; absent when we attached to an already-running Chrome
    /// (`profile.op=connect` against an external `--remote-debugging-port`).
    /// Source of truth for "did we launch this Chrome?".
    chrome_child: Option<Child>,
    /// Heartbeat keepalive task — see [`spawn_heartbeat`].
    heartbeat_task: Option<JoinHandle<()>>,
    /// Cached page handles by target_id.
    pub pages: HashMap<String, Page>,
    /// Currently active tab/page target ID.
    pub active_page_id: Option<String>,
    /// Element refs from the most recent snapshot.
    pub element_refs: Vec<ElementRef>,
    /// URL when the snapshot was taken (for staleness detection).
    pub snapshot_url: Option<String>,
    /// Connection URL (for reconnection / UI display).
    pub connection_url: Option<String>,
    /// Active browser profile name (None = default Chrome profile).
    pub profile: Option<String>,
    /// Set to `false` by the heartbeat task when `browser.version()` fails
    /// or times out, indicating the ws transport is permanently dead. Read
    /// by [`Self::is_connected`] so the next tool call's
    /// `ensure_connected_or_launch_managed` triggers a fresh spawn.
    transport_alive: Arc<AtomicBool>,
}

/// Default heartbeat interval. Chrome's WebSocket idle timeout is ~4 minutes,
/// so 120s gives us at least two pings before idle close.
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 120;

/// Long-lived runtime dedicated to browser background tasks (CDP handler
/// loop + heartbeat). The reason this exists is subtle but critical: tool
/// calls run via [`crate::async_jobs::spawn::dispatch_with_auto_background`],
/// which spawns a fresh `current_thread` tokio runtime per tool invocation
/// and drops it the moment the tool returns. If `Browser::connect`'s handler
/// task is `tokio::spawn`'d from that ephemeral runtime, the runtime drop
/// silently cancels the task — `JoinHandle::is_finished()` flips to true,
/// any wrapper code after the `await` never runs (no panic, no stream-end
/// log), and the BrowserState looks "connected" but the CDP event loop is
/// dead. Routing handler / heartbeat through this dedicated multi-thread
/// runtime keeps them alive across the entire app lifetime, decoupled from
/// per-tool runtimes.
pub(crate) fn browser_runtime() -> &'static tokio::runtime::Handle {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ha-browser-bg")
            .build()
            .expect("failed to build ha-browser background runtime")
    });
    rt.handle()
}

/// Per-tick timeout for the heartbeat `browser.version()` probe. Holding the
/// state lock this long is acceptable at 120s intervals; if the call hasn't
/// answered in 10s the transport is genuinely dead.
const HEARTBEAT_PROBE_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserReadyMode {
    ExistingConnection,
    ManagedLaunch,
}

impl BrowserState {
    fn new() -> Self {
        Self {
            browser: None,
            handler_task: None,
            chrome_child: None,
            heartbeat_task: None,
            pages: HashMap::new(),
            active_page_id: None,
            element_refs: Vec::new(),
            snapshot_url: None,
            connection_url: None,
            profile: None,
            transport_alive: Arc::new(AtomicBool::new(true)),
        }
    }

    /// `true` when this `BrowserState` owns the Chrome process (i.e. we
    /// spawned it via `spawn_chrome_and_connect`). Used by `browser_ui` to
    /// distinguish "launch" mode from "connect" mode.
    pub fn has_chrome_child(&self) -> bool {
        self.chrome_child.is_some()
    }

    /// `true` when there is any residual state — Chrome process, browser
    /// handle, or background task — that a `disconnect()` would clean up.
    ///
    /// **Distinct from [`is_connected`]**: when the heartbeat marks the ws
    /// transport dead, `is_connected` returns false but the Chrome process
    /// may still be alive (idle ws close doesn't kill Chrome). Disconnect
    /// paths must use this method, not `is_connected`, otherwise the dead
    /// Chrome leaks and its `SingletonLock` blocks the next launch.
    pub fn needs_cleanup(&self) -> bool {
        self.browser.is_some()
            || self.chrome_child.is_some()
            || self.handler_task.is_some()
            || self.heartbeat_task.is_some()
    }

    /// Connect to an already-running Chrome instance via CDP
    pub async fn connect(&mut self, debug_url: &str) -> anyhow::Result<()> {
        // First, discover the WebSocket debugger URL from /json/version
        let ws_url = discover_ws_url(debug_url).await?;

        app_info!("browser", "cdp", "Connecting to Chrome at {}", ws_url);

        // CRITICAL: `Browser::connect` must run ON the long-lived
        // `browser_runtime`, not the current (per-tool) runtime. The reason:
        // `Browser::connect` opens the CDP WebSocket and registers the
        // socket's IO driver with whatever tokio runtime is currently
        // active. If we let the per-tool runtime own that driver, the next
        // tool call's runtime drop tears down the IO and the handler task
        // dies with "A Tokio 1.x context was found, but it is being
        // shutdown". Spawning the connect future on `browser_runtime` makes
        // the IO driver, the WebSocket, and the handler loop all live on
        // the same long-lived runtime.
        let ws_url_for_connect = ws_url.clone();
        let connect_handle = browser_runtime().spawn(async move {
            Browser::connect(&ws_url_for_connect).await
        });
        let (browser, mut handler) = connect_handle
            .await
            .map_err(|e| anyhow::anyhow!("Failed to join Browser::connect task: {}", e))?
            .map_err(|e| anyhow::anyhow!("Failed to connect to Chrome at {}: {}. Make sure Chrome is running with --remote-debugging-port", debug_url, e))?;

        // Spawn the handler task — drives the CDP event loop. Same
        // long-lived-runtime rule applies (see comment above): both the
        // WebSocket IO and the poller must live on `browser_runtime`. Do
        // NOT break on errors — only exit when the stream ends.
        let handle = browser_runtime().spawn(async move {
            loop {
                match handler.next().await {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        app_warn!("browser", "cdp", "CDP handler error (continuing): {}", e);
                    }
                    None => {
                        app_info!("browser", "cdp", "CDP handler stream ended");
                        break;
                    }
                }
            }
        });

        let browser_arc = Arc::new(browser);
        self.browser = Some(browser_arc.clone());
        self.handler_task = Some(handle);
        self.connection_url = Some(debug_url.to_string());
        // Reset alive flag for this fresh connection.
        self.transport_alive.store(true, Ordering::SeqCst);

        // chromiumoxide's `Handler::new` fires `Target.setDiscoverTargets(true)`
        // on construction, but the response (and subsequent `Target.targetCreated`
        // events for pre-existing tabs) only arrive after the handler task
        // pumps the websocket. A single `yield_now()` is not enough time —
        // poll for ~600ms so the reconnect path actually sees existing tabs.
        self.refresh_pages_until_seen(std::time::Duration::from_millis(600))
            .await?;

        // Spawn heartbeat task. Defeats Chrome's ~4-minute ws idle timeout
        // by issuing a `browser.version()` ping every 120s (configurable).
        // Holds its own `Arc<Browser>` reference so the global mutex is
        // never held during the 10s probe timeout.
        let interval = heartbeat_interval_from_config();
        self.heartbeat_task = Some(spawn_heartbeat(
            browser_arc,
            self.transport_alive.clone(),
            interval,
        ));

        Ok(())
    }

    /// Poll `refresh_pages` until at least one page is seen or the deadline
    /// elapses. Used by `connect` so a reconnect-to-running-Chrome can
    /// observe pre-existing tabs without racing chromiumoxide's
    /// `Target.targetCreated` ingestion.
    ///
    /// Returning Ok with `self.pages` empty is legitimate (a freshly-launched
    /// Chrome with no tabs yet). The caller decides what an empty result
    /// means — `connect` treats it as fine.
    async fn refresh_pages_until_seen(
        &mut self,
        timeout: std::time::Duration,
    ) -> anyhow::Result<()> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.refresh_pages().await?;
            if !self.pages.is_empty() {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Unified Chrome launch entry point.
    ///
    /// Spawns Chrome as a child process with a fixed user-data-dir and
    /// `--remote-debugging-port`, polls `/json/version` until the debug
    /// listener is online, then connects via CDP. Unlike chromiumoxide's
    /// `Browser::launch`, we retain the `Child` handle inside `BrowserState`
    /// so `disconnect` / `Drop` can kill the Chrome process directly instead
    /// of relying on `kill_on_drop` heuristics.
    ///
    /// Caller responsibilities:
    /// - Pre-pick `spec.port` (via [`crate::browser::spawn::pick_managed_port`]
    ///   or hand the well-known 9222).
    /// - Pre-create the parent of `spec.user_data_dir`. The directory itself
    ///   is created here, but a missing parent (`~/.hope-agent/browser`) means
    ///   the caller has a config bug.
    ///
    /// SingletonLock cleanup runs inside — callers don't need to pre-check.
    pub async fn spawn_chrome_and_connect(&mut self, spec: LaunchSpec<'_>) -> anyhow::Result<()> {
        // Refuse the launch if this profile tripped the circuit breaker
        // within its cooldown window. Bookkeeping happens at the end.
        crate::browser::launch_circuit::check(spec.profile).map_err(|m| anyhow::anyhow!(m))?;
        let profile_key = spec.profile.to_string();
        let result = self.spawn_chrome_and_connect_inner(spec).await;
        match &result {
            Ok(()) => crate::browser::launch_circuit::record_success(&profile_key),
            Err(_) => crate::browser::launch_circuit::record_failure(&profile_key),
        }
        result
    }

    async fn spawn_chrome_and_connect_inner(&mut self, spec: LaunchSpec<'_>) -> anyhow::Result<()> {
        let exec = crate::browser::spawn::resolve_chrome_executable(spec.executable)?;

        // `managed` is documented as an ephemeral runner: cookies / sessions
        // / cache must NOT carry across launches, otherwise an LLM that
        // visited an authenticated page in the previous turn still has the
        // login next turn (and the skill / tool prompt advertises the
        // opposite). Tempdirs would be cleaner but rotating dir names break
        // SingletonLock paths and complicate stale-lock cleanup; instead we
        // keep the fixed `managed-runner/` location and wipe its contents
        // before each spawn. `user_attach` and user-defined profiles are
        // persistent by design and are left alone.
        if spec.profile == crate::browser::profile::BUILTIN_MANAGED
            && spec.user_data_dir.exists()
        {
            if let Err(e) = wipe_dir_contents(spec.user_data_dir) {
                app_warn!(
                    "browser",
                    "spawn",
                    "Failed to wipe managed runner udd before relaunch (continuing with leftover state): {}",
                    e
                );
            }
        }
        std::fs::create_dir_all(spec.user_data_dir)?;

        // Centralised SingletonLock check. All launch entry points (profile
        // tool, lazy auto-launch, settings UI) flow through here so the
        // stale-lock cleanup logic lives in one place.
        use crate::browser::singleton_lock;
        if singleton_lock::user_data_dir_is_locked(spec.user_data_dir) {
            if singleton_lock::is_lock_stale(spec.user_data_dir) {
                singleton_lock::cleanup_stale_lock(spec.user_data_dir)?;
            } else {
                anyhow::bail!(
                    "Chrome user-data-dir is already in use: {}. \
                     Disconnect (profile.op=disconnect) or pick a different profile.",
                    spec.user_data_dir.display()
                );
            }
        }

        let mut cmd = crate::browser::spawn::build_chrome_argv(&spec, &exec);
        let child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to launch Chrome at {exec:?}: {e}. \
                 Double-check the executable path in settings → Browser."
            )
        })?;
        let pid = child.id();
        self.chrome_child = Some(child);
        app_info!(
            "browser",
            "spawn",
            "Spawned Chrome pid={} port={} udd={}",
            pid,
            spec.port,
            spec.user_data_dir.display()
        );

        // Wait for Chrome's `--remote-debugging-port` listener to come online.
        // Cold start ranges 0.5–3s; 15s upper bound covers slow disks /
        // first-run UI prompts. Drop the child on failure so we don't leak a
        // half-started Chrome.
        let browser_url = format!("http://127.0.0.1:{}", spec.port);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if fetch_chrome_json_version(&browser_url, 1).await.is_ok() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                // Clean up the half-started Chrome before returning.
                if let Some(mut c) = self.chrome_child.take() {
                    let _ = c.kill();
                    let _ = tokio::task::spawn_blocking(move || {
                        let _ = c.wait();
                    })
                    .await;
                }
                anyhow::bail!(
                    "Spawned Chrome at {browser_url} but its debug listener never came online \
                     within 15s. Chrome may have failed to start (existing Chrome instance on \
                     this machine may be intercepting the launch). Try quitting your daily \
                     Chrome and retrying."
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Connect the CDP client side (same logic as `connect()` minus the
        // upfront `Browser::connect` wrapper — we reuse `connect()` directly
        // so the handler task and pages refresh are identical).
        if let Err(e) = self.connect(&browser_url).await {
            // Connect failed — reap Chrome so it doesn't outlive the failed
            // attempt and squat on the user-data-dir SingletonLock.
            if let Some(mut c) = self.chrome_child.take() {
                let _ = c.kill();
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = c.wait();
                })
                .await;
            }
            return Err(e);
        }
        self.profile = Some(spec.profile.to_string());

        // macOS Chrome activates itself on launch, stealing focus from Hope
        // Agent every time we spawn (and the LLM may spawn / re-launch
        // repeatedly during a session). Re-activate Hope Agent so the user's
        // chat stays in front. The user can always cmd-tab to the Chrome
        // window if they need to interact with it (login flows etc.).
        refocus_hope_agent_after_spawn();

        Ok(())
    }

    /// Disconnect from the browser, clean up resources, and reap the spawned
    /// Chrome process if we own one.
    ///
    /// Order matters: abort heartbeat → abort handler → drop Browser (closes
    /// ws) → kill + wait Chrome child. Waiting blocks the runtime, so we
    /// move the `wait()` to `spawn_blocking`.
    pub async fn disconnect(&mut self) {
        if let Some(handle) = self.heartbeat_task.take() {
            handle.abort();
        }
        if let Some(handle) = self.handler_task.take() {
            handle.abort();
        }
        self.browser.take();
        if let Some(mut child) = self.chrome_child.take() {
            let _ = child.kill();
            // Spawn-blocking the wait so we don't stall the async runtime;
            // best-effort — we don't fail disconnect on wait error.
            let _ = tokio::task::spawn_blocking(move || {
                let _ = child.wait();
            })
            .await;
        }
        self.pages.clear();
        self.active_page_id = None;
        self.element_refs.clear();
        self.snapshot_url = None;
        self.connection_url = None;
        self.profile = None;
        // Reset alive flag so the next connect starts with a clean signal.
        self.transport_alive.store(true, Ordering::SeqCst);

        app_info!("browser", "cdp", "Browser disconnected");
    }

    /// Check if connected to a browser. Three-way check:
    /// 1. `browser` handle exists (we called `connect`)
    /// 2. handler task hasn't broken its `handler.next()` loop
    /// 3. heartbeat hasn't observed a dead transport
    ///
    /// (1) and (2) catch the case where chromiumoxide noticed the ws closed
    /// itself; (3) catches the silent idle-timeout case where Chrome closes
    /// the ws but the handler hasn't seen it yet.
    pub fn is_connected(&self) -> bool {
        self.browser.is_some()
            && self.transport_alive.load(Ordering::SeqCst)
            && self.handler_task.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Refresh the page list from the browser
    pub async fn refresh_pages(&mut self) -> anyhow::Result<()> {
        let browser = self
            .browser
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to browser"))?;

        let pages = browser
            .pages()
            .await
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
        let page_id = self.active_page_id.as_ref().ok_or_else(|| {
            anyhow::anyhow!("No active page. Use 'new_page' or 'select_page' first.")
        })?;

        self.pages.get(page_id).ok_or_else(|| {
            anyhow::anyhow!(
                "Active page {} no longer exists. Use 'list_pages' to see available pages.",
                page_id
            )
        })
    }

    /// Find an element ref by ref_id
    pub fn find_ref(&self, ref_id: u32) -> anyhow::Result<&ElementRef> {
        self.element_refs
            .iter()
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

impl Drop for BrowserState {
    /// Best-effort kill of the spawned Chrome process at app shutdown. We
    /// don't `wait()` here — that would block. The `BrowserState` is a
    /// `OnceLock`-backed process singleton so `Drop` only fires at process
    /// exit; the kernel reparents the zombie to init/launchd which reaps it.
    fn drop(&mut self) {
        if let Some(mut child) = self.chrome_child.take() {
            let _ = child.kill();
        }
    }
}

// ── Global Singleton ─────────────────────────────────────────────

static BROWSER_STATE: OnceLock<Mutex<BrowserState>> = OnceLock::new();

pub fn get_browser_state() -> &'static Mutex<BrowserState> {
    BROWSER_STATE.get_or_init(|| Mutex::new(BrowserState::new()))
}

/// Refresh the global page table without holding [`BROWSER_STATE`]'s mutex
/// across the CDP `Browser.pages()` round-trip. Prefer this for status/UI
/// refresh paths; [`BrowserState::refresh_pages`] remains available for
/// code that is already performing a larger locked state transition.
pub async fn refresh_pages_unlocked() -> anyhow::Result<()> {
    let browser = {
        let state = get_browser_state().lock().await;
        state
            .browser
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Not connected to browser"))?
    };

    let pages = browser
        .pages()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list pages: {}", e))?;

    let mut state = get_browser_state().lock().await;
    state.pages.clear();
    for page in pages {
        let target_id = page.target_id().as_ref().to_string();
        state.pages.insert(target_id, page);
    }

    if state
        .active_page_id
        .as_ref()
        .is_some_and(|id| !state.pages.contains_key(id))
    {
        state.active_page_id = None;
    }
    if state.active_page_id.is_none() {
        state.active_page_id = state.pages.keys().next().cloned();
    }

    Ok(())
}

/// Auto-connect to Chrome if not already connected (tries 127.0.0.1:9222)
pub async fn ensure_connected() -> anyhow::Result<()> {
    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        return Ok(());
    }

    // Preserve the active page selection across an implicit reconnect.
    // chromiumoxide's `handler.next()` returning `None` (idle ws close
    // after a few minutes of inactivity) is silent — the user expects to
    // come back and find the same tab focused, not to lose state. The
    // target_id is stable across reconnects because Chrome owns it.
    let preserved_active_id = state.active_page_id.clone();

    // Clean up stale connection if handler died
    if state.browser.is_some() {
        app_info!(
            "browser",
            "cdp",
            "Cleaning up stale browser connection (handler died)"
        );
        state.disconnect().await;
    }

    state.connect("http://127.0.0.1:9222").await.map_err(|_| {
        anyhow::anyhow!(
            "Browser not connected. Please either:\n\
             1. Launch Chrome with: chrome --remote-debugging-port=9222\n\
             2. Use action=\"launch\" to start a managed Chrome instance\n\
             3. Use action=\"connect\" with a custom URL"
        )
    })?;

    if let Some(id) = preserved_active_id {
        if state.pages.contains_key(&id) {
            state.active_page_id = Some(id);
        }
    }
    Ok(())
}

/// Like `ensure_connected`, but falls back to launching a managed Chrome
/// instance for workflows that can safely open one on demand.
pub async fn ensure_connected_or_launch_managed() -> anyhow::Result<BrowserReadyMode> {
    let mut state = get_browser_state().lock().await;
    if state.is_connected() {
        return Ok(BrowserReadyMode::ExistingConnection);
    }

    // Same as `ensure_connected`: preserve the active page selection so a
    // silent reconnect (handler died after idle) restores user-visible state.
    let preserved_active_id = state.active_page_id.clone();

    if state.browser.is_some() {
        app_info!(
            "browser",
            "cdp",
            "Cleaning up stale browser connection before reconnect/launch fallback"
        );
        state.disconnect().await;
    }

    match state.connect("http://127.0.0.1:9222").await {
        Ok(()) => {
            app_info!(
                "browser",
                "cdp",
                "Connected to existing Chrome at default debug port for browser action"
            );
            if let Some(id) = preserved_active_id {
                if state.pages.contains_key(&id) {
                    state.active_page_id = Some(id);
                }
            }
            Ok(BrowserReadyMode::ExistingConnection)
        }
        Err(err) => {
            app_info!(
                "browser",
                "cdp",
                "Default debug port unavailable ({}); launching managed Chrome",
                err
            );
            // Route lazy auto-launch through the same fixed user-data-dir
            // as `profile=managed`. `spawn_chrome_and_connect` does the
            // SingletonLock check + cleanup + circuit-breaker bookkeeping
            // internally.
            let udd = crate::paths::browser_managed_runner_dir()?;
            let port = crate::browser::spawn::pick_managed_port().await?;
            let extra: Vec<String> = Vec::new();
            let spec = LaunchSpec {
                profile: crate::browser::profile::BUILTIN_MANAGED,
                executable: None,
                user_data_dir: &udd,
                port,
                headless: false,
                extra_args: &extra,
            };
            state.spawn_chrome_and_connect(spec).await?;
            Ok(BrowserReadyMode::ManagedLaunch)
        }
    }
}

// ── Helper: Discover WebSocket URL ───────────────────────────────

/// Fetch Chrome's `/json/version` JSON. Uses the hope-agent proxy-aware
/// reqwest builder so corporate-proxy users see the same probe behaviour
/// here, in the settings doctor banner, and anywhere else that wants to
/// know whether a Chrome is reachable. `timeout_secs` lets the doctor
/// path use a tighter 2s budget vs. the connect path's 5s.
pub async fn fetch_chrome_json_version(
    base_url: &str,
    timeout_secs: u64,
) -> anyhow::Result<serde_json::Value> {
    let version_url = format!("{}/json/version", base_url.trim_end_matches('/'));
    let client = crate::provider::apply_proxy_for_url(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)),
        &version_url,
    )
    .build()?;
    let resp = client.get(&version_url).send().await.map_err(|e| {
        anyhow::anyhow!(
            "Cannot reach Chrome at {}. Is Chrome running with --remote-debugging-port? Error: {}",
            base_url,
            e
        )
    })?;
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| anyhow::anyhow!("Invalid response from Chrome: {}", e))
}

/// Read the heartbeat interval from `AppConfig.browser.heartbeatIntervalSecs`,
/// clamped to a sane range. `0` and absent both yield the default.
fn heartbeat_interval_from_config() -> u64 {
    let cfg = crate::config::cached_config();
    let raw = cfg
        .browser
        .as_ref()
        .and_then(|b| b.heartbeat_interval_secs)
        .unwrap_or(0) as u64;
    if raw == 0 {
        DEFAULT_HEARTBEAT_INTERVAL_SECS
    } else {
        // Clamp: don't let user accidentally pick something too short (would
        // burn lock-time per tick) or too long (would defeat the point).
        raw.clamp(30, 600)
    }
}

/// Spawn a heartbeat task that pings `browser.version()` every
/// `interval_secs` to keep Chrome from idling out the WebSocket
/// (~4 minutes by default).
///
/// The task holds its own `Arc<Browser>` reference so the global
/// `BROWSER_STATE` mutex is **never** held during the probe — tool calls
/// arriving while a probe is in flight don't queue. On failure / timeout
/// the task sets `transport_alive=false` and exits; `is_connected` then
/// returns false and the next tool call triggers a fresh reconnect cycle
/// via `ensure_connected_or_launch_managed`.
///
/// `disconnect()` aborts the JoinHandle and drops the original Arc; the
/// task's clone is the last surviving reference and Browser drops with
/// the task when it observes the disconnect (next probe gets a use-after
/// -close error and exits gracefully).
fn spawn_heartbeat(
    browser: Arc<Browser>,
    transport_alive: Arc<AtomicBool>,
    interval_secs: u64,
) -> JoinHandle<()> {
    // Spawn on the dedicated long-lived runtime, same rationale as the
    // handler task: per-tool runtimes would silently cancel this on drop.
    browser_runtime().spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // First tick fires immediately — burn it so the actual probe waits a
        // full interval (no point in pinging Chrome 0ms after we connected).
        tick.tick().await;
        loop {
            tick.tick().await;
            let probe_result = tokio::time::timeout(
                Duration::from_secs(HEARTBEAT_PROBE_TIMEOUT_SECS),
                browser.version(),
            )
            .await;
            match probe_result {
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => {
                    app_warn!(
                        "browser",
                        "heartbeat",
                        "version() failed, marking transport dead: {}",
                        e
                    );
                    transport_alive.store(false, Ordering::SeqCst);
                    break;
                }
                Err(_) => {
                    app_warn!(
                        "browser",
                        "heartbeat",
                        "version() timed out after {}s; marking transport dead",
                        HEARTBEAT_PROBE_TIMEOUT_SECS
                    );
                    transport_alive.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    })
}

/// Fetch the WebSocket debugger URL from Chrome's /json/version endpoint
async fn discover_ws_url(base_url: &str) -> anyhow::Result<String> {
    let body = fetch_chrome_json_version(base_url, 5).await?;
    body.get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Chrome did not return webSocketDebuggerUrl. Response: {}",
                body
            )
        })
}

/// Wipe a directory's contents without removing the directory itself.
/// Used to make `profile=managed` ephemeral between launches. `remove_dir_all`
/// the directory itself is tempting but breaks if the directory is a mount
/// point or has special permissions; iterating entries is safe.
fn wipe_dir_contents(dir: &std::path::Path) -> std::io::Result<()> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in read.flatten() {
        let p = entry.path();
        let meta = match entry.file_type() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    Ok(())
}

/// Pull focus back to Hope Agent after spawning Chrome. macOS only — Chrome
/// calls `[NSApplication activateIgnoringOtherApps]` on launch which steals
/// focus from whichever app the user is working in (typically our chat
/// window).
///
/// We target the current process by Unix PID rather than app name. App name
/// matching (`tell application "Hope Agent"`) fails in dev mode where the
/// Tauri binary is loaded under its process name (`hope-agent`), not the
/// productName, and varies across release / debug / packaged builds. Unix
/// id is stable regardless.
///
/// Background-spawned for a few hundred ms because Chrome's
/// `activateIgnoringOtherApps` and AppKit window mapping fire over a couple
/// of frames after the process starts; a single shot fires before Chrome
/// has actually grabbed focus and the activate is no-op. Three retries at
/// 80 / 200 / 400 ms cover the typical race window without blocking the
/// caller. Fire-and-forget; ignore failures — UX nicety, not correctness.
fn refocus_hope_agent_after_spawn() {
    #[cfg(target_os = "macos")]
    {
        if !crate::app_init::is_desktop() {
            // Headless server mode has no GUI to refocus.
            return;
        }
        let pid = std::process::id();
        std::thread::spawn(move || {
            for delay_ms in [80u64, 200, 400] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let script = format!(
                    "tell application \"System Events\" to set frontmost of \
                     (first process whose unix id is {}) to true",
                    pid
                );
                let _ = std::process::Command::new("osascript")
                    .args(["-e", &script])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        });
    }
}

// Unit tests for the spawn primitives moved to `browser::spawn`. Launch
// end-to-end requires a real Chrome — kept as manual smoke test only.
