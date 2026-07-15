use anyhow::{Context, Result};

use crate::memory::{claims, MemoryScope};
use crate::session::SessionDB;

use super::{
    memory, read_project_instructions, ProjectDB, ProjectInstructionsStats, ProjectOverviewSummary,
};

pub const RECENT_PROJECT_SESSIONS_LIMIT: u32 = 5;

/// Build the project settings overview from the session database, Core Memory
/// repository, structured-claim store and root AGENTS.md. Memory/filesystem
/// sources are deliberately best-effort; the session database and project row
/// remain the only hard requirements for rendering the dashboard.
pub fn build_project_overview(
    project_id: &str,
    project_db: &ProjectDB,
    session_db: &SessionDB,
) -> Result<ProjectOverviewSummary> {
    project_db
        .get(project_id)?
        .with_context(|| format!("project not found: {project_id}"))?;

    let (recent_sessions, session_count) = session_db
        .list_recent_regular_chats_for_project(project_id, RECENT_PROJECT_SESSIONS_LIMIT)?;

    let auto_memory_topic_count = memory::list(project_id)
        .ok()
        .and_then(|entries| u32::try_from(entries.len()).ok());

    let active_claim_count = claims::list_claims_page(claims::ClaimListFilter {
        scope: Some(MemoryScope::Project {
            id: project_id.to_string(),
        }),
        status: Some("active".to_string()),
        limit: Some(1),
        offset: Some(0),
        ..Default::default()
    })
    .ok()
    .and_then(|page| u32::try_from(page.total).ok());

    let instructions = read_project_instructions(project_id, project_db)
        .ok()
        .map(|file| ProjectInstructionsStats {
            path: file.path,
            line_count: markdown_line_count(&file.content),
            size_bytes: file.content.len() as u64,
            empty: file.content.trim().is_empty(),
        });

    Ok(ProjectOverviewSummary {
        session_count,
        recent_sessions,
        auto_memory_topic_count,
        active_claim_count,
        instructions,
    })
}

fn markdown_line_count(content: &str) -> u32 {
    if content.is_empty() {
        0
    } else {
        let trailing_empty_line = usize::from(content.ends_with('\n') || content.ends_with('\r'));
        u32::try_from(content.lines().count().saturating_add(trailing_empty_line))
            .unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::markdown_line_count;

    #[test]
    fn markdown_line_count_handles_empty_and_trailing_newline() {
        assert_eq!(markdown_line_count(""), 0);
        assert_eq!(markdown_line_count("one"), 1);
        assert_eq!(markdown_line_count("one\ntwo\n"), 3);
    }
}
