use crate::acp_control;
use crate::channel;
use crate::cron;
use crate::globals::AppState;
use crate::globals::{
    ACP_MANAGER, APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY,
    CODEX_TOKEN_CACHE, CRON_DB, EVENT_BUS, IDLE_EXTRACT_HANDLES, KNOWLEDGE_DB, LOG_DB,
    MEMORY_BACKEND, PROJECT_DB, REASONING_EFFORT, SESSION_DB, SUBAGENT_CANCELS,
};
use crate::knowledge::KnowledgeRegistry;
use crate::logging::{self, AppLogger, LogDB};
use crate::memory;
use crate::paths;
use crate::project::ProjectDB;
use crate::session::{self, SessionDB};
use crate::subagent;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// Sentinel marking that `init_runtime` has run successfully. Set last so
/// that any fatal panic during init leaves it clear and a retry attempt is
/// distinguishable from a successful first run.
static INIT_DONE: OnceLock<()> = OnceLock::new();

/// Records the runtime role passed to `init_runtime("desktop"|"server"|"acp"|"test")`.
/// First-write-wins. Tests in the same binary share this `OnceLock` — once
/// `init_runtime("test")` runs, `is_desktop()` stays `false` for every test.
static RUNTIME_ROLE: OnceLock<&'static str> = OnceLock::new();

/// User-facing app version, set by each binary entrypoint via
/// [`set_app_version`]. Distinct from `env!("CARGO_PKG_VERSION")` in
/// ha-core — `pnpm sync:version` syncs `package.json` → `src-tauri/Cargo.toml`
/// + `tauri.conf.json`, but does NOT touch this library crate. So
/// `ha-core`'s own crate version drifts behind the app version and must
/// not be used for "current version" comparisons in the updater path.
static APP_VERSION: OnceLock<&'static str> = OnceLock::new();

/// Register the calling binary's `CARGO_PKG_VERSION` so [`app_version`]
/// returns the user-facing app version. Idempotent; first call wins.
pub fn set_app_version(version: &'static str) {
    let _ = APP_VERSION.set(version);
}

/// Returns the version registered by the binary entrypoint via
/// [`set_app_version`]. Falls back to `ha-core`'s own crate version when
/// no entrypoint registered (test harnesses, library consumers). Self-update
/// callers MUST use this — never `env!("CARGO_PKG_VERSION")` directly.
pub fn app_version() -> &'static str {
    APP_VERSION
        .get()
        .copied()
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Returns the role string from the first `init_runtime()` call, or `None`
/// if `init_runtime` hasn't run yet. Most callers want [`is_desktop`] for
/// readable mode checks instead of comparing the string directly.
pub fn runtime_role() -> Option<&'static str> {
    RUNTIME_ROLE.get().copied()
}

/// True iff the process started as the desktop (Tauri) shell. Used by paths
/// that need to vary behavior by runtime mode without threading a parameter
/// through the call stack (e.g. `system_prompt::build` injecting
/// desktop-only guidance for clickable file paths).
pub fn is_desktop() -> bool {
    runtime_role() == Some("desktop")
}

/// True iff the process started as the ACP stdio bridge (`hope-agent acp`).
/// ACP runs over stdio for an editor client (Zed etc.); approvals can only
/// reach a human if that client declared a permission capability (Epic D7).
/// **Must use this, not `ChatSource`** — ACP turns reuse `ChatSource::Http`
/// ([`crate::acp`]), so source alone can't distinguish ACP from a real HTTP
/// client (D1 risk note).
pub fn is_acp() -> bool {
    runtime_role() == Some("acp")
}

/// Whether an interactive, approval-capable client is attached to this
/// process. Drives the unattended-approval surface check (Epic D): a headless
/// `server` with no web client and no IM-attached session has no one to answer
/// an `Ask`. Desktop always counts (the window + OS notification surface always
/// exist); `server` mode is attended while ≥1 client holds the `/ws/events`
/// stream — that's the channel `approval_required` broadcasts reach, and its
/// live count is already maintained by `server_status::events_ws_count`
/// (no extra wiring). ACP does NOT use this — it gates on the client's declared
/// permission capability instead.
pub fn desktop_client_present() -> bool {
    is_desktop()
        || crate::server_status::events_ws_counter().load(std::sync::atomic::Ordering::SeqCst) > 0
}

