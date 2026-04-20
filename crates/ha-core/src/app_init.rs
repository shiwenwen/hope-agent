use crate::acp_control;
use crate::channel;
use crate::cron;
use crate::globals::AppState;
use crate::globals::{
    ACP_MANAGER, APP_LOGGER, CHANNEL_DB, CHANNEL_REGISTRY, CRON_DB, EVENT_BUS,
    IDLE_EXTRACT_HANDLES, MEMORY_BACKEND, PROJECT_DB, SESSION_DB, SUBAGENT_CANCELS,
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

    // Initialize the LogDB and AppLogger
    let log_db_path = fatal(logging::db_path(), "Cannot resolve log database path");
    let log_db = Arc::new(fatal(LogDB::open(&log_db_path), "Cannot open log database"));

    // Load log config and cleanup old logs
    let log_config = logging::load_log_config().unwrap_or_default();
    let _ = log_db.cleanup_old(log_config.max_age_days);
    let _ = logging::cleanup_old_log_files(log_config.max_age_days);
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

    // Initialize channel cancel registry
    let channel_cancels = Arc::new(channel::ChannelCancelRegistry::new());

    // Clean up orphan sub-agent runs from previous app session
    subagent::cleanup_orphan_runs(&session_db);

    // Clean up orphan team members from previous app session
    crate::team::cleanup::cleanup_orphan_teams(&session_db);

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

        // Spawn the inbound message dispatcher
        channel::worker::spawn_dispatcher(registry.clone(), channel_db.clone(), inbound_rx);

        // Spawn the IM channel approval listener (routes tool approval prompts to IM)
        channel::worker::approval::spawn_channel_approval_listener(
            channel_db.clone(),
            registry.clone(),
        );

        // Spawn the IM channel ask_user_question listener
        channel::worker::ask_user::spawn_channel_ask_user_listener(
            channel_db.clone(),
            registry.clone(),
        );

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

    AppState {
        agent: Mutex::new(None),
        auth_result: Arc::new(Mutex::new(None)),
        reasoning_effort: Mutex::new("medium".to_string()),
        codex_token: Mutex::new(None),
        current_agent_id: Mutex::new("default".to_string()),
        session_db,
        project_db,
        chat_cancel: Arc::new(AtomicBool::new(false)),
        log_db,
        logger,
        cron_db,
        subagent_cancels,
        channel_cancels,
    }
}

/// Start background async tasks that require a tokio runtime.
/// Must be called from within a tokio async context (e.g., Tauri's `.setup()` or a server runtime).
pub async fn start_background_tasks() {
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
}
