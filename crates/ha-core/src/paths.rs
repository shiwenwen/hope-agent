use anyhow::Result;
use std::path::PathBuf;

// ── Root Directory ───────────────────────────────────────────────

/// Returns the root directory for all Hope Agent data.
///
/// Resolution order:
/// 1. `HA_DATA_DIR` env var, used as-is (no `.hope-agent` suffix).
///    Lets users run in portable mode and lets cross-platform integration
///    tests redirect into a tempdir — `dirs::home_dir()` on Windows reads
///    `SHGetKnownFolderPath`, not `%USERPROFILE%`, so HOME-style overrides
///    don't work there.
/// 2. `dirs::home_dir().join(".hope-agent")` for the normal install path.
pub fn root_dir() -> Result<PathBuf> {
    if let Some(override_dir) = std::env::var_os("HA_DATA_DIR") {
        let p = PathBuf::from(override_dir);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".hope-agent"))
}

// ── Config ───────────────────────────────────────────────────────

/// Global config file path: ~/.hope-agent/config.json
pub fn config_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("config.json"))
}

// ── Agents ───────────────────────────────────────────────────────

/// Agents root directory: ~/.hope-agent/agents/
pub fn agents_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("agents"))
}

/// Specific agent directory: ~/.hope-agent/agents/{id}/
pub fn agent_dir(id: &str) -> Result<PathBuf> {
    Ok(agents_dir()?.join(id))
}

// ── User Config ─────────────────────────────────────────────────

/// User config file path: ~/.hope-agent/user.json
pub fn user_config_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("user.json"))
}

// ── Credentials ──────────────────────────────────────────────────

/// Credentials directory: ~/.hope-agent/credentials/
pub fn credentials_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("credentials"))
}

/// OAuth auth token path: ~/.hope-agent/credentials/auth.json
pub fn auth_path() -> Result<PathBuf> {
    Ok(credentials_dir()?.join("auth.json"))
}

/// MCP credentials directory: ~/.hope-agent/credentials/mcp/
pub fn mcp_credentials_dir() -> Result<PathBuf> {
    Ok(credentials_dir()?.join("mcp"))
}

/// Per-server MCP credentials file: ~/.hope-agent/credentials/mcp/{server_id}.json
pub fn mcp_credential_path(server_id: &str) -> Result<PathBuf> {
    Ok(mcp_credentials_dir()?.join(format!("{server_id}.json")))
}

/// GitHub token used only by the Issue Reporting tool.
pub fn github_issue_credential_path() -> Result<PathBuf> {
    Ok(credentials_dir()?.join("github-issue.json"))
}

// ── Channels ─────────────────────────────────────────────────────

/// Channels runtime state directory: ~/.hope-agent/channels/
pub fn channels_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("channels"))
}

/// Specific channel runtime state directory: ~/.hope-agent/channels/{channel_id}/
pub fn channel_dir(channel_id: &str) -> Result<PathBuf> {
    Ok(channels_dir()?.join(channel_id))
}

// ── Skills ───────────────────────────────────────────────────────

/// Skills directory: ~/.hope-agent/skills/
pub fn skills_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("skills"))
}

// ── Permission ───────────────────────────────────────────────────

/// Permission system directory: ~/.hope-agent/permission/
/// Holds `protected-paths.json`, `dangerous-commands.json`,
/// `edit-commands.json`, `global-allowlist.json`.
pub fn permission_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("permission"))
}

// ── Agent Home ───────────────────────────────────────────────────

/// Main agent home directory: ~/.hope-agent/home/
pub fn home_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("home"))
}

/// Named agent home directory: ~/.hope-agent/{name}-home/
pub fn agent_home_dir(name: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join(format!("{}-home", name)))
}

// ── Attachments ──────────────────────────────────────────────────

/// Attachments directory for a session: ~/.hope-agent/attachments/{session_id}/
pub fn attachments_dir(session_id: &str) -> Result<PathBuf> {
    Ok(root_dir()?.join("attachments").join(session_id))
}

// ── Sessions (per-session artifacts: hook transcript mirror, …) ─────

/// Root for per-session artifact directories: ~/.hope-agent/sessions/
pub fn sessions_root() -> Result<PathBuf> {
    Ok(root_dir()?.join("sessions"))
}

/// Per-session artifact directory: ~/.hope-agent/sessions/{session_id}/
///
/// Like [`attachments_dir`], this only computes the path — callers create it
/// lazily (e.g. the hooks transcript mirror on first write).
pub fn session_dir(session_id: &str) -> Result<PathBuf> {
    Ok(sessions_root()?.join(session_id))
}

// ── Hooks ───────────────────────────────────────────────────────────

