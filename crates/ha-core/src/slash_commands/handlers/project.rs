//! `/project [name]` — switch to or pick a project.
//!
//! - No args → returns a `ShowProjectPicker` action so the front-end can
//!   render an interactive picker (uses `ProjectPickerItem` rows).
//! - With args → fuzzy-match the project name and emit `EnterProject`
//!   (desktop / HTTP) or `AssignProject` (IM-channel sessions).
//!
//! Phase A1 removed the project↔channel reverse-claim, so IM chats now
//! honour `/project <id>`: the action re-points the chat's current
//! session to that project (no new session is spawned) and the channel
//! slash dispatcher applies it. Desktop / HTTP sessions still get
//! `EnterProject` because their UX expects a fresh session inside the
//! project container.

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
    // Detect IM-channel context — affects which CommandAction we emit.
    let is_im_session = session_id
        .and_then(|sid| session_db.get_session(sid).ok().flatten())
        .map(|m| m.channel_info.is_some())
        .unwrap_or(false);

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

    if is_im_session {
        Ok(CommandResult {
            content: format!(
                "Linking this session to project **{}**…",
                matched.project.name
            ),
            action: Some(CommandAction::AssignProject {
                project_id: matched.project.id.clone(),
            }),
        })
    } else {
        Ok(CommandResult {
            content: format!("Entering project **{}**…", matched.project.name),
            action: Some(CommandAction::EnterProject {
                project_id: matched.project.id.clone(),
            }),
        })
    }
}

/// /projects — list projects as a picker (no fuzzy-match input).
pub fn handle_projects() -> Result<CommandResult, String> {
    let project_db = crate::require_project_db().map_err(|e| e.to_string())?;
    let projects: Vec<ProjectMeta> = project_db.list(false).map_err(|e| e.to_string())?;

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

    let summary_lines: Vec<String> = items
        .iter()
        .take(10)
        .map(|p| {
            format!(
                "- **{}**{} — {} session(s)",
                p.name,
                p.description
                    .as_deref()
                    .map(|d| format!(" — {}", d))
                    .unwrap_or_default(),
                p.session_count
            )
        })
        .collect();
    let content = if summary_lines.is_empty() {
        String::new()
    } else {
        format!(
            "**Projects** ({})\n{}",
            items.len(),
            summary_lines.join("\n")
        )
    };

    Ok(CommandResult {
        content,
        action: Some(CommandAction::ShowProjectPicker { projects: items }),
    })
}