/// Initialize all global singletons (databases, OnceLocks, channel registry,
/// ACP control plane, orphan cleanup, embedder, welcome log). Idempotent —
/// the second call is a no-op so dev hot-reload and accidental double-call
/// don't reopen DB files or double-register channel plugins.
///
/// Side effects only — does not construct `AppState`. Desktop callers should
/// follow up with `build_app_state()`. Server / ACP modes don't need
/// `AppState` and stop here.
pub fn init_runtime(role: &'static str) {
    // Record role before the idempotent early-return so the first caller's
    // role wins. Subsequent `OnceLock::set` returns Err and is dropped.
    let _ = RUNTIME_ROLE.set(role);

    if INIT_DONE.get().is_some() {
        return;
    }

    /// Unwrap a Result or print a fatal error to stderr and panic.
    fn fatal<T>(result: anyhow::Result<T>, msg: &str) -> T {
        result.unwrap_or_else(|e| {
            eprintln!("[FATAL] {msg}: {e}");
            panic!("{msg}: {e}");
        })
    }

    // Make sure the data dir exists before we try to put a lock file in
    // it. Idempotent — Tauri / server / acp entrypoints already call
    // this, but covering it here keeps `init_runtime` self-sufficient.
    if let Err(e) = paths::ensure_dirs() {
        eprintln!("[runtime_lock] ensure_dirs failed: {e}");
    }

    // Pre-warm the user's login-shell environment snapshot on a background
    // thread so the first `exec` doesn't pay the one-time (~1s) cost of sourcing
    // the shell on its hot path. Unix-only; Windows inherits the process env.
    #[cfg(unix)]
    std::thread::spawn(|| {
        let _ = crate::tools::exec::login_shell_env();
    });

    // Elect Primary / Secondary across the data dir. Tier is captured
    // here but logged via `app_info!` further down once APP_LOGGER is
    // initialised; commit C8 wires the cleanup + single-owner loop
    // gating against `runtime_lock::is_primary()`.
    let tier = crate::runtime_lock::acquire_or_secondary_for(role);

    // Bootstrap a default EventBus if no caller pre-installed one. Tauri
    // shell installs its own bridged bus before `.manage(...)`; the HTTP
    // server installs one before building AppContext; ACP doesn't bridge
    // anywhere but still wants `emit()` to be a no-op rather than a panic.
    // First-write-wins (`OnceLock::set` returns Err on second call), so this
    // is safe regardless of order.
    if EVENT_BUS.get().is_none() {
        let bus: Arc<dyn crate::event_bus::EventBus> =
            Arc::new(crate::event_bus::BroadcastEventBus::new(256));
        let _ = EVENT_BUS.set(bus);
    }

    // Initialize the SessionDB
    let db_path = fatal(session::db_path(), "Cannot resolve session database path");
    let session_db = Arc::new(fatal(
        SessionDB::open(&db_path),
        "Cannot open session database",
    ));
    let _ = SESSION_DB.set(session_db.clone());

    // Initialize the ProjectDB (shares the SessionDB SQLite connection).
    // Run its table-creation migration so `projects` / `project_files` exist
    // before any command touches them.
    let project_db = Arc::new(ProjectDB::new(session_db.clone()));
    if let Err(e) = project_db.migrate() {
        eprintln!("[FATAL] Cannot run project DB migration: {e}");
        panic!("project DB migration failed: {e}");
    }
    let _ = PROJECT_DB.set(project_db);

    // Initialize the KnowledgeRegistry (also shares the SessionDB connection).
    // Its migration creates `knowledge_bases` + `session/project_knowledge_bases`
    // before any command or tool touches them.
    let knowledge_db = Arc::new(KnowledgeRegistry::new(session_db.clone()));
    if let Err(e) = knowledge_db.migrate() {
        eprintln!("[FATAL] Cannot run knowledge DB migration: {e}");
        panic!("knowledge DB migration failed: {e}");
    }
    let _ = KNOWLEDGE_DB.set(knowledge_db);

    // Open the knowledge index cache (index.db) + install the note embedder.
    // Non-fatal: notes degrade to FTS-only / no search if this fails.
    if let Err(e) = crate::knowledge::index::init_index_db() {
        eprintln!("[runtime] knowledge index init failed: {e}");
    }

    // Initialize the LogDB and AppLogger. `LogDB` captures the db path
    // internally so we don't need to keep it around in this scope.
    let log_db_path = fatal(logging::db_path(), "Cannot resolve log database path");
    let log_db = Arc::new(fatal(LogDB::open(&log_db_path), "Cannot open log database"));
    let _ = LOG_DB.set(log_db.clone());

    // Retention cleanup (by age + by DB size) is owned entirely by
    // `AppLogger::cleanup_loop`; its interval fires immediately after the
    // logger starts so startup stays off the VACUUM hot path.
    let log_config = logging::load_log_config().unwrap_or_default();
    let logs_dir = fatal(paths::logs_dir(), "Cannot resolve logs directory");
    let logger = AppLogger::new(log_db, logs_dir);
    logger.update_config(log_config);

    // Store logger globally for access from non-State contexts
    let _ = APP_LOGGER.set(logger.clone());

    // First-run seed: give a fresh install one usable knowledge space out of the
    // box (idempotent via a sentinel — never recreated if the user deletes it).
    // Placed after the logger so its app_info!/app_warn! land; registry + index
    // (set above) are the only other prerequisites. Primary-only — the sentinel
    // check isn't atomic across instances, so a Secondary booting a fresh shared
    // data dir could otherwise race the Primary and seed a duplicate space.
    if crate::runtime_lock::is_primary() {
        crate::knowledge::service::ensure_default_knowledge_base();
    }

    recover_startup_session_state(&session_db, tier);

    // Initialize the hooks subsystem. Lightweight + synchronous here (the
    // registry/transcript are lazy); the transcript backfill of existing
    // sessions runs off the critical path in `start_background_tasks`.
    crate::hooks::init();

    // Initialize the MemoryDB
    let memory_db_path = fatal(
        paths::memory_db_path(),
        "Cannot resolve memory database path",
    );
    // Build the concrete backend Arc once: the trait object goes into
    // MEMORY_BACKEND, and clones seed the Dreaming durable store + the claim
    // store. The dreaming_* and claim tables live in the same memory.db, so
    // both stores reuse this backend's write/read connections rather than
    // opening their own.
    let sqlite_backend = Arc::new(fatal(
        memory::SqliteMemoryBackend::open(&memory_db_path),
        "Cannot open memory database",
    ));
    crate::memory::dreaming::init_store(sqlite_backend.clone());
    crate::memory::claims::init_claim_store(sqlite_backend.clone());
    crate::memory::episodes::init_episode_store(sqlite_backend.clone());
    let memory_backend: Arc<dyn memory::MemoryBackend> = sqlite_backend;
    let _ = MEMORY_BACKEND.set(memory_backend);

    // Memory embedding provider initialization is DEFERRED to
    // `start_background_tasks` / `start_minimal_background_tasks` (see
    // `spawn_embedding_init`). Constructing the provider reaches the network
    // (API probe, or a local Ollama endpoint that may still be starting) — far
    // too heavy for this synchronous,
    // pre-window init path. The backend tolerates a missing embedder (recall
    // degrades to FTS-only until it lands) and the config hot-reload path
    // (`tools::settings::trigger_backend_hot_reload`) sets the embedder
    // independently, so deferring is safe.

    // Initialize the CronDB (scheduler started in start_background_tasks)
    let cron_db_path = fatal(paths::cron_db_path(), "Cannot resolve cron database path");
    let cron_db = Arc::new(fatal(
        cron::CronDB::open(&cron_db_path),
        "Cannot open cron database",
    ));
    if let Err(e) = session_db.reconcile_terminal_loop_cron_jobs(&cron_db) {
        crate::app_warn!(
            "loop",
            "cron_status_reconcile",
            "Failed to reconcile terminal Loop cron statuses: {}",
            e
        );
    }
    let _ = CRON_DB.set(cron_db);

    // R1: the background-jobs cache moved from `async_jobs.db` → `background_jobs.db`.
    // Best-effort discard the legacy file/dir (pure rebuildable cache, no migration).
    if let Ok(old) = paths::legacy_async_jobs_db_path() {
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(old.with_extension("db-wal"));
        let _ = std::fs::remove_file(old.with_extension("db-shm"));
    }
    if let Ok(old_dir) = paths::legacy_async_jobs_dir() {
        let _ = std::fs::remove_dir_all(&old_dir);
    }
    // Failure here is non-fatal — async tools degrade to sync mode if the DB cannot be opened.
    match paths::background_jobs_db_path().and_then(|p| crate::async_jobs::JobsDB::open(&p)) {
        Ok(db) => crate::async_jobs::set_async_jobs_db(Arc::new(db)),
        Err(e) => crate::app_warn!(
            "async_jobs",
            "init",
            "Failed to open background_jobs DB ({}); async tool backgrounding disabled",
            e
        ),
    }

    // Agent self-scheduled wakeups (R10). Non-fatal: without it `schedule_wakeup`
    // arms in-memory-only timers that don't survive a restart.
    match paths::wakeups_db_path().and_then(|p| crate::wakeup::WakeupDB::open(&p)) {
        Ok(db) => crate::wakeup::set_wakeup_db(Arc::new(db)),
        Err(e) => crate::app_warn!(
            "wakeup",
            "init",
            "Failed to open wakeups DB ({}); scheduled wakeups won't survive restart",
            e
        ),
    }

    // Failure here is non-fatal — local model setup stays available through
    // the older synchronous commands, but the global task center is disabled.
    match paths::local_model_jobs_db_path()
        .and_then(|p| crate::local_model_jobs::LocalModelJobsDB::open(&p))
    {
        Ok(db) => crate::local_model_jobs::set_local_model_jobs_db(Arc::new(db)),
        Err(e) => crate::app_warn!(
            "local_model_jobs",
            "init",
            "Failed to open local_model_jobs DB ({}); model install jobs disabled",
            e
        ),
    }

    // Log system startup
    logger.log(
        "info",
        "system",
        "lib::run",
        "Hope Agent started",
        None,
        None,
        None,
    );

    // Tier election result. Logging it after APP_LOGGER is set so it
    // lands in the SQLite log + log file alongside the welcome line.
    app_info!(
        "runtime",
        "tier",
        "elected {:?} (role={}, holder={:?})",
        tier,
        role,
        crate::runtime_lock::current_holder()
    );

    // Sub-agent cancel registry + idle-extract handle map
    let _ = SUBAGENT_CANCELS.set(Arc::new(subagent::SubagentCancelRegistry::new()));
    let _ = IDLE_EXTRACT_HANDLES.set(std::sync::Mutex::new(std::collections::HashMap::new()));

    // Per-AppState fields that desktop reads via OnceLock too. Constructed
    // here so server / acp modes get the same defaults without depending on
    // `build_app_state`.
    let _ = CHANNEL_CANCELS.set(Arc::new(channel::ChannelCancelRegistry::new()));
    let _ = CODEX_TOKEN_CACHE.set(Arc::new(Mutex::new(None::<(String, String)>)));
    let global_reasoning_effort = crate::config::cached_config().reasoning_effort.clone();
    let _ = REASONING_EFFORT.set(Arc::new(Mutex::new(global_reasoning_effort)));
    let _ = CACHED_AGENT.set(Arc::new(Mutex::new(None::<crate::agent::AssistantAgent>)));

    // Idempotent convergence for a previous Provider delete that completed
    // config persistence but was interrupted while repairing Agent/Session
    // references across their separate stores.
    let repair = crate::provider::repair_hard_deleted_model_references();
    if repair.failures > 0 {
        crate::app_warn!(
            "provider",
            "startup-repair",
            "model reference repair incomplete: failures={}",
            repair.failures
        );
    }

    // Startup orphan sweeps. Gated on Primary tier so a Secondary process
    // (e.g. acp launching while desktop is running) doesn't mark-error the
    // desktop's live subagent runs / team members or, worst case, hard-
    // delete its incognito sessions. Defense in depth: incognito purge
    // also has a per-row updated_at < now-60s SQL guard added in this
    // commit (see purge_orphan_incognito_sessions).
    if crate::runtime_lock::is_primary() {
        // Clean up orphan sub-agent runs from previous app session
        subagent::cleanup_orphan_runs(&session_db);

        // Clean up orphan team members from previous app session
        crate::team::cleanup::cleanup_orphan_teams(&session_db);

        // Backstop the live close-on-leave path: incognito sessions left from a
        // crash / SIGKILL / power loss never reach the frontend purge call.
        crate::session::cleanup_orphan_incognito(&session_db);

        // Recover crash-orphaned Dreaming state from a previous process:
        // fail stale `running` rows, clear expired locks, and return abandoned
        // `claimed` pending sources to the queue (next-gen Dreaming Phase 0).
        crate::memory::dreaming::recover_on_startup();

        // One-shot rename of the legacy `"default"` agent id to the new
        // hardcoded `DEFAULT_AGENT_ID` (`"ha-main"`). Idempotent — writes a
        // sentinel and short-circuits on subsequent startups. Failure is
        // logged but non-fatal: the app keeps booting on the old id, and the
        // next startup retries.
        if let Err(e) = crate::agent::migration::migrate_default_agent_id_to_ha_main() {
            app_error!(
                "agent",
                "migration",
                "default-agent-id rename migration failed: {}",
                e
            );
        }
    }

    // Initialize IM Channel system
    {
        // Inbound buffer 1024: non-Message events (reactions / read receipts /
        // membership / etc.) can be high-volume on busy chats and we don't
        // want them to back-pressure real chat messages. v0.2.0 keeps the
        // non-Message variants log-only so per-event work is < 1ms.
        let (mut registry, inbound_rx) = channel::ChannelRegistry::new(1024);

        // Register built-in channel plugins
        registry.register_plugin(Arc::new(channel::telegram::TelegramPlugin::new()));
        registry.register_plugin(Arc::new(channel::wechat::WeChatPlugin::new()));
        registry.register_plugin(Arc::new(channel::slack::SlackPlugin::new()));
        registry.register_plugin(Arc::new(channel::feishu::FeishuPlugin::new()));
        registry.register_plugin(Arc::new(channel::discord::DiscordPlugin::new()));
        registry.register_plugin(Arc::new(channel::qqbot::QqBotPlugin::new()));
        registry.register_plugin(Arc::new(channel::irc::IrcPlugin::new()));
        registry.register_plugin(Arc::new(channel::signal::SignalPlugin::new()));
        registry.register_plugin(Arc::new(channel::imessage::IMessagePlugin::new()));
        registry.register_plugin(Arc::new(channel::whatsapp::WhatsAppPlugin::new()));
        registry.register_plugin(Arc::new(channel::googlechat::GoogleChatPlugin::new()));
        registry.register_plugin(Arc::new(channel::line::LinePlugin::new()));

        let registry = Arc::new(registry);
        let channel_db = Arc::new(channel::ChannelDB::new(session_db.clone()));

        // Run channel DB migration
        if let Err(e) = channel_db.migrate() {
            app_error!(
                "channel",
                "init",
                "Failed to run channel DB migration: {}",
                e
            );
        }

        // Spawn the inbound message dispatcher. Self-hosted on a dedicated
        // OS thread with its own tokio runtime, so it's safe to call from
        // sync init regardless of which mode (desktop / server / acp) is
        // bringing up the runtime.
        channel::worker::spawn_dispatcher(registry.clone(), channel_db.clone(), inbound_rx);

        // NOTE: approval / ask_user listeners use bare `tokio::spawn` and
        // require an ambient tokio runtime. They moved to
        // `start_background_tasks()` so server / acp paths (which call
        // `init_runtime` from sync stacks) don't panic on missing runtime.

        let _ = CHANNEL_REGISTRY.set(registry);
        let _ = CHANNEL_DB.set(channel_db);
    }

    // Initialize ACP control plane (non-async parts only).
    // This is also the first `cached_config()` call on the Tauri setup path,
    // which synchronously populates the in-memory provider-store cache so
    // later async hot paths (tool execution, chat, channel workers) never
    // block on the initial disk read. Do not remove without auditing.
    {
        let store = crate::config::cached_config();
        if store.acp_control.enabled {
            let registry = Arc::new(acp_control::AcpRuntimeRegistry::new());
            let manager = Arc::new(acp_control::AcpSessionManager::new(registry));
            let _ = ACP_MANAGER.set(manager);
        }
    }

    // Install a panic hook that flushes any in-flight stream persisters
    // before the original hook (Tauri's logger / test harness / etc.)
    // takes over. Idempotent — multiple `init_runtime` calls are no-op.
    // Signal handlers (SIGINT/SIGTERM) need a tokio runtime, so each
    // mode entrypoint installs them separately after their runtime is up.
    crate::crash_flush::install_panic_hook();

    // Mark init complete only after every fallible step has succeeded. Any
    // earlier `fatal()` panic kills the process before this set runs.
    let _ = INIT_DONE.set(());
}

