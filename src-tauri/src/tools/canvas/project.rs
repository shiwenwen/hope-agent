use anyhow::Result;
use chrono::Local;
use uuid::Uuid;

use crate::canvas_db::{CanvasDB, CanvasProject, CanvasVersion};
use crate::paths;

use super::renderer;

/// Create a new canvas project, write files, insert into DB, return project info.
pub fn create_project(
    db: &CanvasDB,
    title: Option<&str>,
    content_type: &str,
    html: Option<&str>,
    css: Option<&str>,
    js: Option<&str>,
    content: Option<&str>,
    language: Option<&str>,
    session_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<CanvasProject> {
    let project_id = Uuid::new_v4().to_string();
    let now = Local::now().to_rfc3339();
    let title = title.unwrap_or("Untitled Canvas");

    // Write files to disk
    let project_dir = paths::canvas_project_dir(&project_id)?;
    renderer::write_project_files(&project_dir, content_type, html, css, js, content, language)?;

    // Insert project record
    let project = CanvasProject {
        id: project_id.clone(),
        title: title.to_string(),
        content_type: content_type.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        agent_id: agent_id.map(|s| s.to_string()),
        created_at: now.clone(),
        updated_at: now.clone(),
        version_count: 1,
        metadata: None,
    };
    db.create_project(&project)?;

    // Create initial version (v1)
    let version = CanvasVersion {
        id: 0, // auto-increment
        project_id: project_id.clone(),
        version_number: 1,
        message: Some("Initial version".to_string()),
        html: html.map(|s| s.to_string()),
        css: css.map(|s| s.to_string()),
        js: js.map(|s| s.to_string()),
        content: content.map(|s| s.to_string()),
        created_at: now,
    };
    db.create_version(&version)?;

    Ok(project)
}

/// Update an existing canvas project: write files, create new version, update DB.
pub fn update_project(
    db: &CanvasDB,
    project_id: &str,
    title: Option<&str>,
    html: Option<&str>,
    css: Option<&str>,
    js: Option<&str>,
    content: Option<&str>,
    language: Option<&str>,
    version_message: Option<&str>,
    max_versions: i64,
) -> Result<CanvasProject> {
    let project = db
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Canvas project '{}' not found", project_id))?;

    let now = Local::now().to_rfc3339();
    let new_version_number = project.version_count + 1;

    // Write updated files
    let project_dir = paths::canvas_project_dir(project_id)?;
    renderer::write_project_files(
        &project_dir,
        &project.content_type,
        html,
        css,
        js,
        content,
        language,
    )?;

    // Create new version snapshot
    let version = CanvasVersion {
        id: 0,
        project_id: project_id.to_string(),
        version_number: new_version_number,
        message: version_message.map(|s| s.to_string()),
        html: html.map(|s| s.to_string()),
        css: css.map(|s| s.to_string()),
        js: js.map(|s| s.to_string()),
        content: content.map(|s| s.to_string()),
        created_at: now.clone(),
    };
    db.create_version(&version)?;

    // Update project metadata
    db.update_project_meta(project_id, title, &now, new_version_number)?;

    // Cleanup old versions if needed
    if max_versions > 0 {
        let _ = db.cleanup_old_versions(project_id, max_versions);
    }

    db.get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Project disappeared after update"))
}

/// Delete a canvas project: remove files and DB records.
pub fn delete_project(db: &CanvasDB, project_id: &str) -> Result<()> {
    // Remove files
    let project_dir = paths::canvas_project_dir(project_id)?;
    if project_dir.exists() {
        std::fs::remove_dir_all(&project_dir)?;
    }

    // Remove DB records (cascade deletes versions)
    db.delete_project(project_id)?;
    Ok(())
}

/// Restore a project to a specific version.
pub fn restore_version(
    db: &CanvasDB,
    project_id: &str,
    version_number: i64,
) -> Result<CanvasProject> {
    let version = db
        .get_version(project_id, version_number)?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Version {} not found for project '{}'",
                version_number,
                project_id
            )
        })?;

    let project = db
        .get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Canvas project '{}' not found", project_id))?;

    // Re-write files from version snapshot
    let project_dir = paths::canvas_project_dir(project_id)?;
    renderer::write_project_files(
        &project_dir,
        &project.content_type,
        version.html.as_deref(),
        version.css.as_deref(),
        version.js.as_deref(),
        version.content.as_deref(),
        None,
    )?;

    // Create a new version for the restore action
    let now = Local::now().to_rfc3339();
    let new_version_number = project.version_count + 1;
    let restore_version = CanvasVersion {
        id: 0,
        project_id: project_id.to_string(),
        version_number: new_version_number,
        message: Some(format!("Restored from version {}", version_number)),
        html: version.html,
        css: version.css,
        js: version.js,
        content: version.content,
        created_at: now.clone(),
    };
    db.create_version(&restore_version)?;
    db.update_project_meta(project_id, None, &now, new_version_number)?;

    db.get_project(project_id)?
        .ok_or_else(|| anyhow::anyhow!("Project disappeared after restore"))
}
