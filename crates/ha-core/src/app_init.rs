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
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// Sentinel marking that `init_runtime` has run successfully. Set last so
/// that any fatal panic during init leaves it clear and a retry attempt is
/// distinguishable from a successful first run.
static INIT_DONE: OnceLock<()> = OnceLock::new();

/// Initialize all global singletons (databases, OnceLocks, channel registry,
/// ACP control plane, orphan cleanup, embedder, welcome log). Idempotent —
/// the second call is a no-op so dev hot-reload and accidental double-call
/// don't reopen DB files or double-register channel plugins.
///
/// Side effects only — does not construct `AppState`. Desktop callers should
/// follow up with `build_app_state()`. Server / ACP modes don't need
/// `AppState` and stop here.
pub fn init_runtime(role: &str) {
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

    // Elect Primary / Secondary across the data dir. Tier is captured
    // here but logged via `app_info!` further down once APP_LOGGER is
    // initialised; commit C8 wires the cleanup + single-owner loop
    // gating against `runtime_lock::is_primary()`.
    let tier = crate::runtime_lock::acquire_or_secondary(role);

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

    // Auto-initialize memory embedding model if enabled in config
    if let Some(backend) = MEMORY_BACKEND.get() {
        match crate::config::load_config() {
            Ok(store) if store.memory_embedding.enabled => {
                match memory::resolve_memory_embedding_config(
                    &store.memory_embedding,
                    &store.embedding_models,
                )
                .and_then(|resolved| {
                    resolved
                        .map(|(_, config, _)| memory::create_embedding_provider(&config))
                        .transpose()
                }) {
                    Ok(Some(emb_provider)) => {
                        backend.set_embedder(emb_provider);
                        logger.log(
                            "info",
                            "memory",
                            "embedding",
                            "Memory embedding provider auto-initialized on startup",
                            None,
                            None,
                            None,
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        logger.log(
                            "warn",
                            "memory",
                            "embedding",
                            &format!("Failed to auto-initialize memory embedding provider: {}", e),
                            None,
                            None,
                            None,
                        );
                    }
                }
            }
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

    // Initialize the CronDB (scheduler started in start_background_tasks)
    let cron_db_path = fatal(paths::cron_db_path(), "Cannot resolve cron database path");
    let cron_db = Arc::new(fatal(
        cron::CronDB::open(&cron_db_path),
        "Cannot open cron database",
    ));
    let _ = CRON_DB.set(cron_db);

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

    // Send welcome notification on startup via EventBus
    if let Some(bus) = EVENT_BUS.get() {
        let payload = serde_json::json!({
            "type": "agent_notification",
            "title": "欢迎使用 Hope Agent",
            "body": "文文，准备好开始今天的工作了吗？",
        });
        let _ = bus.emit("agent:send_notification", payload);
    }

    // Sub-agent cancel registry + idle-extract handle map
    let _ = SUBAGENT_CANCELS.set(Arc::new(subagent::SubagentCancelRegistry::new()));
    let _ = IDLE_EXTRACT_HANDLES.set(std::sync::Mutex::new(std::collections::HashMap::new()));

    // Per-AppState fields that desktop reads via OnceLock too. Constructed
    // here so server / acp modes get the same defaults without depending on
    // `build_app_state`.
    let _ = CHANNEL_CANCELS.set(Arc::new(channel::ChannelCancelRegistry::new()));
    let _ = CODEX_TOKEN_CACHE.set(Arc::new(Mutex::new(None::<(String, String)>)));
    let _ = REASONING_EFFORT.set(Arc::new(Mutex::new("medium".to_string())));
    let _ = CACHED_AGENT.set(Arc::new(Mutex::new(None::<crate::agent::AssistantAgent>)));

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
    }

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
pub async fn start_background_tasks() {
    let primary = crate::runtime_lock::is_primary();

    // Tier-agnostic: EventBus subscription is multi-subscriber-safe.
    spawn_channel_listeners();

    if primary {
        // Cron scheduler self-hosts a dedicated OS thread with its own tokio
        // runtime (see scheduler.rs). Primary-only because the periodic
        // tick's `claim_scheduled_job_for_execution` would double-claim
        // jobs across processes; manual run-now uses an atomic SQL claim
        // that's still safe in any tier.
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
        tokio::spawn(async move {
            crate::async_jobs::replay_pending_jobs();
        });
        crate::local_model_jobs::replay_interrupted_jobs();

        // Retention sweep for async_jobs (rows + spool files). Runs once at
        // startup and then once per day. Disabled entirely when both
        // `retention_secs` and `orphan_grace_secs` are `0`.
        crate::async_jobs::spawn_retention_loop();

        // Retention sweep for recap session facets. Runs once at startup and
        // then once per day. Disabled when `recap.cache_retention_days == 0`.
        crate::recap::spawn_facet_retention_loop();

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
    }

    // Initialize the MCP subsystem. `init_global` is idempotent and the
    // catalog snippet must be visible to every process so all tiers see
    // MCP-namespaced tools. Watchdog (long-running reconnect loop) is
    // Primary-only — Secondary's idle catalog is enough.
    if init_mcp_subsystem() && primary {
        crate::mcp::watchdog::spawn_watchdog_loop();
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

    if primary {
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
        });

        // Replay leftover async tool jobs. Primary-only: a Secondary ACP
        // booting alongside an active desktop would flip the desktop's
        // running tools to Interrupted.
        tokio::spawn(async move {
            crate::async_jobs::replay_pending_jobs();
        });
        crate::local_model_jobs::replay_interrupted_jobs();
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