/// Construct the desktop `AppState` from already-initialised global
/// singletons. **Must be preceded by `init_runtime()`.** Server / ACP modes
/// do not call this — they consume the OnceLocks directly.
pub fn build_app_state() -> AppState {
    debug_assert!(
        INIT_DONE.get().is_some(),
        "build_app_state called before init_runtime"
    );

    // OnceLocks are always Some after `init_runtime()`. The `require_*`
    // accessors share one error shape ("X not initialized") so we surface
    // the same message everyone else (HTTP routes, slash commands) sees.
    let session_db = crate::require_session_db()
        .expect("init_runtime contract")
        .clone();
    let project_db = crate::require_project_db()
        .expect("init_runtime contract")
        .clone();
    let knowledge_db = crate::require_knowledge_db()
        .expect("init_runtime contract")
        .clone();
    let log_db = crate::require_log_db()
        .expect("init_runtime contract")
        .clone();
    let cron_db = crate::require_cron_db()
        .expect("init_runtime contract")
        .clone();
    let subagent_cancels = crate::require_subagent_cancels()
        .expect("init_runtime contract")
        .clone();
    let channel_cancels = crate::require_channel_cancels()
        .expect("init_runtime contract")
        .clone();
    let codex_token = crate::require_codex_token_cache()
        .expect("init_runtime contract")
        .clone();
    let reasoning_effort = crate::require_reasoning_effort_cell()
        .expect("init_runtime contract")
        .clone();
    let cached_agent = crate::require_cached_agent()
        .expect("init_runtime contract")
        .clone();
    let logger = crate::require_logger()
        .expect("init_runtime contract")
        .clone();

    let state = AppState {
        agent: cached_agent,
        auth_result: Arc::new(Mutex::new(None)),
        reasoning_effort,
        codex_token,
        current_agent_id: Mutex::new(crate::agent_loader::DEFAULT_AGENT_ID.to_string()),
        session_db,
        project_db,
        knowledge_db,
        chat_cancel: Arc::new(AtomicBool::new(false)),
        log_db,
        logger,
        cron_db,
        subagent_cancels,
        channel_cancels,
    };

    // Guardrail: every OnceLock-backed AppState field must share the
    // same Arc. A drift silently breaks cross-runtime reads — this
    // exact bug class motivated removing the dead `APP_STATE`.
    debug_assert!(
        ptr_eq_lock(&CHANNEL_CANCELS, &state.channel_cancels),
        "CHANNEL_CANCELS OnceLock and AppState.channel_cancels must share the same Arc"
    );
    debug_assert!(
        ptr_eq_lock(&CODEX_TOKEN_CACHE, &state.codex_token),
        "CODEX_TOKEN_CACHE OnceLock and AppState.codex_token must share the same Arc"
    );
    debug_assert!(
        ptr_eq_lock(&REASONING_EFFORT, &state.reasoning_effort),
        "REASONING_EFFORT OnceLock and AppState.reasoning_effort must share the same Arc"
    );
    debug_assert!(
        ptr_eq_lock(&CACHED_AGENT, &state.agent),
        "CACHED_AGENT OnceLock and AppState.agent must share the same Arc"
    );

    state
}

/// Backwards-compatible shim. New call sites should use `init_runtime`
/// + `build_app_state()` directly so it's clear which side effect they
/// want. The role is hard-coded to `"desktop"` because the only
/// surviving caller of this shim is the Tauri shell.
pub fn init_app_state() -> AppState {
    init_runtime("desktop");
    build_app_state()
}

fn ptr_eq_lock<T>(lock: &std::sync::OnceLock<Arc<T>>, field: &Arc<T>) -> bool {
    lock.get()
        .map(|arc| Arc::ptr_eq(arc, field))
        .unwrap_or(false)
}

/// Spawn the IM channel approval + ask_user listeners. Both internally
/// use bare `tokio::spawn` so they require an ambient tokio runtime —
/// callers are `start_background_tasks` and `start_minimal_background_tasks`,
/// never `init_runtime` (which can run on a sync stack). No-op if the
/// channel registry isn't initialised yet.
fn spawn_channel_listeners() {
    if let (Some(channel_db), Some(registry)) = (CHANNEL_DB.get(), CHANNEL_REGISTRY.get()) {
        channel::worker::approval::spawn_channel_approval_listener(
            channel_db.clone(),
            registry.clone(),
        );
        channel::worker::ask_user::spawn_channel_ask_user_listener(
            channel_db.clone(),
            registry.clone(),
        );
        channel::worker::spawn_channel_eviction_watcher(registry.clone());
        spawn_channel_menu_resync_listener(registry.clone());
        // Send a single "back online" notice to recently-active IM
        // conversations after a fresh process boot. Self-gates on
        // runtime_lock::is_primary() + AppConfig.startup_notification.enabled
        // and is a no-op otherwise.
        channel::worker::spawn_startup_notifier(registry.clone());
    }
}

