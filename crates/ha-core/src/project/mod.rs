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
    delete_project_cascade, purge_project_dir, resolve_project_dir, MAX_PROJECT_FILE_BYTES,
};
pub use types::{CreateProjectInput, Project, ProjectMeta, UpdateProjectInput};
