//! `/project [name]` — switch to or pick a project.
//!
//! - No args → returns a `ShowProjectPicker` action so the front-end can
//!   render an interactive picker (uses `ProjectPickerItem` rows).
//! - With args → fuzzy-match the project name and emit `EnterProject`.
//!
//! IM channels are forbidden from invoking `/project` because IM sessions
//! are tied to a channel-account and cannot also be a "project session" of
//! the desktop variety. The handler self-checks `session.channel_info` to
//! enforce this without changing the dispatcher signature.
//!
//! See `docs/architecture/api-reference.md` for the full slash-command
//! contract.

use crate::project::ProjectMeta;
use crate::session::SessionDB;
use crate::slash_commands::fuzzy;
use crate::slash_commands::types::{CommandAction, CommandResult, ProjectPickerItem};

/// /project [name] — pick or enter a project.
pub fn handle_project(
    session_db: &SessionDB,
    session_id: Option<&str>,
    args: &str,
) -> Result<CommandResult, String> {
    // IM-channel guard — the IM_DISABLED_COMMANDS list in `registry.rs` keeps
    // the menu in sync with this runtime check.
    if let Some(sid) = session_id {
        if let Ok(Some(meta)) = session_db.get_session(sid) {
            if meta.channel_info.is_some() {
                return Ok(CommandResult {
                    content: "`/project` is not available in IM channels. Use the desktop or Web app to manage projects.".into(),
                    action: Some(CommandAction::DisplayOnly),
                });
            }
        }
    }

    let project_db = crate::require_project_db().map_err(|e| e.to_string())?;
    let projects: Vec<ProjectMeta> = project_db.list(false).map_err(|e| e.to_string())?;

    if args.trim().is_empty() {
        if projects.is_empty() {
            return Ok(CommandResult {
                content: "No projects yet. Create one from the sidebar first.".into(),
                action: Some(CommandAction::DisplayOnly),
            });
        }
        let items: Vec<ProjectPickerItem> = projects
            .iter()
            .map(|p| ProjectPickerItem {
                id: p.project.id.clone(),
                name: p.project.name.clone(),
                emoji: p.project.emoji.clone(),
                logo: p.project.logo.clone(),
                color: p.project.color.clone(),
                description: p.project.description.clone(),
                session_count: p.session_count,
            })
            .collect();
        return Ok(CommandResult {
            content: String::new(),
            action: Some(CommandAction::ShowProjectPicker { projects: items }),
        });
    }

    let matched = fuzzy::fuzzy_match_one(
        &projects,
        args,
        |p: &ProjectMeta| vec![p.project.name.clone(), p.project.id.clone()],
        |p: &ProjectMeta| p.project.name.clone(),
        "project",
    )?;
    Ok(CommandResult {
        content: format!("Entering project **{}**…", matched.project.name),
        action: Some(CommandAction::EnterProject {
            project_id: matched.project.id.clone(),
        }),
    })
}