/// Subscribe to `skills:catalog_changed` and config events that touch the
/// slash-command catalog (skill enable/disable, extra dirs) and re-sync each
/// running IM channel's bot menu.
///
/// Debounced with a 2s trailing-edge timer so a bulk import or a chain of
/// `bump_skill_version` calls collapses into one `setMyCommands` /
/// `bulk_overwrite_global_commands` round-trip per affected account.
fn spawn_channel_menu_resync_listener(registry: Arc<channel::ChannelRegistry>) {
    let Some(bus) = crate::globals::get_event_bus() else {
        app_warn!(
            "channel",
            "menu_sync",
            "EventBus not initialized — IM menu auto-resync disabled"
        );
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        const DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);
        let mut pending: Option<tokio::time::Instant> = None;

        loop {
            // Either wake on a new event, or wake when the debounce window
            // closes for a previously-buffered event.
            let recv = if let Some(deadline) = pending {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => {
                        pending = None;
                        let synced = registry.sync_commands_for_all().await;
                        if synced > 0 {
                            app_info!(
                                "channel",
                                "menu_sync",
                                "Re-synced slash command menus on {} running account(s)",
                                synced
                            );
                        }
                        continue;
                    }
                    ev = rx.recv() => ev,
                }
            } else {
                rx.recv().await
            };

            match recv {
                Ok(event) => {
                    if menu_resync_event_relevant(&event) {
                        pending = Some(tokio::time::Instant::now() + DEBOUNCE);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    app_warn!(
                        "channel",
                        "menu_sync",
                        "EventBus lagged {} events — forcing menu re-sync",
                        n
                    );
                    pending = Some(tokio::time::Instant::now() + DEBOUNCE);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Subscribe to `config:changed` and rebuild the global hooks registry when a
/// hooks-relevant category changes (design §4.7 hot reload). Tier-agnostic: the
/// compiled registry is per-process in-memory state, so each process keeps its
/// own copy current. Also performs the initial load.
fn spawn_hooks_config_listener() {
    let Some(bus) = crate::globals::get_event_bus() else {
        app_warn!(
            "hooks",
            "config",
            "EventBus not initialized — hooks hot-reload disabled"
        );
        return;
    };
    // Initial registry load from current config.
    crate::hooks::registry::reload_from_config();
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if hooks_config_event_relevant(&event) {
                        crate::hooks::registry::reload_from_config();
                        app_info!(
                            "hooks",
                            "config",
                            "hooks registry reloaded after config change"
                        );
                    }
                }
                // Missed events — force a reload to converge.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    crate::hooks::registry::reload_from_config();
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Apply the `prevent_sleep` user setting and keep it in sync with config
/// changes. Holds an OS sleep assertion (macOS `caffeinate` / Linux logind
/// inhibitor / Windows `ES_SYSTEM_REQUIRED`) for the lifetime of the setting.
/// Primary-only: sleep prevention is a host-level resource, so only one process
/// should hold the assertion — secondary instances must not spawn duplicates.
fn spawn_keep_awake_listener() {
    // `keep_awake::apply` does blocking work (fork/exec a helper + waitpid on
    // Unix, thread join on Windows), so every call is offloaded to a blocking
    // thread and never run on a tokio worker. The loop also tracks the last
    // applied value, so unrelated `config:changed` events (which carry no
    // category we can cheaply discriminate on) don't re-enter `apply` — this
    // avoids a spawn/log storm when the OS helper is unavailable.
    let Some(bus) = crate::globals::get_event_bus() else {
        // No hot-reload without a bus; still honour the current setting once,
        // off any runtime thread.
        let enabled = crate::config::cached_config().prevent_sleep;
        std::thread::spawn(move || crate::platform::keep_awake::apply(enabled));
        app_warn!(
            "platform",
            "keep_awake",
            "EventBus not initialized — sleep-prevention hot-reload disabled"
        );
        return;
    };
    // Subscribe before the initial apply so a `config:changed` racing startup is
    // buffered for the loop rather than lost.
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        let mut last = crate::config::cached_config().prevent_sleep;
        let _ = tokio::task::spawn_blocking(move || crate::platform::keep_awake::apply(last)).await;
        loop {
            match rx.recv().await {
                Ok(event) if event.name != "config:changed" => continue,
                Ok(_) => {} // config:changed
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {} // converge
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
            // Cache is refreshed before `config:changed` fires, so this sees the
            // new value. Only act on an actual transition.
            let desired = crate::config::cached_config().prevent_sleep;
            if desired != last {
                last = desired;
                let _ = tokio::task::spawn_blocking(move || {
                    crate::platform::keep_awake::apply(desired)
                })
                .await;
            }
        }
    });
}

/// Whether a `config:changed` event should rebuild the hooks registry.
fn hooks_config_event_relevant(event: &crate::event_bus::AppEvent) -> bool {
    event.name == "config:changed"
        && event
            .payload
            .get("category")
            .and_then(|c| c.as_str())
            .map(|c| matches!(c, "hooks" | "user" | "app"))
            .unwrap_or(false)
}

/// Config categories whose changes can shift the slash-command catalog. Kept
/// as an explicit list (rather than a `starts_with("skill")` heuristic) so a
/// future unrelated `skill_*` field can't silently force IM bot menu re-syncs.
/// Matches the categories used in `skills::commands::*` and `tools::settings`.
const MENU_RESYNC_CATEGORIES: &[&str] = &[
    "skills",
    "extra_skills_dirs",
    "disabled_skills",
    "skill_env",
    "skill_env_check",
    "skills.auto_review",
];

fn menu_resync_event_relevant(event: &crate::event_bus::AppEvent) -> bool {
    match event.name.as_str() {
        "skills:catalog_changed" => true,
        "config:changed" => event
            .payload
            .get("category")
            .and_then(|c| c.as_str())
            .map(|c| MENU_RESYNC_CATEGORIES.contains(&c))
            .unwrap_or(false),
        _ => false,
    }
}

/// Start background async tasks that require a tokio runtime.
/// Must be called from within a tokio async context (e.g., Tauri's `.setup()` or a server runtime).
///
/// Most spawns here are guarded by `runtime_lock::is_primary()` because
/// they own shared SQLite state (cron scheduler, retention sweeps,
/// dreaming, MCP watchdog, ACP backend discovery, async-jobs replay)
/// or compete for an external resource (channel auto-start fights with
/// any other process for the same Telegram bot webhook). Manual user
/// actions on the same subsystems (`/api/cron/jobs/{id}/run` via atomic
/// SQL claim, `/api/dreaming/run`, channel `start_account` button) are
/// tier-agnostic and continue to work in Secondary processes.
/// Construct the memory embedding provider off the critical path and install
/// it via `set_embedder`. Heavy (ONNX init + possible first-run model
/// download), so it runs on a blocking thread. No-op when embedding is
/// disabled in config or the backend isn't initialized. Idempotent w.r.t. the
/// config hot-reload path (`trigger_backend_hot_reload`) — both call
/// `set_embedder`, last write wins; `load_config()` reads the latest config at
/// task-run time, so the race window is sub-second and self-heals on any later
/// config save.
fn spawn_embedding_init() {
    tokio::task::spawn_blocking(|| {
        let Some(backend) = MEMORY_BACKEND.get() else {
            return;
        };
        let Ok(store) = crate::config::load_config() else {
            return;
        };

        // New selector (memory_embedding) takes priority; legacy single
        // `embedding` config is the fallback — mirrors the old init_runtime
        // branch order exactly.
        let provider = if store.memory_embedding.enabled {
            match memory::resolve_memory_embedding_config(
                &store.memory_embedding,
                &store.embedding_models,
            )
            .and_then(|resolved| {
                resolved
                    .map(|(_, config, _)| memory::create_embedding_provider(&config))
                    .transpose()
            }) {
                Ok(p) => p,
                Err(e) => {
                    app_warn!(
                        "memory",
                        "embedding",
                        "Failed to auto-initialize memory embedding provider (deferred): {}",
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(p) = provider {
            backend.set_embedder(p);
            app_info!(
                "memory",
                "embedding",
                "Memory embedding provider initialized (deferred, off critical path)"
            );
        }
    });
}

pub async fn start_background_tasks() {
    let primary = crate::runtime_lock::is_primary();

    // Tier-agnostic: EventBus subscription is multi-subscriber-safe.
    spawn_channel_listeners();

    // Tier-agnostic: local Chrome Extension broker. It only binds loopback and
    // writes a rebuildable discovery file for the Native Messaging host.
    crate::browser::BrowserExtensionBroker::spawn_global();

    // Tier-agnostic: session-lifecycle cleanup fan-out (delete/purge → deny
    // pending approvals, cancel jobs, drop IM pending, clear rules). NOT inside
    // spawn_channel_listeners — server / ACP have no channel registry but still
    // delete sessions.
    crate::session::cleanup_watcher::spawn_session_cleanup_watcher();

    // Tier-agnostic: per-process in-memory hooks registry + hot-reload.
    spawn_hooks_config_listener();

    // Memory embedding provider: deferred off init_runtime's synchronous
    // pre-window path (ONNX init + possible model download is 300ms–2s).
    // Per-process in-memory state, tier-agnostic.
    spawn_embedding_init();

    // Pending upload leases are opaque client staging, not durable files.
    // Sweep legacy chat leases and generic leases once at startup and every
    // 15 minutes thereafter.
    crate::blocking::run_blocking(|| {
        let _ = crate::attachments::cleanup_expired_chat_attachment_uploads();
        let _ = crate::file_upload::cleanup_expired_uploads();
    })
    .await;
    tokio::spawn(async {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            crate::blocking::run_blocking(|| {
                let _ = crate::attachments::cleanup_expired_chat_attachment_uploads();
                let _ = crate::file_upload::cleanup_expired_uploads();
            })
            .await;
        }
    });

    // Background weather cache refresh — desktop UI only. Moved here from
    // src-tauri setup.rs so it shares the ambient runtime instead of spawning
    // its own OS thread + tokio Runtime. Gated on desktop to skip the loop
    // entirely in server / ACP (refresh also self-checks weather_enabled).
    if is_desktop() {
        crate::weather::start_background_refresh();
    }

    // R7.1 background-job scheduler: promotes queued jobs (status `Queued`) into
    // free slots, per-session round-robin, as running jobs finish. Tier-agnostic
    // — the wait queue is process-local (pins live ctx), so each process
    // schedules its OWN queue and never touches another process's jobs;
    // `run_scheduler` is idempotent (at most one loop per process).
    tokio::spawn(async move {
        crate::async_jobs::JobManager::run_scheduler().await;
    });
    // R7.2: subagent reject→queue scheduler — promotes parked (`Queued`) spawns
    // as running children settle. Idempotent per process (mirrors run_scheduler).
    tokio::spawn(async move {
        crate::subagent::queue::run_subagent_scheduler().await;
    });
    // R8 follow-up: mirror background-subagent inner approvals onto their
    // projection label (running ⇄ awaiting_approval). Idempotent per process.
    crate::async_jobs::approval_projection_watcher::spawn_subagent_approval_projection_watcher();

    if primary {
        // Host-level sleep prevention (`prevent_sleep` setting). Primary-only so
        // a single process owns the OS assertion; reacts to config changes.
        spawn_keep_awake_listener();

        // Cron scheduler self-hosts a dedicated OS thread with its own tokio
        // runtime (see scheduler.rs). Primary-only because the periodic
        // tick's `claim_scheduled_job_for_execution` would double-claim
        // jobs across processes; manual run-now uses an atomic SQL claim
        // that's still safe in any tier.
        if let (Some(cron_db), Some(session_db)) = (CRON_DB.get(), SESSION_DB.get()) {
            let _handle = cron::start_scheduler(cron_db.clone(), session_db.clone());
        }
        crate::loop_control::spawn_loop_event_trigger_watcher();

        // Headless auto-update: periodic check + optional silent pre-download.
        // Primary-only (avoids N processes racing to download/stage the same
        // build) and a no-op on desktop (the JS plugin-updater owns that path).
        crate::updater::auto_check::spawn_auto_update_loop();

        // One-time migration: legacy flat-layout plan files
        // (`<plans>/plan-{short_id}-...md`) → per-session subdirs
        // (`<plans>/<agent>/<session>/plan-...md`). Idempotent — already-
        // nested files are left alone, so it's safe to run on every start.
        // Spawned as blocking because std::fs ops shouldn't tie up the
        // tokio runtime even during a one-off migration.
        tokio::task::spawn_blocking(crate::plan::migrate_flat_plans_to_subdirs);

        // Mirror the embedded user manual to <data-dir>/manual/ for the
        // `ha-manual` skill's read/grep path. Idempotent (fingerprint marker
        // short-circuits), primary-only (shared data dir), off-runtime, and
        // failure is non-fatal — the GUI reads the embedded bytes directly
        // and the skill re-triggers a lazy ensure on activation.
        tokio::task::spawn_blocking(|| {
            crate::manual::ensure_local_manual();
        });

        // Best-effort backfill of hook transcript mirrors (`§10`) for sessions
        // that predate the feature. Primary-only (writes shared session dirs)
        // and off-runtime (blocking fs + sqlite). Idempotent: sessions that
        // already have a transcript are skipped.
        if let Some(session_db) = SESSION_DB.get() {
            let db = session_db.clone();
            tokio::task::spawn_blocking(
                move || match crate::hooks::TranscriptMirror::backfill_all(&db) {
                    Ok(n) if n > 0 => {
                        app_info!(
                            "hooks",
                            "transcript",
                            "backfilled {} session transcript(s)",
                            n
                        )
                    }
                    Ok(_) => {}
                    Err(e) => app_warn!("hooks", "transcript", "transcript backfill failed: {e}"),
                },
            );
        }

        // Clean up the `ask_user_questions` table: drop old answered rows,
        // expire tool rows whose in-memory oneshots vanished on restart, and
        // re-arm durable owner-plane timeout tasks.
        tokio::spawn(async move {
            if let Some(db) = crate::get_session_db() {
                if let Err(e) = db.purge_old_answered_ask_user_groups(7) {
                    app_warn!(
                        "ask_user",
                        "startup",
                        "Failed to purge old ask_user rows: {}",
                        e
                    );
                }
            }

            // Tool-created rows cannot resume because the oneshot registry is
            // empty. Durable owner rows are preserved by this method and
            // restored immediately afterward.
            if let Some(db) = crate::get_session_db() {
                match db.expire_pending_ask_user_groups() {
                    Ok(0) => {}
                    Ok(n) => app_info!(
                        "ask_user",
                        "startup",
                        "Expired {} orphaned pending ask_user rows from previous process",
                        n
                    ),
                    Err(e) => app_warn!(
                        "ask_user",
                        "startup",
                        "Failed to expire pending ask_user rows: {}",
                        e
                    ),
                }
            }
            match crate::ask_user::restore_owner_question_timeouts() {
                Ok(0) => {}
                Ok(n) => app_info!(
                    "ask_user",
                    "startup",
                    "Re-armed {} durable owner ask_user timeout(s)",
                    n
                ),
                Err(e) => app_warn!(
                    "ask_user",
                    "startup",
                    "Failed to restore owner ask_user timeouts: {}",
                    e
                ),
            }
        });

        // Daily purge loop: keeps `ask_user_questions` bounded in long-running
        // server/launchd/systemd deployments where start_background_tasks only
        // runs once at boot.
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(crate::SECS_PER_DAY));
            ticker.tick().await; // skip immediate tick (startup path already purged)
            loop {
                ticker.tick().await;
                if let Some(db) = crate::get_session_db() {
                    if let Err(e) = db.purge_old_answered_ask_user_groups(7) {
                        app_warn!("ask_user", "purge", "Daily ask_user purge failed: {}", e);
                    }
                }
            }
        });

        // Auto-start enabled channel accounts. Two processes auto-starting
        // the same Telegram bot would fight over its webhook; users still
        // start accounts manually via the API/UI in any tier. Boot failures
        // are picked up by `channel::start_watchdog` and retried in the
        // background until success or user action.
        if let Some(registry) = CHANNEL_REGISTRY.get() {
            let registry = registry.clone();
            channel::start_watchdog::spawn_loop(registry.clone());
            let store = crate::config::cached_config();
            tokio::spawn(async move {
                for account in store.channels.enabled_accounts() {
                    if let Err(e) = registry.start_account(account).await {
                        channel::start_watchdog::register_failure(account, &e).await;
                    }
                }
            });
        }

        // Replay async tool jobs left over from the previous process: mark
        // `running` rows as interrupted (their host process is gone) and inject
        // any terminal-but-not-injected results back into their parent sessions.
        // Primary-only: a Secondary process running this would flip the
        // Primary's still-running tools to Interrupted.
        //
        // **Ordering invariant**: `recover_startup_session_state` ran
        // synchronously during `init_runtime` (before this background-task
        // bring-up), so by the time `replay_pending_jobs` starts, every
        // stale chat_turn has already been finalized (sentinel-aware
        // Shutdown / Crash) and any `ParentInjection` turn that the
        // replay schedules will see a coherent `context_json`.
        tokio::spawn(async move {
            crate::async_jobs::JobManager::replay_pending();
        });
        crate::workflow::spawn_startup_recovery_if_primary();
        crate::local_model_jobs::replay_interrupted_jobs();

        // Re-arm agent self-scheduled wakeups (R10). Primary-only — the rows
        // are shared, so a Secondary re-arming would double-deliver. Past-due
        // wakeups fire promptly.
        crate::wakeup::replay_pending();

        // Retention sweep for async_jobs (rows + spool files). Runs once at
        // startup and then once per day. Disabled entirely when both
        // `retention_secs` and `orphan_grace_secs` are `0`.
        crate::async_jobs::JobManager::spawn_retention_loop();
        // Durable chat journals remain available for diagnostics/replay for
        // 24 hours after terminal convergence, then cascade away without
        // touching canonical messages/context.
        spawn_chat_stream_journal_gc(true);

        // Retention sweep for recap session facets. Runs once at startup and
        // then once per day. Disabled when `recap.cache_retention_days == 0`.
        crate::recap::spawn_facet_retention_loop();

        // Retention sweep for the Dreaming pending-source queue + expired
        // locks (next-gen Dreaming Phase 0). Runs once at startup, then daily.
        crate::memory::dreaming::spawn_retention_loop();

        // Dreaming idle-trigger loop (Phase B3). Every minute, check whether
        // the app has been idle long enough and fire an offline consolidation
        // cycle. The DREAMING_RUNNING AtomicBool serialises within one
        // process — Primary-only here serialises across processes.
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // skip immediate tick
            loop {
                ticker.tick().await;
                let cfg = crate::config::cached_config().dreaming.clone();
                if crate::memory::dreaming::check_idle_trigger(&cfg) {
                    let profile_enabled = cfg.profile_synthesis.enabled;
                    tokio::spawn(async move {
                        let report = crate::memory::dreaming::manual_run(
                            crate::memory::dreaming::DreamTrigger::Idle,
                        )
                        .await;
                        app_info!(
                            "memory",
                            "dreaming::idle_trigger",
                            "idle-trigger cycle: scanned={}, promoted={}, note={:?}",
                            report.candidates_scanned,
                            report.promoted.len(),
                            report.note,
                        );
                        // After promotion releases the single-cycle guard, run a
                        // cheap rule-based Memory Profile synthesis (Phase 4) so
                        // the profile stays fresh without a separate trigger.
                        // Gated by `profile_synthesis.enabled` (on by default).
                        if profile_enabled {
                            let p = crate::memory::dreaming::run_profile_synthesis_cycle(
                                crate::memory::dreaming::DreamTrigger::Idle,
                            )
                            .await;
                            app_info!(
                                "memory",
                                "dreaming::idle_trigger",
                                "idle profile synthesis: scanned={}, snapshots={}, note={:?}",
                                p.scanned,
                                p.snapshots_written,
                                p.note,
                            );
                        }
                    });
                }
                // Knowledge maintenance idle trigger (WS6) — same idle clock.
                let mcfg = crate::config::cached_config().knowledge_maintenance.clone();
                if crate::knowledge::maintenance::check_idle_trigger(&mcfg) {
                    tokio::spawn(async {
                        let report = crate::knowledge::maintenance::manual_run(
                            crate::knowledge::maintenance::MaintenanceTrigger::Idle,
                        )
                        .await;
                        app_info!(
                            "knowledge",
                            "maintenance::idle_trigger",
                            "idle-trigger cycle: generated={}, autoApplied={}, note={:?}",
                            report.generated,
                            report.auto_applied,
                            report.note,
                        );
                    });
                }
            }
        });

        // Dreaming cron-trigger loop. Reads `dreaming.cron_trigger` and
        // fires `manual_run(Cron)` on the configured schedule. Re-evaluates
        // on every `config:changed { category: "dreaming" }`.
        crate::memory::dreaming::spawn_dreaming_cron_loop();

        // Additive external memory providers reconcile off the chat hot path.
        // The loop filters out manual policies and every adapter remains
        // fail-closed when credentials, endpoint or capability checks fail.
        crate::memory::spawn_external_memory_provider_sync_loop();

        // Knowledge maintenance cron-trigger loop (WS6). Reads
        // `knowledge_maintenance.cron_trigger`; off unless the user enables it.
        crate::knowledge::maintenance::spawn_maintenance_cron_loop();

        // Optional skill draft consolidation loop. Re-reads the
        // auto-review config after every interval or config change.
        crate::skills::auto_review::curator::spawn_auto_curator_loop();

        // STT streaming-session GC. Sweeps abandoned sessions every 5
        // minutes — a front-end crash / lost connection between `start`
        // and `finalize` would otherwise leak the upstream WS forever.
        tokio::spawn(async {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let evicted = crate::stt::SttSessionManager::global().gc_idle();
                if evicted > 0 {
                    app_info!(
                        "stt",
                        "session-gc",
                        "evicted {} idle STT session(s)",
                        evicted
                    );
                }
            }
        });

        // Knowledge base index: reconcile every KB against disk (catches edits
        // made while the app was off) and start a live watcher per KB root so
        // external-vault edits stay indexed (D6).
        crate::knowledge::index::spawn_startup_reconcile();
        crate::knowledge::watcher::start_all_watchers();

        // 设计空间「关联代码仓库」落地文件监听：外部改动 → 产物标 stale（code→design 回灌）。
        crate::design::code_watcher::start_all_watchers();

        // One-shot reconciler for orphan project-scoped memory rows. The
        // delete_project cascade touches both `session.db` and `memory.db` and
        // cannot wrap them in a single transaction, so a crash between the two
        // can leave unreachable memory rows behind. Project deletion is
        // low-frequency, so a startup sweep is enough — no periodic timer.
        crate::project::reconcile::spawn_startup_reconciler();

        // Auto-discover ACP backends
        if let Some(acp_mgr) = ACP_MANAGER.get() {
            let store = crate::config::cached_config();
            if store.acp_control.enabled {
                let registry = acp_mgr.runtime_registry().clone();
                let acp_config = store.acp_control.clone();
                tokio::spawn(async move {
                    acp_control::registry::auto_discover_and_register(&registry, &acp_config).await;
                });
            }
        }
    }

    // Initialize the MCP subsystem. `init_global` is idempotent and the
    // catalog snippet must be visible to every process so all tiers see
    // MCP-namespaced tools. Watchdog (long-running reconnect loop) is
    // Primary-only — Secondary's idle catalog is enough.
    if init_mcp_subsystem() && primary {
        crate::mcp::watchdog::spawn_watchdog_loop();
    }

    // Default-model auto-maintenance watchdog. Self-heals stale Ollama
    // models (cold-started after `ollama stop`, OS reboot, daemon restart)
    // and surfaces missing-file alerts via `local_model:missing_alert`.
    // Primary-only because two processes preloading the same model would
    // wastefully race; secondaries see the same `running` state through
    // the shared Ollama daemon anyway.
    if primary {
        crate::local_llm::auto_maintainer::spawn_loop();
    }
}

