use crate::channel;
use crate::cron;
use crate::globals::AppState;
use crate::globals::{
    APP_LOGGER, CACHED_AGENT, CHANNEL_CANCELS, CHANNEL_DB, CHANNEL_REGISTRY, CODEX_TOKEN_CACHE,
    CRON_DB, EVENT_BUS, IDLE_EXTRACT_HANDLES, KNOWLEDGE_DB, LOG_DB, MEMORY_BACKEND, PROJECT_DB,
    REASONING_EFFORT, SESSION_DB, SUBAGENT_CANCELS, TERMINAL_MANAGER,
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

/// ha-base 配置钩子的一次性注册闸。**不能复用 `INIT_DONE`**——那个 OnceLock 直到
/// `init_runtime` 末尾才置位，函数开头的早退检查并非互斥，并发调用会双双穿过。
static REGISTER_BASE_HOOKS: std::sync::Once = std::sync::Once::new();

/// 特征 crate 启动任务的执行档位。两个消费点都在 `start_background_tasks`
/// 内（即只有 desktop-GUI / server 形态会跑；acp / mcp / eval 注册了也不
/// 消费），差别只在 Primary 门：
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupStage {
    /// 每个跑 `start_background_tasks` 的进程都执行（原 weather 刷新调用位，
    /// desktop 等更细的门由任务自己判）。
    EveryProcess,
    /// 仅 Primary 进程执行（原 `spawn_auto_update_loop` 调用位）。
    PrimaryOnly,
}

/// 特征 crate 装配期登记的启动任务队列。`None` = 已被消费——与工具注册表
/// 同款「开闸判定与队列消费同一把锁」语义（见
/// `tools/registry.rs::PENDING_EXTERNAL` 的 TOCTOU 说明）。
type StartupQueue = std::sync::Mutex<Option<Vec<fn()>>>;
static PENDING_STARTUP_EVERY: StartupQueue = std::sync::Mutex::new(Some(Vec::new()));
static PENDING_STARTUP_PRIMARY: StartupQueue = std::sync::Mutex::new(Some(Vec::new()));
static PENDING_INIT_TASKS: StartupQueue = std::sync::Mutex::new(Some(Vec::new()));

/// 特征 crate 的 **init 期**装配任务：在 `init_runtime` 主体内、子系统装配
/// 段消费（cached_config / 各 DB 全局已就绪，tokio runtime **不保证**存在，
/// 任务内禁 spawn——需要后台循环用 [`register_startup_task`]）。与 startup
/// 两档的差别：**所有 role**（含 acp / mcp / eval）都执行——原 ACP manager
/// 创建等「无条件装配」类代码的原时序点。同款同锁消费语义。
pub fn register_init_task(task: fn()) -> Result<(), crate::AlreadyRegistered> {
    let mut pending = PENDING_INIT_TASKS.lock().unwrap_or_else(|p| p.into_inner());
    match pending.as_mut() {
        Some(queue) => {
            queue.push(task);
            Ok(())
        }
        None => Err(crate::AlreadyRegistered(
            "init tasks (already consumed — register before init_runtime)",
        )),
    }
}

fn run_registered_init_tasks() {
    let tasks = PENDING_INIT_TASKS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .unwrap_or_default();
    for task in tasks {
        task();
    }
}

/// 特征 crate（如 ha-updater / ha-weather）在装配期登记启动任务（后台循环
/// 等）。消费点**不在 `init_runtime` 内**，而在 `start_background_tasks` 的
/// 对应档位调用位（tokio runtime 内、核心子系统已初始化），时序与各特征
/// 迁出前的原调用点逐位一致。消费后再注册返回 `Err`：fail-loud，静默丢弃
/// ＝该特征的后台行为在运行期直接消失。
pub fn register_startup_task(
    stage: StartupStage,
    task: fn(),
) -> Result<(), crate::AlreadyRegistered> {
    let queue = match stage {
        StartupStage::EveryProcess => &PENDING_STARTUP_EVERY,
        StartupStage::PrimaryOnly => &PENDING_STARTUP_PRIMARY,
    };
    let mut pending = queue.lock().unwrap_or_else(|p| p.into_inner());
    match pending.as_mut() {
        Some(queue) => {
            queue.push(task);
            Ok(())
        }
        None => Err(crate::AlreadyRegistered(
            "startup tasks (already consumed — register before start_background_tasks)",
        )),
    }
}

/// 消费并执行指定档位全部登记的启动任务（按注册序）。take() 置 `None`：
/// 从这一刻起该档位注册通道关闭。
fn run_registered_startup_tasks(stage: StartupStage) {
    let queue = match stage {
        StartupStage::EveryProcess => &PENDING_STARTUP_EVERY,
        StartupStage::PrimaryOnly => &PENDING_STARTUP_PRIMARY,
    };
    let tasks = queue
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .unwrap_or_default();
    for task in tasks {
        task();
    }
}

/// config 写路径副作用的生产装配（backup 快照 / 写原因作用域 / 保存后
/// 广播 / ConfigChange hook）。`init_runtime` 与需要真实快照行为的
/// persistence 测试共用这一份，防止两处漂移。
pub(crate) fn default_config_side_effects() -> crate::config::ConfigSideEffects {
    crate::config::ConfigSideEffects {
        post_save: |config, change_category, change_source| {
            // `allowRemoteWrites` is a live capability: publishing a
            // disabled value must immediately revoke remote-created
            // shells, not merely reject the next HTTP request.
            if let Some(manager) = crate::globals::get_terminal_manager() {
                manager.set_remote_access_allowed(config.filesystem.allow_remote_writes);
            }
            if let Some(bus) = crate::globals::get_event_bus() {
                let mut payload = serde_json::json!({ "category": change_category });
                if let Some(source) = change_source {
                    payload["source"] = serde_json::json!(source);
                }
                bus.emit("config:changed", payload);
            }
        },
        post_reload: |config| {
            if let Some(manager) = crate::globals::get_terminal_manager() {
                manager.set_remote_access_allowed(config.filesystem.allow_remote_writes);
            }
            if let Some(bus) = crate::globals::get_event_bus() {
                bus.emit(
                    "config:changed",
                    serde_json::json!({ "category": "app", "source": "reload" }),
                );
            }
        },
        config_changed: |category, source| crate::hooks::fire_config_change(category, source),
    }
}

// 运行模式与版本号原语已下沉 ha-base::runtime_role（模式判定是全仓通用
// 查询，不该背 →app_init 边）；原地再导出保持 `app_init::is_desktop()` 等
// 既有路径。角色仍由本模块的 `init_runtime` 写入（装配职责不动）。
pub use ha_base::runtime_role::{app_version, is_acp, is_desktop, runtime_role, set_app_version};

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
    // role wins. Subsequent sets are dropped (first-write-wins).
    ha_base::runtime_role::set_runtime_role(role);

    if INIT_DONE.get().is_some() {
        return;
    }

    // 装配钩子必须在**任何**业务代码跑之前注册，共六组，两类去向：
    //
    //   · ha-base 反向依赖钩子（base 不能 `use AppConfig`）：plans 目录来源、
    //     Dangerous Mode 配置源、process_registry 通知回调
    //   · ha-core 内部切边钩子（防止 config / filesystem 焊进大环）：config
    //     写路径副作用（保存后广播 / ConfigChange hook）、WorkspaceScope 三个
    //     上下文根解析器、slash 命令分发三槽（channel 不得反向 use 装配层）。
    //     注意 autosave 快照**不走钩子**——它必须无条件执行
    //     （server setup 等入口在 init_runtime 之前/之外写 config），persistence
    //     直调 `config::autosave`
    //
    // 冲突处置分两级，刻意不对称：
    //   · 功能级（plans 目录 / 进程通知 / 根解析器 / slash 三槽）——被顶替
    //     只是对应功能退化（根解析器 fail-closed；slash 未装配只是命令不可
    //     用、无权限语义），记 error 继续
    //   · 安全级（Dangerous Mode 配置源、config 副作用——post_save 携带
    //     `allowRemoteWrites` 的远程 shell 即时撤销）——来源被顶替不可接受，
    //     宁可起不来（panic 在各 register 内/此处触发）
    //
    // 放在 `INIT_DONE` 早退之后、其余初始化之前。必须走 `Once` 而不是靠
    // `INIT_DONE` 早退：`INIT_DONE` 直到本函数**末尾**才置位，两个并发调用者
    // 会双双穿过那道守卫（测试 harness 多线程并行跑 `init_runtime("test")`），
    // 没有 `Once` 的话安全级的 panic 会被这种良性竞态误触发。
    REGISTER_BASE_HOOKS.call_once(|| {
        // 此处 APP_LOGGER 尚未初始化（logger 在 init_runtime 后段才 set），
        // app_error! 会静默 no-op——stderr 兜底保住冲突分支的可观测性。
        fn warn_dup(source: &str, e: impl std::fmt::Display) {
            eprintln!("[app_init] {source}: {e}; keeping first registration");
            app_error!(
                "app_init",
                "register_hooks",
                "{source}: {e}; keeping first registration"
            );
        }

        if let Err(e) = ha_base::paths::register_plans_dir_source(|| {
            crate::config::cached_config().plans_directory.clone()
        }) {
            warn_dup(
                "plans_dir_source (custom plansDirectory will be ignored)",
                e,
            );
        }
        crate::config::register_side_effects(default_config_side_effects());
        if let Err(e) = ha_base::process_registry::register_notifiers(
            crate::process_notification::on_process_exited,
            crate::process_notification::emit_output,
        ) {
            warn_dup("process_notifiers", e);
        }
        if let Err(e) = crate::filesystem::register_root_resolvers(
            crate::knowledge::workspace_root,
            crate::session::workspace_root,
            crate::project::workspace_root,
        ) {
            warn_dup("workspace_root_resolvers", e);
        }
        if let Err(e) = ha_base::security::dangerous::register_config_flag_source(|| {
            crate::config::cached_config().permission.global_yolo
        }) {
            eprintln!("[FATAL] dangerous-mode config source already registered: {e}");
            panic!("dangerous-mode config source already registered: {e}");
        }
        // slash 分发三槽（装配层 → kernel / IM 渠道的唯一回调面）。功能级：
        // 被顶替只会让 slash 命令走到另一份装配，记 error 继续。
        if let Err(e) = crate::slash_hooks::register_slash_hooks(crate::slash_hooks::SlashHooks {
            dispatch: |session_id, agent_id, command, args| {
                Box::pin(crate::slash_commands::handlers::dispatch(
                    session_id, agent_id, command, args,
                ))
            },
            menu_entries: || Box::pin(crate::slash_commands::im_menu_entries()),
            skill_command_help: |name| {
                let store = crate::config::cached_config();
                let skill = crate::skills_hooks::invocable_skills(
                    &store.extra_skills_dirs,
                    &store.disabled_skills,
                )
                .into_iter()
                .find(|s| crate::skills::normalize_skill_command_name(&s.name) == name)?;
                Some(crate::slash_hooks::SkillCommandHelp {
                    arg_placeholder: skill.command_arg_placeholder,
                    arg_options: skill.command_arg_options,
                })
            },
        }) {
            warn_dup(
                "slash_hooks (slash commands may dispatch to a stale assembly)",
                e,
            );
        }
    });

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
    crate::channel_hooks::configure_im_delivery_ownership(
        crate::runtime_role(),
        tier == crate::runtime_lock::Tier::Primary,
    );

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

    if TERMINAL_MANAGER.get().is_none() {
        let event_bus = EVENT_BUS.get().expect("event bus initialized").clone();
        let _ = TERMINAL_MANAGER.set(Arc::new(crate::terminal::TerminalManager::new(event_bus)));
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
    crate::knowledge_hooks::init_index_db();

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
        crate::knowledge_hooks::ensure_default_knowledge_base();
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
    // (e.g. acp launching while desktop is running) doesn't mark-interrupted the
    // desktop's live subagent runs / team members or, worst case, hard-
    // delete its incognito sessions. Defense in depth: incognito purge
    // also has a per-row updated_at < now-60s SQL guard added in this
    // commit (see purge_orphan_incognito_sessions).
    if crate::runtime_lock::is_primary() {
        // Clean up orphan sub-agent runs and converge their durable delivery
        // states. Ordinary pending ParentInjection dispatch is deliberately
        // deferred to background ownership/readiness; armed
        // `injecting_no_replay` rows remain fail-closed because process
        // liveness cannot be inferred without a durable owner token.
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

        // 12 个内置插件的实现随 ha-channel 上浮，由它在 `wire()` 注册的
        // 装配槽装入——registry 本体与 `ChannelId` 键仍是 kernel 契约，
        // 故 `CHANNEL_REGISTRY` 全局与 `AppState` 字段一处未改。
        crate::channel_hooks::install_plugins(&mut registry);

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
        crate::channel_hooks::spawn_dispatcher(registry.clone(), channel_db.clone(), inbound_rx);

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
        if let Some(manager) = TERMINAL_MANAGER.get() {
            manager.set_remote_access_allowed(store.filesystem.allow_remote_writes);
        }
        // 特征 crate 的 init 期装配任务（如 ha-acp 的 SessionManager 创建
        // ——原 ACP manager 创建位，所有 role 执行）。
        run_registered_init_tasks();
    }

    // Install a panic hook that flushes any in-flight stream persisters
    // before the original hook (Tauri's logger / test harness / etc.)
    // takes over. Idempotent — multiple `init_runtime` calls are no-op.
    // Signal handlers (SIGINT/SIGTERM) need a tokio runtime, so each
    // mode entrypoint installs them separately after their runtime is up.
    crate::crash_flush::install_panic_hook();

    // Mark init complete only after every fallible step has succeeded. Any
    // earlier `fatal()` panic kills the process before this set runs.
    // 工具注册表在装配收尾时主动冻结：外部条目（特征 crate）必须已在上面
    // 注册完毕；重名等装配 bug 现在就 panic，不留到首次工具分发。
    crate::tools::registry::freeze_now();

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
    let terminal_manager = crate::require_terminal_manager()
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
        terminal_manager,
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
    debug_assert!(
        ptr_eq_lock(&TERMINAL_MANAGER, &state.terminal_manager),
        "TERMINAL_MANAGER OnceLock and AppState.terminal_manager must share the same Arc"
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
        // approval / ask_user / eviction 随 ha-channel 上浮。
        crate::channel_hooks::spawn_listeners(channel_db.clone(), registry.clone());
        // 菜单重同步**留 kernel**：它只用 registry 的公开 `sync_commands_for_all()`
        // 与 EventBus，订阅的是 skills / config 事件，不碰任何插件实现。
        spawn_channel_menu_resync_listener(registry.clone());
        // startup notifier 排在菜单重同步**之后**——与迁移前逐位一致，故它
        // 单独占一槽而不并进 `spawn_listeners`。自带 is_primary +
        // startup_notification.enabled 双重自门控，语义不变。
        crate::channel_hooks::spawn_startup_notifier(registry.clone());
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

/// Run the two durable ParentInjection replay sources as an idempotent sweep.
/// Each source resolves its own IM binding/readiness: no attach proceeds
/// GUI-only, a running account mirrors normally, and an enabled-but-unavailable
/// account leaves that source pending. Both functions are synchronous SQLite
/// walkers that only spawn their actual injectors, so keep the walk itself off
/// the async runtime worker.
async fn replay_recovered_parent_injections(session_db: Arc<SessionDB>) {
    crate::blocking::run_blocking(move || {
        subagent::replay_pending_parent_deliveries(&session_db);
        crate::async_jobs::JobManager::replay_pending_injections();
    })
    .await;
}

struct DurableReplaySweepPermit<'a>(&'a AtomicBool);

impl Drop for DurableReplaySweepPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn try_acquire_durable_replay_sweep(running: &AtomicBool) -> Option<DurableReplaySweepPermit<'_>> {
    running
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .ok()
        .map(|_| DurableReplaySweepPermit(running))
}

static DURABLE_PARENT_INJECTION_REPLAY_SWEEP_RUNNING: AtomicBool = AtomicBool::new(false);

/// Start one idempotent durable replay sweep without overlapping an earlier
/// tick. Source-level CAS remains the correctness boundary; this process-local
/// single-flight guard avoids unnecessary SQLite contention and log reordering
/// when a walk takes longer than the five-second handoff interval.
fn spawn_durable_parent_injection_replay_sweep() {
    if crate::channel_hooks::im_delivery_ownership()
        != crate::channel_hooks::ImDeliveryOwnership::LocalOwner
    {
        return;
    }
    let Some(_permit) =
        try_acquire_durable_replay_sweep(&DURABLE_PARENT_INJECTION_REPLAY_SWEEP_RUNNING)
    else {
        return;
    };
    let Some(session_db) = SESSION_DB.get().cloned() else {
        app_error!(
            "channel",
            "handoff",
            "Session DB unavailable; durable ParentInjection handoff sweep deferred"
        );
        return;
    };
    tokio::spawn(async move {
        let _permit = _permit;
        replay_recovered_parent_injections(session_db).await;
    });
}

static DELIVERY_SURFACE_REPLAY_LISTENER_STARTED: AtomicBool = AtomicBool::new(false);

/// Successful starts and persisted disable/removal mutations converge through
/// one delivery-surface event. The one-shot startup DB sweep has already put
/// unavailable sources into the unified per-session FIFO with a Channel gate,
/// so later events only open matching gates. A low-frequency pure durable
/// sweep is also retained as the cross-process Secondary→Primary handoff
/// backstop; it never runs startup recovery/reset logic.
fn spawn_delivery_surface_replay_listener() {
    if crate::channel_hooks::im_delivery_ownership()
        != crate::channel_hooks::ImDeliveryOwnership::LocalOwner
        || DELIVERY_SURFACE_REPLAY_LISTENER_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let Some(bus) = crate::globals::get_event_bus() else {
        DELIVERY_SURFACE_REPLAY_LISTENER_STARTED.store(false, std::sync::atomic::Ordering::SeqCst);
        app_error!(
            "channel",
            "startup",
            "EventBus unavailable; delivery-surface parent injection sweeps disabled"
        );
        return;
    };
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        let mut durable_handoff = tokio::time::interval(std::time::Duration::from_secs(5));
        durable_handoff.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        durable_handoff.tick().await; // consume the immediate first tick
        loop {
            tokio::select! {
                _ = durable_handoff.tick() => {
                    spawn_durable_parent_injection_replay_sweep();
                }
                received = rx.recv() => match received {
                    Ok(event)
                        if event.name == channel::registry::DELIVERY_SURFACE_STATE_CHANGED_EVENT =>
                    {
                        if let Some(account_id) = event
                            .payload
                            .get("accountId")
                            .and_then(serde_json::Value::as_str)
                        {
                            crate::subagent::injection::flush_channel_pending_injections(Some(
                                account_id,
                            ));
                        } else {
                            app_warn!(
                                "channel",
                                "startup",
                                "Ignoring delivery-surface event without accountId"
                            );
                        }
                    }
                    Ok(_) => {}
                    // A missed delivery-surface event is indistinguishable from
                    // other lag here. Open every in-memory Channel gate; each
                    // retry re-resolves the live surface before mutation.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        crate::subagent::injection::flush_channel_pending_injections(None);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });
}

/// Primary/full-runtime startup coordinator. Recover and sweep durable sources
/// before starting accounts: GUI-only sources proceed immediately, while each
/// enabled binding enters the unified FIFO behind its own Channel gate. Account
/// starts then run concurrently and open their exact gates as soon as each one
/// succeeds, so one slow/broken account cannot delay healthy peers.
async fn start_channels_then_replay_parent_injections(
    registry: Arc<channel::ChannelRegistry>,
    session_db: Arc<SessionDB>,
    accounts: Vec<channel::types::ChannelAccountConfig>,
) {
    if crate::channel_hooks::im_delivery_ownership()
        != crate::channel_hooks::ImDeliveryOwnership::LocalOwner
    {
        app_error!(
            "channel",
            "startup",
            "Refusing channel startup/durable replay without LocalOwner capability"
        );
        return;
    }
    // State convergence precedes replay so terminal jobs newly classified as
    // interrupted participate in this process's single durable sweep.
    crate::blocking::run_blocking(crate::async_jobs::JobManager::recover_interrupted).await;
    replay_recovered_parent_injections(session_db).await;

    let account_count = accounts.len();
    let mut starts = tokio::task::JoinSet::new();
    for account in accounts {
        let registry = registry.clone();
        starts.spawn(async move {
            if registry.health(&account.id).await.is_running {
                return;
            }
            if let Err(error) = registry.start_account(&account).await {
                // A concurrent manual start can win between the health probe
                // and start_account's worker check. The winning success already
                // emitted the precise surface event.
                if !registry.health(&account.id).await.is_running {
                    crate::channel_hooks::start_watchdog_register_failure(&account, &error).await;
                }
            }
        });
    }
    while let Some(result) = starts.join_next().await {
        if let Err(error) = result {
            app_error!(
                "channel",
                "startup",
                "Channel account startup task failed: {}",
                error
            );
        }
    }

    app_info!(
        "channel",
        "startup",
        "Completed concurrent initial start attempts for {} enabled channel account(s)",
        account_count
    );
}

pub async fn start_background_tasks() {
    let primary = crate::runtime_lock::is_primary();

    // Tier-agnostic: EventBus subscription is multi-subscriber-safe.
    spawn_channel_listeners();
    spawn_delivery_surface_replay_listener();

    // Tier-agnostic: local Chrome Extension broker（特征钩子——未 wire 不起，
    // 首次未命中 wrapper 内部打一次 warn 便于 headless 部署审计）。
    crate::browser_hooks::spawn_broker();

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

    // 特征 crate 的 EveryProcess 档启动任务（原 weather 后台刷新调用位——
    // 不设 Primary 门，desktop 等更细的门由任务闭包自己判）。
    run_registered_startup_tasks(StartupStage::EveryProcess);

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
        // 调度器机器已随 ha-cron 迁出，但**调用位不动**——经 `cron_hooks` 转发，
        // 迁移前后时序逐位相同。刻意不做成 startup task 也只是为了保持这个
        // 时序，**不再是正确性要求**：这里曾经声称「调用序保护了启动清理与
        // 下面 watcher 之间的竞态」，但 `start_scheduler` 起的是独立 OS 线程、
        // 立即返回，清理与 watcher 实际并发，那条保护从未成立。竞态现已在
        // 数据层消掉：`CronDB::clear_stale_running` / `recover_orphaned_runs`
        // 按 **`running_owner != CronDB::owner_token` 的 owner 界**判断遗留
        // （详见 `cron/db.rs:1499-1502`——刻意否决了 `< opened_at` 时间界，
        // 因为 `Utc::now()` 不受 Rust happens-before 约束，clock rollback 会
        // 让本进程写的 `running_at` 落到 `opened_at` 之前被误清）。owner
        // token 与墙上时钟解耦。
        if let (Some(cron_db), Some(session_db)) = (CRON_DB.get(), SESSION_DB.get()) {
            crate::cron_hooks::start_scheduler(cron_db.clone(), session_db.clone());
        }
        crate::loop_control::spawn_loop_event_trigger_watcher();

        // 特征 crate 的 PrimaryOnly 档启动任务（装配期经 `register_startup_task`
        // 登记，如 ha-updater 的 headless 自动更新循环）。在此消费保持原调用
        // 点的时序与条件（primary-gated、tokio runtime 内）；消费后注册通道
        // 关闭。
        run_registered_startup_tasks(StartupStage::PrimaryOnly);

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

        // Sweep durable ParentInjection sources, then auto-start enabled
        // channel accounts concurrently. Two processes
        // auto-starting the same Telegram bot would fight over its webhook,
        // hence the surrounding Primary gate. Failed handshakes stay in the
        // watchdog; they defer only sources bound to that account, not healthy
        // accounts or GUI-only sessions.
        if crate::channel_hooks::im_delivery_ownership()
            == crate::channel_hooks::ImDeliveryOwnership::LocalOwner
        {
            if let (Some(registry), Some(session_db)) = (CHANNEL_REGISTRY.get(), SESSION_DB.get()) {
                let registry = registry.clone();
                let session_db = session_db.clone();
                crate::channel_hooks::spawn_start_watchdog_loop(registry.clone());
                let store = crate::config::cached_config();
                let accounts = store
                    .channels
                    .enabled_accounts()
                    .into_iter()
                    .cloned()
                    .collect();
                tokio::spawn(start_channels_then_replay_parent_injections(
                    registry, session_db, accounts,
                ));
            } else {
                // init_runtime populates both globals. If that invariant
                // breaks, fail closed rather than settling an IM-bound source
                // through a non-owner process. Unrelated background services
                // must still start.
                app_error!(
                    "channel",
                    "startup",
                    "Channel registry/session DB unavailable; durable parent injection replay deferred"
                );
            }
        }

        // Async-job process-state recovery + initial ParentInjection sweep now
        // live in the Channel startup coordinator above. The coordinator also
        // preserves the existing ordering invariant: synchronous
        // `recover_startup_session_state` completed in `init_runtime` before
        // this function can run, so replay sees coherent `context_json`.
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

        // recap facet 保留期清理已随 ha-dash 迁出：`wire()` 注册为 PrimaryOnly
        // startup task（原位就在本 primary 块内，档位一致；执行点前移到本函数
        // 中段的 run_registered_startup_tasks，该循环本就启动即扫一次再进 24h
        // 周期，前移只是让首次清理更早）。

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
                crate::knowledge_hooks::maintenance_idle_tick();
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
        crate::knowledge_hooks::spawn_maintenance_cron_loop();

        // Optional skill draft consolidation loop. Re-reads the
        // auto-review config after every interval or config change.
        crate::skills_hooks::spawn_auto_curator_loop();

        // STT 流式会话 GC 已随 ha-media 迁出：wire() 注册为 PrimaryOnly
        // startup task（本块原位在 primary 块内，档位语义一致）。

        // Knowledge base index: reconcile every KB against disk (catches edits
        // made while the app was off) and start a live watcher per KB root so
        // external-vault edits stay indexed (D6).
        crate::knowledge_hooks::index_spawn_startup_reconcile();
        crate::knowledge_hooks::watcher_start_all_watchers();

        // One-shot reconciler for orphan project-scoped memory rows. The
        // delete_project cascade touches both `session.db` and `memory.db` and
        // cannot wrap them in a single transaction, so a crash between the two
        // can leave unreachable memory rows behind. Project deletion is
        // low-frequency, so a startup sweep is enough — no periodic timer.
        crate::project::reconcile::spawn_startup_reconciler();

        // ACP backend 自动发现已随 ha-acp 迁出：wire() 注册为 PrimaryOnly
        // startup task（本块原位在 primary 块内，档位语义一致）。
    }

    // Initialize the MCP subsystem. `init_global` is idempotent and the
    // catalog snippet must be visible to every process so all tiers see
    // MCP-namespaced tools. Watchdog (long-running reconnect loop) is
    // Primary-only — Secondary's idle catalog is enough.
    if init_mcp_subsystem() && primary {
        crate::mcp::spawn_watchdog();
    }

    // 默认模型自维护 watchdog（自愈冷启的 Ollama 模型 + `local_model:missing_alert`）
    // 已随 ha-local-llm 迁出：`wire()` 注册为 PrimaryOnly startup task。
    //
    // **与 ha-media / ha-acp 那两处不同**：它们原本就在上面那个 primary 块里，
    // 位置没动；本块原是函数末尾**独立的一个** `if primary { … }`，改注册后
    // 执行点前移到上面的 `run_registered_startup_tasks(PrimaryOnly)`。等价性
    // 不靠「原位」，靠两条事实：① primary 门相同（消费点同在 primary 块内，
    // 且 PrimaryOnly 档只此一处消费，ACP 路径不消费——与原先 ACP 不调
    // `spawn_loop` 一致）；② `spawn_loop` 先 sleep 一个 `SWEEP_INTERVAL`（60s）
    // 才跑第一轮，而这中间的二百多行在本函数自身层面无 `.await`（全是
    // spawn），微秒级即走完。primary-only 的理由不变：两个进程同时预载同一
    // 模型只会白白互抢，secondary 透过共享 Ollama 守护进程看到同一份 running。
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

    // EventBus listeners — multi-subscriber-safe, tier-agnostic. ACP and other
    // minimal roles never own local IM delivery, so they intentionally do not
    // install the delivery-surface queue listener.
    spawn_channel_listeners();

    // Local Chrome Extension broker（特征钩子；minimal/ACP 也起，保持
    // owner-plane 诊断一致——故不走 startup 档位而走本钩子）。首次未命中
    // wrapper 内部会打一次 warn。
    crate::browser_hooks::spawn_broker();

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

        // Minimal/ACP roles have `ImDeliveryOwnership::Disabled`: they may
        // converge interrupted job state, but must not claim/consume durable
        // ParentInjection rows because an IM-bound source would look Absent and
        // be incorrectly settled GUI-only. A future LocalOwner performs replay.
        tokio::spawn(async move {
            crate::blocking::run_blocking(crate::async_jobs::JobManager::recover_interrupted).await;
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
    // 实现在 ha-mcp（读 cached_config + 幂等 init_global + 日志），经
    // mcp_hooks 回调；未接线返 false 并留审计（观感等价于未启用）。
    crate::mcp::init_subsystem()
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

    // git 操作对账在 ha-vcs（handoff 回滚需要 git 机器），经 vcs_hooks 同步
    // 内联回调——本函数的 ordering invariant（先于 replay_pending_jobs）对
    // 它同样成立，不得改为后台任务。未接线：簿记行保持 running 原状（接线
    // 进程接手时补账），但 handoff 中断的源 checkout 不会被恢复，故 warn。
    match crate::vcs_hooks::vcs_hooks() {
        Some(hooks) => match (hooks.git_ops_reconciler)(session_db) {
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
        },
        None => app_warn!(
            "git_control",
            "startup_recovery",
            "ha-vcs not wired; skipping interrupted Git operation reconciliation"
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
    fn durable_replay_sweep_is_process_local_single_flight() {
        let running = AtomicBool::new(false);
        let first = try_acquire_durable_replay_sweep(&running).expect("first sweep owns permit");
        assert!(
            try_acquire_durable_replay_sweep(&running).is_none(),
            "a slow sweep must suppress overlapping ticker work"
        );
        drop(first);
        assert!(
            try_acquire_durable_replay_sweep(&running).is_some(),
            "normal completion/drop must re-arm the next ticker"
        );
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