/// Hooks working directory: ~/.hope-agent/hooks/ (overflow files, env files).
pub fn hooks_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("hooks"))
}

// ── macOS Control ─────────────────────────────────────────────────

/// macOS control snapshot image directory:
/// ~/.hope-agent/mac-control/snapshots/
pub fn mac_control_snapshots_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("mac-control").join("snapshots"))
}

/// macOS control diagnostics bundle directory:
/// ~/.hope-agent/mac-control/diagnostics/
pub fn mac_control_diagnostics_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("mac-control").join("diagnostics"))
}

// ── Avatars ──────────────────────────────────────────────────────

/// Avatars directory: ~/.hope-agent/avatars/
pub fn avatars_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("avatars"))
}

// ── Logs ──────────────────────────────────────────────────────────

/// Logs database path: ~/.hope-agent/logs.db
pub fn logs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("logs.db"))
}

/// Logs directory for plain text log files: ~/.hope-agent/logs/
pub fn logs_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("logs"))
}

// ── Share ────────────────────────────────────────────────────────

/// Shared directory for inter-agent data: ~/.hope-agent/share/
#[allow(dead_code)]
pub fn share_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("share"))
}

// ── Cron ────────────────────────────────────────────────────────

/// Cron database path: ~/.hope-agent/cron.db
pub fn cron_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("cron.db"))
}

// ── Background Jobs ─────────────────────────────────────────────

/// Background jobs database path: ~/.hope-agent/background_jobs.db
/// (R1: was `async_jobs.db`; pure rebuildable cache, so the rename just points
/// at a fresh file — the legacy file is best-effort removed at startup.)
pub fn background_jobs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("background_jobs.db"))
}

/// Background jobs result spool directory: ~/.hope-agent/background_jobs/
pub fn background_jobs_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("background_jobs"))
}

/// Per-job result file: ~/.hope-agent/background_jobs/{job_id}.txt
pub fn background_job_result_path(job_id: &str) -> Result<PathBuf> {
    Ok(background_jobs_dir()?.join(format!("{}.txt", job_id)))
}

/// Legacy pre-R1 paths (`async_jobs.db` + `async_jobs/`), best-effort removed at
/// startup so the renamed cache doesn't leave orphans on disk. Not a migration —
/// the data is a rebuildable cache and is simply discarded.
pub fn legacy_async_jobs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("async_jobs.db"))
}

pub fn legacy_async_jobs_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("async_jobs"))
}

/// Local model install/pull jobs database path: ~/.hope-agent/local_model_jobs.db
pub fn local_model_jobs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("local_model_jobs.db"))
}

/// Agent self-scheduled wakeups database path: ~/.hope-agent/wakeups.db
///
/// Backs the `schedule_wakeup` tool (R10): one-shot timers that re-enter the
/// originating session after a delay. Rebuildable/transient — incognito
/// wakeups are never written here (close-and-burn), and unfired rows are
/// re-armed on the next Primary startup.
pub fn wakeups_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("wakeups.db"))
}

/// Cached Ollama Library search/tag metadata: ~/.hope-agent/local_llm_library_cache.db
pub fn local_llm_library_cache_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("local_llm_library_cache.db"))
}

// ── Recap ───────────────────────────────────────────────────────

/// Recap directory: ~/.hope-agent/recap/
pub fn recap_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("recap"))
}

/// Recap database path: ~/.hope-agent/recap/recap.db
pub fn recap_db_path() -> Result<PathBuf> {
    Ok(recap_dir()?.join("recap.db"))
}

/// Generated reports output directory: ~/.hope-agent/reports/
pub fn reports_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("reports"))
}

// ── Memory ──────────────────────────────────────────────────────

/// Memory database path: ~/.hope-agent/memory.db
pub fn memory_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("memory.db"))
}

/// Embedding model cache directory: ~/.hope-agent/models/
pub fn models_cache_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("models"))
}

/// Dream Diary directory: ~/.hope-agent/memory/dreams/
/// Holds one markdown file per cycle (by default named with the local date),
/// created by the Dreaming Light pipeline (Phase B3).
pub fn dreams_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("memory").join("dreams"))
}

/// Memory attachments directory: ~/.hope-agent/memory_attachments/
pub fn memory_attachments_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("memory_attachments"))
}

// ── Browser Profiles ────────────────────────────────────────────

/// Browser profiles root directory: ~/.hope-agent/browser-profiles/
pub fn browser_profiles_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("browser-profiles"))
}

/// Specific browser profile directory: ~/.hope-agent/browser-profiles/{profile_name}/
pub fn browser_profile_dir(profile_name: &str) -> Result<PathBuf> {
    Ok(browser_profiles_dir()?.join(profile_name))
}

