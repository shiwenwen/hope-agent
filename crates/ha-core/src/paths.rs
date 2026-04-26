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

// ── Async Tool Jobs ─────────────────────────────────────────────

/// Async tool jobs database path: ~/.hope-agent/async_jobs.db
pub fn async_jobs_db_path() -> Result<PathBuf> {
    Ok(root_dir()?.join("async_jobs.db"))
}

/// Async tool jobs result spool directory: ~/.hope-agent/async_jobs/
pub fn async_jobs_dir() -> Result<PathBuf> {
    Ok(root_dir()?.join("async_jobs"))
}

/// Per-job result file: ~/.hope-agent/async_jobs/{job_id}.txt
pub fn async_job_result_path(job_id: &str) -> Result<PathBuf> {
    Ok(async_jobs_dir()?.join(format!("{}.txt", job_id)))
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

/// Project original files directory: ~/.hope-agent/projects/{id}/files/
pub fn project_files_dir(project_id: &str) -> Result<PathBuf> {
    Ok(project_dir(project_id)?.join("files"))
}

/// Project extracted text directory: ~/.hope-agent/projects/{id}/extracted/
pub fn project_extracted_dir(project_id: &str) -> Result<PathBuf> {
    Ok(project_dir(project_id)?.join("extracted"))
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
        async_jobs_dir()?,
    ];
    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}
