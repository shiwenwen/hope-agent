//! Project — optional containers that group sessions with shared memories,
//! custom instructions, and uploaded files.
//!
//! See `AGENTS.md` (architecture section) for the full design.

mod db;
mod files;
pub mod reconcile;
mod types;

pub use db::ProjectDB;
pub use files::{
    delete_project_cascade, delete_project_file, purge_project_files_dir,
    upload_project_file, UploadInput, MAX_PROJECT_FILE_BYTES,
};
pub use types::{
    CreateProjectInput, Project, ProjectFile, ProjectMeta, UpdateProjectInput,
};