/// User-attach Chrome profile directory: ~/.hope-agent/browser/user-attach/
///
/// Used by the "Take over user Chrome" path in settings: hope-agent spawns
/// a Chrome instance pointed at this directory so the user's daily browsing
/// (their real `Default` / per-OS profile) is never touched.
pub fn browser_user_attach_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("browser").join("user-attach"))
}

/// Managed-launch Chrome user-data-dir: `~/.hope-agent/browser/managed-runner/`.
///
/// Used by `profile.op=launch target=managed` when no `profile` arg is
/// given. chromiumoxide's default behaviour is to pick a random `/tmp`
/// directory which makes SingletonLock observability impossible — a crashed
/// Chrome leaves a stale lock there and the next launch fails with
/// `File exists (17)`. Pinning a stable path lets
/// [`crate::browser::singleton_lock`] detect and clean stale locks.
pub fn browser_managed_runner_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("browser").join("managed-runner"))
}

/// Root for hope-agent–managed browser runtimes:
/// `~/.hope-agent/browser/runtime/`. Holds the unzipped Chromium snapshot
/// when the system has no Chrome / Edge / Brave / Chromium installed.
///
/// Pinned revisions live in [`crate::browser::runtime`] (per-platform
/// constants — Chromium snapshots build each OS independently, so a
/// single workspace-wide revision isn't representable).
pub fn browser_runtime_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("browser").join("runtime"))
}

/// Per-revision Chromium runtime directory:
/// `~/.hope-agent/browser/runtime/chromium-{revision}/`. Versioned so
/// bumping the per-platform pinned revision doesn't collide with an
/// older cached binary (old dirs can be hand-cleaned).
pub fn chromium_runtime_dir(revision: u32) -> Result<PathBuf> {
    Ok(browser_runtime_dir()?.join(format!("chromium-{revision}")))
}

// ── Generated Images ────────────────────────────────────────────────

/// Generated images directory: ~/.hope-agent/generated-images/
pub fn generated_images_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("generated-images"))
}

// ── Crash Journal ──────────────────────────────────────────────────

/// Crash journal file path: ~/.hope-agent/crash_journal.json
pub fn crash_journal_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("crash_journal.json"))
}

// ── Desktop Window State ───────────────────────────────────────────

/// Desktop window state file path: ~/.hope-agent/window-state.json
pub fn window_state_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("window-state.json"))
}

// ── Self-Update ─────────────────────────────────────────────────────

/// Self-update working directory: ~/.hope-agent/updater/
pub fn updater_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("updater"))
}

/// Per-version download staging directory: ~/.hope-agent/updater/staging/{version}/
pub fn updater_staging_dir(version: &str) -> Result<PathBuf> {
    Ok(updater_dir()?
        .join("staging")
        .join(sanitize_path_segment(version)))
}

/// Per-version backup directory: ~/.hope-agent/updater/backup/{version}/
/// Holds the prior binary so `app_update rollback` can restore it.
pub fn updater_backup_dir(version: &str) -> Result<PathBuf> {
    Ok(updater_dir()?
        .join("backup")
        .join(sanitize_path_segment(version)))
}

// ── Backups ────────────────────────────────────────────────────────

/// Backups directory: ~/.hope-agent/backups/
pub fn backups_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("backups"))
}

/// Automatic-snapshot directory for config / user_config changes:
/// ~/.hope-agent/backups/autosave/
pub fn autosave_dir() -> Result<PathBuf> {
    Ok(backups_dir()?.join("autosave"))
}

// ── Canvas ──────────────────────────────────────────────────────

/// Canvas root directory: ~/.hope-agent/canvas/
pub fn canvas_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("canvas"))
}

/// Canvas projects directory: ~/.hope-agent/canvas/projects/
pub fn canvas_projects_dir() -> Result<PathBuf> {
    Ok(canvas_dir()?.join("projects"))
}

/// Specific canvas project directory: ~/.hope-agent/canvas/projects/{id}/
pub fn canvas_project_dir(project_id: &str) -> Result<PathBuf> {
    Ok(canvas_projects_dir()?.join(project_id))
}

/// Canvas database path: ~/.hope-agent/canvas/canvas.db
pub fn canvas_db_path() -> Result<PathBuf> {
    Ok(canvas_dir()?.join("canvas.db"))
}

// ── Projects ────────────────────────────────────────────────────

/// Projects root directory: ~/.hope-agent/projects/
pub fn projects_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("projects"))
}

/// Specific project directory: ~/.hope-agent/projects/{id}/
pub fn project_dir(project_id: &str) -> Result<PathBuf> {
    Ok(projects_dir()?.join(project_id))
}

