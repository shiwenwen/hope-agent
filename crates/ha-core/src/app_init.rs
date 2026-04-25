use crate::acp_control;
use crate::channel;
use crate::cron;
use crate::globals::AppState;
use crate::globals::{
    ACP_MANAGER, APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY,
    CODEX_TOKEN_CACHE, CRON_DB, EVENT_BUS, IDLE_EXTRACT_HANDLES, LOG_DB, MEMORY_BACKEND,
    PROJECT_DB, REASONING_EFFORT, SESSION_DB, SUBAGENT_CANCELS,
};
use crate::logging::{self, AppLogger, LogDB};
use crate::memory;
use crate::paths;
use crate::project::ProjectDB;
use crate::session::{self, SessionDB};
use crate::subagent;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Initialize all databases, subsystems, and construct the `AppState`.
pub fn init_app_state() -> AppState {
    /// Unwrap a Result or print a fatal error to stderr and panic.
    fn fatal<T>(result: anyhow::Result<T>, msg: &str) -> T {
        result.unwrap_or_else(|e| {
            eprintln!("[FATAL] {msg}: {e}");
            panic!("{msg}: {e}");
        })
    }

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

    // Initialize the ProjectDB (shares the SessionDB SQLite connection).
    // Run its table-creation migration so `projects` / `project_files` exist
    // before any command touches them.
    let project_db = Arc::new(ProjectDB::new(session_db.clone()));
    if let Err(e) = project_db.migrate() {
        eprintln!("[FATAL] Cannot run project DB migration: {e}");
        panic!("project DB migration failed: {e}");
    }
    let _ = PROJECT_DB.set(project_db.clone());

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
    let logger = AppLogger::new(log_db.clone(), logs_dir);
    logger.update_config(log_config);

    // Store logger globally for access from non-State contexts
    let _ = APP_LOGGER.set(logger.clone());

    // Initialize the MemoryDB
    let memory_db_path = fatal(
        paths::memory_db_path(),
        "Cannot resolve memory database path",
    );
    let memory_backend: Arc<dyn memory::MemoryBackend> = Arc::new(fatal(
        memory::SqliteMemoryBackend::open(&memory_db_path),
        "Cannot open memory database",
    ));
    let _ = MEMORY_BACKEND.set(memory_backend);

    // Auto-initialize embedder if enabled in config
    if let Some(backend) = MEMORY_BACKEND.get() {
        match crate::config::load_config() {
            Ok(store) if store.embedding.enabled => {
                match memory::create_embedding_provider(&store.embedding) {
                    Ok(emb_provider) => {
                        backend.set_embedder(emb_provider);
                        logger.log(
                            "info",
                            "memory",
                            "embedding",
                            "Embedding provider auto-initialized on startup",
                            None,
                            None,
                            None,
                        );
                    }
                    Err(e) => {
                        logger.log(
                            "warn",
                            "memory",
                            "embedding",
                            &format!("Failed to auto-initialize embedding provider: {}", e),
                            None,
                            None,
                            None,
                        );
                    }
                }
            }
            _ => {} // Embedding not enabled or config load failed — skip silently
        }
    }

    // Initialize the CronDB (scheduler started in .setup() where tokio runtime is available)
    let cron_db_path = fatal(paths::cron_db_path(), "Cannot resolve cron database path");
    let cron_db = Arc::new(fatal(
        cron::CronDB::open(&cron_db_path),
        "Cannot open cron database",
    ));
    let _ = CRON_DB.set(cron_db.clone());

    // Failure here is non-fatal — async tools degrade to sync mode if the DB cannot be opened.
    match paths::async_jobs_db_path().and_then(|p| crate::async_jobs::AsyncJobsDB::open(&p)) {
        Ok(db) => crate::async_jobs::set_async_jobs_db(Arc::new(db)),
        Err(e) => crate::app_warn!(
            "async_jobs",
            "init",
            "Failed to open async_jobs DB ({}); async tool backgrounding disabled",
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

    // Send welcome notification on startup via EventBus
    if let Some(bus) = EVENT_BUS.get() {
        let payload = serde_json::json!({
            "type": "agent_notification",
            "title": "欢迎使用 Hope Agent",
            "body": "文文，准备好开始今天的工作了吗？",
        });
        let _ = bus.emit("agent:send_notification", payload);
    }

    // Initialize sub-agent cancel registry
    let subagent_cancels = Arc::new(subagent::SubagentCancelRegistry::new());
    let _ = SUBAGENT_CANCELS.set(subagent_cancels.clone());
    let _ = SESSION_DB.set(session_db.clone());
    let _ = IDLE_EXTRACT_HANDLES.set(std::sync::Mutex::new(std::collections::HashMap::new()));

    // Published to OnceLocks here, shared into AppState below. The two
    // access styles must see the same Arc — `debug_assert!` at the bottom
    // of this function enforces it.
    let channel_cancels = Arc::new(channel::ChannelCancelRegistry::new());
    let codex_token = Arc::new(Mutex::new(None::<(String, String)>));
    let reasoning_effort = Arc::new(Mutex::new("medium".to_string()));
    let cached_agent = Arc::new(Mutex::new(None::<crate::agent::AssistantAgent>));
    let _ = CHANNEL_CANCELS.set(channel_cancels.clone());
    let _ = CODEX_TOKEN_CACHE.set(codex_token.clone());
    let _ = REASONING_EFFORT.set(reasoning_effort.clone());
    let _ = CACHED_AGENT.set(cached_agent.clone());

    // Clean up orphan sub-agent runs from previous app session
    subagent::cleanup_orphan_runs(&session_db);

    // Clean up orphan team members from previous app session
    crate::team::cleanup::cleanup_orphan_teams(&session_db);

    // Backstop the live close-on-leave path: incognito sessions left from a
    // crash / SIGKILL / power loss never reach the frontend purge call.
    crate::session::cleanup_orphan_incognito(&session_db);

    // Initialize IM Channel system
    {
        let (mut registry, inbound_rx) = channel::ChannelRegistry::new(256);

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

    let state = AppState {
        agent: cached_agent,
        auth_result: Arc::new(Mutex::new(None)),
        reasoning_effort,
        codex_token,
        current_agent_id: Mutex::new("default".to_string()),
        session_db,
        project_db,
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

fn ptr_eq_lock<T>(lock: &std::sync::OnceLock<Arc<T>>, field: &Arc<T>) -> bool {
    lock.get()
        .map(|arc| Arc::ptr_eq(arc, field))
        .unwrap_or(false)
}

/// Start background async tasks that require a tokio runtime.
/// Must be called from within a tokio async context (e.g., Tauri's `.setup()` or a server runtime).
pub async fn start_background_tasks() {
    // IM channel approval / ask_user listeners. These use bare `tokio::spawn`
    // internally, so they live here (post-runtime) rather than in
    // `init_app_state` (which can run on a sync stack). Idempotent — the
    // listeners early-return if `get_event_bus()` is None.
    if let (Some(channel_db), Some(registry)) = (CHANNEL_DB.get(), CHANNEL_REGISTRY.get()) {
        channel::worker::approval::spawn_channel_approval_listener(
            channel_db.clone(),
            registry.clone(),
        );
        channel::worker::ask_user::spawn_channel_ask_user_listener(
            channel_db.clone(),
            registry.clone(),
        );
    }

    // Cron scheduler: `start_scheduler` itself spawns a dedicated OS thread
    // with its own multi-thread tokio runtime (see scheduler.rs), so we just
    // call it here and let the returned JoinHandle detach. Used to live in
    // `src-tauri/src/setup.rs`; centralising it here means server / acp
    // entrypoints don't have to remember to start cron separately.
    if let (Some(cron_db), Some(session_db)) = (CRON_DB.get(), SESSION_DB.get()) {
        let _handle = cron::start_scheduler(cron_db.clone(), session_db.clone());
    }

    // Clean up the `ask_user_questions` table: drop old answered rows and
    // expire any still-pending rows left behind by a previous process
    // (their in-memory oneshots are gone, so the UI could not deliver
    // answers to them anyway).
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

        // Expire any rows left pending by a previous process. The in-memory
        // oneshot registry is empty at startup, so a "resume" would produce
        // orphaned UI entries whose submissions fail with "No pending plan
        // question request".
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
    });

    // Daily purge loop: keeps `ask_user_questions` bounded in long-running
    // server/launchd/systemd deployments where start_background_tasks only
    // runs once at boot.
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(crate::SECS_PER_DAY));
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

    // Auto-start enabled channel accounts
    if let Some(registry) = CHANNEL_REGISTRY.get() {
        let registry = registry.clone();
        let store = crate::config::cached_config();
        tokio::spawn(async move {
            for account in store.channels.enabled_accounts() {
                if let Err(e) = registry.start_account(account).await {
                    app_error!(
                        "channel",
                        "init",
                        "Failed to auto-start channel account '{}': {}",
                        account.label,
                        e
                    );
                }
            }
        });
    }

    // Replay async tool jobs left over from the previous process: mark
    // `running` rows as interrupted (their host process is gone) and inject
    // any terminal-but-not-injected results back into their parent sessions.
    tokio::spawn(async move {
        crate::async_jobs::replay_pending_jobs();
    });

    // Retention sweep for async_jobs (rows + spool files). Runs once at
    // startup and then once per day. Disabled entirely when both
    // `retention_secs` and `orphan_grace_secs` are `0`.
    crate::async_jobs::spawn_retention_loop();

    // Retention sweep for recap session facets. Runs once at startup and
    // then once per day. Disabled when `recap.cache_retention_days == 0`.
    crate::recap::spawn_facet_retention_loop();

    // Dreaming idle-trigger loop (Phase B3). Every minute, check whether
    // the app has been idle long enough and fire an offline consolidation
    // cycle. The cycle itself serialises through a global AtomicBool so
    // overlapping triggers (idle + manual) are safe.
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // skip immediate tick
        loop {
            ticker.tick().await;
            let cfg = crate::config::cached_config().dreaming.clone();
            if crate::memory::dreaming::check_idle_trigger(&cfg) {
                tokio::spawn(async {
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
                });
            }
        }
    });

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

    // Initialize the MCP subsystem. `init_global` is idempotent — safe to
    // call from both the Tauri desktop shell and `hope-agent server`; the
    // second call is a no-op. Must happen before any tool dispatch, so
    // we do it here (start_background_tasks runs before chat sessions).
    {
        let store = crate::config::cached_config();
        let global = store.mcp_global.clone();
        let servers = store.mcp_servers.clone();
        if global.enabled {
            let enabled_count = servers.iter().filter(|s| s.enabled).count();
            crate::mcp::McpManager::init_global(global, servers);
            // Watchdog owns periodic reconnect + eager warm-up. Spawned
            // once per process — subsequent `start_background_tasks`
            // calls are rare but harmless.
            crate::mcp::watchdog::spawn_watchdog_loop();
            app_info!(
                "mcp",
                "init",
                "MCP subsystem initialized ({} enabled server(s))",
                enabled_count
            );
        } else {
            app_info!(
                "mcp",
                "init",
                "MCP subsystem disabled via mcpGlobal.enabled=false"
            );
        }
    }
}