/// ACP-shaped background tasks. ACP is a single-conversation-per-process
/// model spawned by an IDE — daily timers leak file handles, channel
/// auto-start has no IM out, dreaming makes no sense. Intentionally
/// **excludes**:
///   - daily ask_user purge loop
///   - daily async_jobs retention loop
///   - daily recap retention loop
///   - dreaming idle trigger loop
///   - channel auto-start
///   - cron scheduler (it would survive past the ACP exit)
///   - ACP backend auto-discover (we *are* the ACP backend)
///   - MCP watchdog (process is short-lived; init_global is enough)
///
/// Future maintainers: think before adding to this list. The point is to
/// stay small.
pub async fn start_minimal_background_tasks() {
    let primary = crate::runtime_lock::is_primary();

    // EventBus listeners — multi-subscriber-safe, tier-agnostic.
    spawn_channel_listeners();

    // Local Chrome Extension broker. Short-lived ACP processes may not need it,
    // but starting it here keeps browser owner-plane diagnostics consistent.
    crate::browser::BrowserExtensionBroker::spawn_global();

    // Session-lifecycle cleanup fan-out — tier-agnostic, required in
    // server / ACP too (they delete sessions but have no channel registry).
    crate::session::cleanup_watcher::spawn_session_cleanup_watcher();

    // Hooks registry initial load + hot-reload. Required in server / ACP modes
    // too (this fn is their only background-task entry): without it the global
    // registry stays empty and every dispatch is a no-op, contradicting the
    // "hooks run in desktop / server / ACP alike" contract.
    spawn_hooks_config_listener();

    // Memory embedding provider deferred (per-process, tier-agnostic). See
    // start_background_tasks for rationale.
    spawn_embedding_init();

    // R7.1 background-job scheduler (tier-agnostic: process-local queue, idempotent).
    tokio::spawn(async move {
        crate::async_jobs::JobManager::run_scheduler().await;
    });
    // R7.2: subagent reject→queue scheduler — promotes parked (`Queued`) spawns
    // as running children settle. Idempotent per process (mirrors run_scheduler).
    tokio::spawn(async move {
        crate::subagent::queue::run_subagent_scheduler().await;
    });
    // R8 follow-up: mirror background-subagent inner approvals onto their
    // projection label (running ⇄ awaiting_approval). Idempotent per process.
    crate::async_jobs::approval_projection_watcher::spawn_subagent_approval_projection_watcher();

    if primary {
        // Manual mirror for the `ha-manual` skill — same as the full-tier
        // startup (ACP agents activate skills too). Idempotent + non-fatal.
        tokio::task::spawn_blocking(|| {
            crate::manual::ensure_local_manual();
        });

        // One-shot ask_user table cleanup. Primary-only because Secondary
        // would expire the desktop's still-live pending questions.
        tokio::spawn(async move {
            if let Some(db) = crate::get_session_db() {
                if let Err(e) = db.purge_old_answered_ask_user_groups(7) {
                    app_warn!(
                        "ask_user",
                        "startup",
                        "Failed to purge old ask_user rows: {}",
                        e
                    );
                }
                if let Err(e) = db.expire_pending_ask_user_groups() {
                    app_warn!(
                        "ask_user",
                        "startup",
                        "Failed to expire pending ask_user rows: {}",
                        e
                    );
                }
            }
            if let Err(e) = crate::ask_user::restore_owner_question_timeouts() {
                app_warn!(
                    "ask_user",
                    "startup",
                    "Failed to restore owner ask_user timeouts: {}",
                    e
                );
            }
        });

        // Replay leftover async tool jobs. Primary-only: a Secondary ACP
        // booting alongside an active desktop would flip the desktop's
        // running tools to Interrupted.
        tokio::spawn(async move {
            crate::async_jobs::JobManager::replay_pending();
        });
        crate::workflow::spawn_startup_recovery_if_primary();
        crate::local_model_jobs::replay_interrupted_jobs();
        crate::loop_control::spawn_loop_event_trigger_watcher();
        // ACP processes are intentionally short-lived: run one retention
        // sweep, but do not install another daily timer.
        spawn_chat_stream_journal_gc(false);

        // Re-arm agent self-scheduled wakeups (R10). Primary-only (shared rows).
        crate::wakeup::replay_pending();
    }

    // MCP init (no watchdog). Tier-agnostic — ACP tool dispatch may hit
    // MCP-namespaced tools regardless of tier and `init_global` is
    // idempotent. Watchdog (long-running reconnect loop) skipped here
    // even when ACP is Primary because the process is short-lived; the
    // next process start re-runs init_global.
    let _ = init_mcp_subsystem();
}