/// Default project workspace directory: ~/.hope-agent/projects/{id}/workspace/
///
/// Used as the per-project working directory when the user has not selected an
/// explicit one. Created lazily on first resolution (see
/// `session::helpers::effective_session_working_dir`); never written into the
/// DB so the `~/.hope-agent` tree stays relocatable via `HA_DATA_DIR`.
pub fn project_workspace_dir(project_id: &str) -> Result<PathBuf> {
    Ok(project_dir(project_id)?.join("workspace"))
}

// ── Knowledge Base ──────────────────────────────────────────────

/// Knowledge base root directory: ~/.hope-agent/knowledge/
pub fn knowledge_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("knowledge"))
}

/// Global knowledge index database: ~/.hope-agent/knowledge/index.db
///
/// Pure rebuildable cache (note / note_chunk / note_link / note_tag + FTS5 +
/// vec). Deleting it loses nothing — the `.md` files + the `knowledge_bases`
/// registry in `sessions.db` are the single source of truth.
pub fn knowledge_index_db_path() -> Result<PathBuf> {
    Ok(knowledge_dir()?.join("index.db"))
}

/// Per-KB default notes directory: ~/.hope-agent/knowledge/{kb_id}/notes/
///
/// Used when a knowledge base's `root_dir` is NULL (internal, app-managed).
/// Created lazily on first resolution (see `knowledge::resolve_kb_dir`); never
/// written into the DB so the `~/.hope-agent` tree stays relocatable via
/// `HA_DATA_DIR`. A non-NULL `root_dir` points at an external vault instead.
pub fn knowledge_kb_notes_dir(kb_id: &str) -> Result<PathBuf> {
    Ok(knowledge_dir()?
        .join(sanitize_path_segment(kb_id))
        .join("notes"))
}

// ── Plans ───────────────────────────────────────────────────────

/// Plans directory: uses custom `plansDirectory` config if set,
/// otherwise `~/.hope-agent/plans/`.
pub fn plans_dir() -> Result<PathBuf> {
    let store = crate::config::cached_config();
    if let Some(ref custom_dir) = store.plans_directory {
        if !custom_dir.is_empty() {
            let expanded = if custom_dir.starts_with('~') {
                if let Some(home) = dirs::home_dir() {
                    let suffix = custom_dir
                        .strip_prefix("~/")
                        .or_else(|| custom_dir.strip_prefix("~"))
                        .unwrap_or(custom_dir);
                    if suffix.is_empty() {
                        home
                    } else {
                        home.join(suffix)
                    }
                } else {
                    PathBuf::from(custom_dir)
                }
            } else {
                PathBuf::from(custom_dir)
            };
            return Ok(expanded);
        }
    }
    Ok(root_dir()?.join("plans"))
}

/// Per-session plan directory: `<plans_dir>/<agent_id>/<session_id>/`.
///
/// Two-level isolation: keeps each session's plan files (current + version
/// backups + result) physically separate so a model `ls`-ing the plans dir
/// can only see its own work. Grouping by agent first means historical
/// plans are also browseable per agent (handy for export / archival).
///
/// Both `agent_id` and `session_id` are sanitized to bare alphanumerics +
/// `-` / `_` to defang any path-traversal attempt from upstream — session
/// ids are UUIDs and agent ids are slug-validated, so this is defense in
/// depth, not the primary boundary.
pub fn session_plans_dir(agent_id: &str, session_id: &str) -> Result<PathBuf> {
    Ok(plans_dir()?
        .join(sanitize_path_segment(agent_id))
        .join(sanitize_path_segment(session_id)))
}

/// Sanitize an untrusted id (agent / session / version / kb) into a bare path
/// segment: ASCII alphanumerics plus `-` / `_`, with everything else (including
/// `.` and `/`) collapsed to `_`, defanging `..` / separator traversal. Shared
/// by `paths.rs`, `tools::execution` (large-result spill + `tool_results` purge)
/// and `tools::image_markers` (materialized vision files) so all three derive
/// the same `tool_results/<segment>/` directory for a given session — otherwise
/// materialization and purge can diverge into different directories.
pub(crate) fn sanitize_path_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ── Directory Initialization ──────────────────────────────────────

/// Ensure all required directories exist.
pub fn ensure_dirs() -> Result<()> {
    let dirs_to_create = [
        root_dir()?,
        credentials_dir()?,
        channels_dir()?,
        skills_dir()?,
        agents_dir()?,
        home_dir()?,
        avatars_dir()?,
        share_dir()?,
        logs_dir()?,
        models_cache_dir()?,
        browser_profiles_dir()?,
        backups_dir()?,
        generated_images_dir()?,
        canvas_dir()?,
        canvas_projects_dir()?,
        projects_dir()?,
        plans_dir()?,
        recap_dir()?,
        reports_dir()?,
        background_jobs_dir()?,
        knowledge_dir()?,
    ];
    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}