/// Shared MCP bring-up used by both `start_background_tasks` and
/// `start_minimal_background_tasks`. Returns `true` when MCP was enabled
/// and `init_global` was called — the caller decides whether to also
/// spawn the long-running watchdog.
fn init_mcp_subsystem() -> bool {
    let store = crate::config::cached_config();
    let global = store.mcp_global.clone();
    let servers = store.mcp_servers.clone();
    if global.enabled {
        let enabled_count = servers.iter().filter(|s| s.enabled).count();
        crate::mcp::McpManager::init_global(global, servers);
        app_info!(
            "mcp",
            "init",
            "MCP subsystem initialized ({} enabled server(s))",
            enabled_count
        );
        true
    } else {
        app_info!(
            "mcp",
            "init",
            "MCP subsystem disabled via mcpGlobal.enabled=false"
        );
        false
    }
}

fn recover_durable_chat_streams(
    session_db: &Arc<SessionDB>,
    cause: crate::chat_engine::finalize::sentinel::StartupCause,
) {
    let mut spool_integrity_errors = std::collections::HashMap::<String, String>::new();
    // Import every complete emergency-spool frame first. The spool is only
    // deleted after the corresponding run converges transactionally below.
    match crate::chat_engine::spool::list_run_ids() {
        Ok(run_ids) => {
            for run_id in run_ids {
                match crate::chat_engine::spool::read_batches(&run_id) {
                    Ok(spool) => {
                        if let Some(error) = spool.integrity_error {
                            app_warn!(
                                "session",
                                "stream_recovery",
                                "emergency spool for run {} has a damaged tail: {}",
                                run_id,
                                error
                            );
                            spool_integrity_errors.insert(run_id.clone(), error);
                        }
                        if !spool.batches.is_empty() {
                            if let Err(group_error) =
                                session_db.append_stream_journal_batches(&spool.batches)
                            {
                                // Exceptional corruption/mismatch path: retain
                                // the largest valid prefix instead of making
                                // the healthy frames after SQLite's existing
                                // prefix all-or-nothing.
                                app_warn!(
                                    "session",
                                    "stream_recovery",
                                    "group spool import failed for run {}, scanning prefix: {}",
                                    run_id,
                                    group_error
                                );
                                for batch in spool.batches {
                                    if let Err(error) =
                                        session_db.append_stream_journal_batch(&batch)
                                    {
                                        app_warn!(
                                            "session",
                                            "stream_recovery",
                                            "spool import stopped for run {} at seq {}..{}: {}",
                                            run_id,
                                            batch.seq_start,
                                            batch.seq_end,
                                            error
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => app_warn!(
                        "session",
                        "stream_recovery",
                        "cannot read emergency spool for run {}: {}",
                        run_id,
                        error
                    ),
                }
            }
        }
        Err(error) => app_warn!(
            "session",
            "stream_recovery",
            "cannot enumerate emergency stream spool: {}",
            error
        ),
    }

    let runs = match session_db.recoverable_stream_runs() {
        Ok(runs) => runs,
        Err(error) => {
            app_warn!(
                "session",
                "stream_recovery",
                "cannot enumerate unfinished stream runs: {}",
                error
            );
            return;
        }
    };
    for run in runs {
        let Some(snapshot) = session_db
            .stream_run_snapshot(&run.run_id)
            .unwrap_or_else(|error| {
                app_warn!(
                    "session",
                    "stream_recovery",
                    "cannot load stream run {}: {}",
                    run.run_id,
                    error
                );
                None
            })
        else {
            continue;
        };

        // One shared selector is used by live failure convergence, ACP, and
        // startup replay so attempt choice and corruption truncation cannot
        // drift between entry points.
        let (selected_attempt, through_seq, events, journal_error) =
            crate::session::select_recoverable_attempt_prefix(&snapshot);
        let selected_attempt_meta = snapshot
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_no == selected_attempt);
        let provider_kind = selected_attempt_meta
            .and_then(|attempt| attempt.provider_shape.as_deref())
            .or(snapshot.run.provider_shape.as_deref())
            .and_then(crate::chat_engine::finalize::ProviderApiKind::from_shape);
        let recovery_bytes = events.iter().map(|event| event.event.len()).sum::<usize>();
        let spool_damaged = spool_integrity_errors.contains_key(&run.run_id);
        let recovery_error =
            journal_error.or_else(|| spool_integrity_errors.get(&run.run_id).cloned());
        let trailing_text = crate::session::trailing_text_from_journal_events(&events);

        let (context_json, context_checkpoint_seq, context_revision) = match session_db
            .recovery_context_for_prefix(&run.run_id, selected_attempt, through_seq)
        {
            Ok(value) => value,
            Err(error) => {
                app_warn!(
                    "session",
                    "stream_recovery",
                    "cannot load trusted context checkpoint for run {}: {}",
                    run.run_id,
                    error
                );
                continue;
            }
        };
        let mut history: Vec<serde_json::Value> = context_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        let reason = cause.to_termination_reason();
        if let Err(error) = crate::chat_engine::finalize::rebuild::append_journal_suffix_to_history(
            &mut history,
            &events,
            context_checkpoint_seq,
            provider_kind,
        ) {
            app_warn!(
                "session",
                "stream_recovery",
                "cannot rebuild provider-native journal suffix for run {}: {}",
                run.run_id,
                error
            );
            continue;
        }
        history.push(serde_json::json!({
            "role": "assistant",
            "content": crate::chat_engine::finalize::copy::model_marker(&reason),
        }));
        let context_json = match serde_json::to_string(&history) {
            Ok(json) => json,
            Err(error) => {
                app_warn!(
                    "session",
                    "stream_recovery",
                    "cannot serialize recovered context for run {}: {}",
                    run.run_id,
                    error
                );
                continue;
            }
        };
        let source = crate::chat_engine::ChatSource::from_db_string(&run.source);
        let assistant = crate::session::journal_events_have_assistant_output(&events).then(|| {
            let mut message = crate::session::NewMessage::assistant(&trailing_text);
            message.source = Some(source.as_str().to_string());
            message
        });
        let mut recovery_event =
            if cause == crate::chat_engine::finalize::sentinel::StartupCause::Crash {
                crate::session::NewMessage::error_event(
                    &crate::chat_engine::finalize::copy::user_notice(&reason),
                )
            } else {
                crate::session::NewMessage::event(&crate::chat_engine::finalize::copy::user_notice(
                    &reason,
                ))
            };
        recovery_event.source = Some(source.as_str().to_string());
        let commit = crate::session::CommitInterruptedTurn {
            run_id: Some(run.run_id.clone()),
            attempt_no: selected_attempt,
            session_id: run.session_id.clone(),
            assistant,
            context_json,
            expected_context_revision: context_revision,
            turn_id: run.turn_id.clone(),
            final_seq: through_seq,
            status: crate::session::ChatTurnStatus::Interrupted,
            interrupt_reason: Some(match cause {
                crate::chat_engine::finalize::sentinel::StartupCause::Clean => {
                    "shutdown".to_string()
                }
                crate::chat_engine::finalize::sentinel::StartupCause::Crash => {
                    "crash_recovery".to_string()
                }
            }),
            error: recovery_error.clone(),
            recovery_event: Some(recovery_event),
        };
        match session_db.commit_interrupted_turn(&commit) {
            Ok(_) => {
                let spool_cleanup = if spool_damaged {
                    crate::chat_engine::spool::quarantine(&run.run_id)
                } else {
                    crate::chat_engine::spool::remove(&run.run_id)
                };
                if let Err(error) = spool_cleanup {
                    app_warn!(
                        "session",
                        "stream_recovery",
                        "recovered run {} but could not archive/remove spool: {}",
                        run.run_id,
                        error
                    );
                }
                app_info!(
                    "session",
                    "stream_recovery",
                    "recovered durable stream run {} through seq {} bytes={}{}",
                    run.run_id,
                    through_seq,
                    recovery_bytes,
                    if recovery_error.is_some() {
                        " (truncated at corruption/gap)"
                    } else {
                        ""
                    }
                );
            }
            Err(error) => app_warn!(
                "session",
                "stream_recovery",
                "failed to converge durable stream run {}: {}",
                run.run_id,
                error
            ),
        }
    }

    // Crash window: the final DB transaction may have committed immediately
    // before the process died, leaving its spool file behind. Such a run is no
    // longer in `recoverable_stream_runs`, so converge the leftover file here
    // instead of re-importing it forever on every startup.
    if let Ok(run_ids) = crate::chat_engine::spool::list_run_ids() {
        for run_id in run_ids {
            match session_db.stream_run_status(&run_id) {
                Ok(Some(status)) if status != "running" => {
                    let result = if spool_integrity_errors.contains_key(&run_id) {
                        crate::chat_engine::spool::quarantine(&run_id)
                    } else {
                        crate::chat_engine::spool::remove(&run_id)
                    };
                    if let Err(error) = result {
                        app_warn!(
                            "session",
                            "stream_recovery",
                            "cannot clean terminal leftover spool for run {}: {}",
                            run_id,
                            error
                        );
                    }
                }
                Ok(None) => {
                    if let Err(error) = crate::chat_engine::spool::quarantine(&run_id) {
                        app_warn!(
                            "session",
                            "stream_recovery",
                            "cannot quarantine orphan stream spool {}: {}",
                            run_id,
                            error
                        );
                    }
                }
                Ok(Some(_)) => {}
                Err(error) => app_warn!(
                    "session",
                    "stream_recovery",
                    "cannot inspect leftover spool run {}: {}",
                    run_id,
                    error
                ),
            }
        }
    }
}

fn spawn_chat_stream_journal_gc(repeat_daily: bool) {
    let Some(db) = crate::get_session_db() else {
        return;
    };
    tokio::spawn(async move {
        loop {
            let cutoff = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
            let gc_db = db.clone();
            match gc_db.run(move |db| db.gc_stream_journals(&cutoff)).await {
                Ok(count) if count > 0 => app_info!(
                    "session",
                    "stream_gc",
                    "removed {} terminal stream journal run(s)",
                    count
                ),
                Ok(_) => {}
                Err(error) => app_warn!(
                    "session",
                    "stream_gc",
                    "stream journal retention sweep failed: {}",
                    error
                ),
            }
            match crate::blocking::run_blocking(|| {
                crate::chat_engine::spool::gc_quarantined(std::time::Duration::from_secs(
                    24 * 60 * 60,
                ))
            })
            .await
            {
                Ok(count) if count > 0 => app_info!(
                    "session",
                    "stream_gc",
                    "removed {} quarantined stream spool file(s)",
                    count
                ),
                Ok(_) => {}
                Err(error) => app_warn!(
                    "session",
                    "stream_gc",
                    "stream spool retention sweep failed: {}",
                    error
                ),
            }
            if !repeat_daily {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(crate::SECS_PER_DAY)).await;
        }
    });
}

fn recover_startup_session_state(session_db: &Arc<SessionDB>, tier: crate::runtime_lock::Tier) {
    if tier != crate::runtime_lock::Tier::Primary {
        return;
    }

    match session_db.reconcile_interrupted_project_bootstraps() {
        Ok(0) => {}
        Ok(count) => app_info!(
            "project_bootstrap",
            "startup_recovery",
            "reconciled {} interrupted project bootstrap run(s)",
            count
        ),
        Err(error) => app_warn!(
            "project_bootstrap",
            "startup_recovery",
            "failed to reconcile interrupted project bootstraps: {error:#}"
        ),
    }

    match session_db.reconcile_interrupted_git_operations() {
        Ok(0) => {}
        Ok(count) => app_info!(
            "git_control",
            "startup_recovery",
            "reconciled {} interrupted Git operation(s)",
            count
        ),
        Err(error) => app_warn!(
            "git_control",
            "startup_recovery",
            "failed to reconcile interrupted Git operations: {error:#}"
        ),
    }

    // Read the shutdown sentinel: present → previous process exited
    // cleanly (signal handler ran), absent → crash/SIGKILL/power loss.
    // Drives which `TerminationReason` we attach to each stale turn.
    let cause = crate::chat_engine::finalize::sentinel::read_and_clear();
    app_info!(
        "session",
        "startup_recovery",
        "Startup cause from sentinel: {}",
        cause.as_str(),
    );

    // 1. Sweep stale `streaming` placeholder rows into `orphaned` so the
    //    finalize reverse-rebuild can recognize them as interrupted.
    match session_db.mark_orphaned_streaming_rows() {
        Ok(0) => {}
        Ok(n) => app_info!(
            "session",
            "stream_persist",
            "promoted {} leftover streaming row(s) to orphaned on startup",
            n
        ),
        Err(e) => app_warn!(
            "session",
            "stream_persist",
            "startup orphan sweep failed: {}",
            e
        ),
    }

    // New journal/spool recovery runs before stale chat_turn convergence so
    // the legacy sweep sees those turns already terminal and cannot overwrite
    // the richer durable reconstruction.
    recover_durable_chat_streams(session_db, cause);

    // 2. Collect every chat_turn left in `running` / `cancelling` state.
    //    finalize will set the terminal status itself; we don't UPDATE
    //    here to avoid the historical "crash_recovery for everything"
    //    behavior the new sentinel-aware path replaces.
    let stale = match session_db.find_stale_chat_turns_for_finalize() {
        Ok(rows) => rows,
        Err(e) => {
            app_warn!(
                "session",
                "turn",
                "startup find_stale_chat_turns_for_finalize failed: {}",
                e
            );
            Vec::new()
        }
    };

    let db_arc = session_db.clone();
    let mut finalized = 0usize;
    let mut covered_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    for turn in &stale {
        let provider_kind =
            crate::chat_engine::finalize::rebuild::resolve_provider_kind_for_session(
                &db_arc,
                &turn.session_id,
            );
        let partial = crate::chat_engine::finalize::rebuild::collect_partial_from_messages(
            &db_arc,
            &turn.session_id,
            provider_kind,
        );
        let reason = cause.to_termination_reason();
        let source = crate::chat_engine::ChatSource::from_db_string(&turn.source);
        let partial = crate::chat_engine::finalize::PartialMeta {
            turn_id: Some(turn.id.clone()),
            ..partial
        };
        let outcome = crate::chat_engine::finalize::finalize_turn_context_blocking(
            &db_arc,
            &turn.session_id,
            reason,
            partial,
            source,
        );
        if !outcome.was_already_finalized {
            finalized += 1;
        }
        covered_sessions.insert(turn.session_id.clone());
    }

    // IM / Cron / Subagent entry points run with `turn_id = None`, so
    // they leave no `chat_turns` row for the sweep above to act on.
    // Their partial output still ends up in `messages` with
    // `stream_status='orphaned'` after the previous step's promotion,
    // and without finalize the next restore for those sessions would
    // load a `context_json` that never received the markers / synthetic
    // tool_results for those orphans, and the GUI would render the
    // tool rows as forever-running. Sweep them here with `turn_id=None`
    // so finalize writes the marker + closes the tool rows + appends
    // an event banner.
    let orphan_session_ids = db_arc.sessions_with_orphaned_rows().unwrap_or_else(|e| {
        app_warn!(
            "session",
            "startup_recovery",
            "sessions_with_orphaned_rows failed: {}",
            e
        );
        Vec::new()
    });
    for session_id in orphan_session_ids {
        if covered_sessions.contains(&session_id) {
            continue;
        }
        let reason = cause.to_termination_reason();
        let startup_notices = [
            crate::chat_engine::finalize::copy::user_notice(
                &crate::chat_engine::finalize::TerminationReason::Crash,
            ),
            crate::chat_engine::finalize::copy::user_notice(
                &crate::chat_engine::finalize::TerminationReason::Shutdown,
            ),
        ];
        let already_finalized = startup_notices.iter().try_fold(false, |found, notice| {
            if found {
                Ok(true)
            } else {
                db_arc.current_turn_orphaned_has_later_event(&session_id, notice)
            }
        });
        match already_finalized {
            Ok(true) => {
                match db_arc.mark_current_turn_orphaned_rows_recovered(&session_id) {
                    Ok(0) => {}
                    Ok(n) => app_info!(
                        "session",
                        "startup_recovery",
                        "marked {} previously-finalized orphaned row(s) recovered for session {}",
                        n,
                        session_id
                    ),
                    Err(e) => app_warn!(
                        "session",
                        "startup_recovery",
                        "failed to mark orphaned rows recovered for session {}: {}",
                        session_id,
                        e
                    ),
                }
                continue;
            }
            Ok(false) => {}
            Err(e) => app_warn!(
                "session",
                "startup_recovery",
                "current_turn_orphaned_has_later_event failed for session {}: {}",
                session_id,
                e
            ),
        }
        let provider_kind =
            crate::chat_engine::finalize::rebuild::resolve_provider_kind_for_session(
                &db_arc,
                &session_id,
            );
        let partial = crate::chat_engine::finalize::rebuild::collect_partial_from_messages(
            &db_arc,
            &session_id,
            provider_kind,
        );
        if partial.text.is_none() && partial.thinking.is_none() && partial.tool_calls.is_empty() {
            // Nothing to rebuild — the orphaned rows didn't carry
            // visible content. Skip to avoid emitting a marker on
            // empty sessions.
            continue;
        }
        let outcome = crate::chat_engine::finalize::finalize_turn_context_blocking(
            &db_arc,
            &session_id,
            reason,
            partial,
            // No source signal for these sessions — pick the safest
            // default for the event row's `source` column. Same value
            // the from_db_string fallback returns.
            crate::chat_engine::ChatSource::Desktop,
        );
        if !outcome.was_already_finalized {
            finalized += 1;
        }
    }

    let cleared = crate::chat_engine::active_turn::clear_all();
    if finalized > 0 || cleared > 0 {
        app_info!(
            "session",
            "startup_recovery",
            "finalized {} stale chat turn(s) ({:?}); cleared {} active turn registry entr{}",
            finalized,
            cause,
            cleared,
            if cleared == 1 { "y" } else { "ies" },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ChatTurnInterruptReason, ChatTurnStatus, NewMessage};

    fn with_temp_data_dir<T>(f: impl FnOnce(Arc<SessionDB>) -> T) -> T {
        let dir = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", dir.path())], || {
            let path = dir.path().join("sessions.db");
            let db = Arc::new(SessionDB::open(&path).expect("open session db"));
            f(db)
        })
    }

    #[test]
    fn secondary_startup_recovery_does_not_mutate_shared_session_state() {
        with_temp_data_dir(|db| {
            let session = db
                .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, None)
                .expect("create session");
            let turn = db
                .create_chat_turn(&session.id, "desktop", Some("stream-1"), None)
                .expect("create turn");
            let mut streaming = NewMessage::text_block("partial");
            streaming.stream_status = Some("streaming".to_string());
            db.append_message(&session.id, &streaming)
                .expect("append streaming message");

            recover_startup_session_state(&db, crate::runtime_lock::Tier::Secondary);

            let persisted_turn = db
                .get_chat_turn(&turn.id)
                .expect("load turn")
                .expect("turn exists");
            assert_eq!(persisted_turn.status, ChatTurnStatus::Running);
            assert!(persisted_turn.interrupt_reason.is_none());

            let messages = db
                .load_session_messages(&session.id)
                .expect("load messages");
            assert_eq!(messages[0].stream_status.as_deref(), Some("streaming"));
        });
    }

    #[test]
    fn primary_startup_recovery_marks_stale_state_recovers_rows_and_clears_active_turns() {
        with_temp_data_dir(|db| {
            let _lock = crate::chat_engine::active_turn::test_lock();
            let session = db
                .create_session_with_project(crate::agent_loader::DEFAULT_AGENT_ID, None, None)
                .expect("create session");
            let turn = db
                .create_chat_turn(&session.id, "desktop", Some("stream-1"), None)
                .expect("create turn");
            let mut streaming = NewMessage::text_block("partial");
            streaming.stream_status = Some("streaming".to_string());
            db.append_message(&session.id, &streaming)
                .expect("append streaming message");

            let _guard = crate::chat_engine::active_turn::try_acquire(
                &session.id,
                crate::chat_engine::stream_seq::ChatSource::Desktop,
                turn.id.clone(),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("acquire active turn");

            recover_startup_session_state(&db, crate::runtime_lock::Tier::Primary);

            let persisted_turn = db
                .get_chat_turn(&turn.id)
                .expect("load turn")
                .expect("turn exists");
            assert_eq!(persisted_turn.status, ChatTurnStatus::Interrupted);
            assert_eq!(
                persisted_turn.interrupt_reason,
                Some(ChatTurnInterruptReason::CrashRecovery)
            );

            let messages = db
                .load_session_messages(&session.id)
                .expect("load messages");
            assert_eq!(messages[0].stream_status.as_deref(), Some("recovered"));
            assert!(crate::chat_engine::active_turn::current(&session.id).is_none());

            // Ensure the dropped guard cannot resurrect a cleared entry.
            drop(_guard);
            assert!(crate::chat_engine::active_turn::current(&session.id).is_none());
        });
    }
}
