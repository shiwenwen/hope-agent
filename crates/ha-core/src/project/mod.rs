//! Project — optional containers that group sessions with shared memories and
//! a shared working directory. Project instructions live only in the working
//! directory's `AGENTS.md`.
//!
//! See `AGENTS.md` (architecture section) for the full design.

mod db;
mod files;
pub mod memory;
pub mod reconcile;
mod types;

pub use db::ProjectDB;
pub use files::{
    create_project_with_instructions_file, delete_project_cascade, ensure_project_instructions,
    inspect_default_project_instructions, inspect_project_instructions, purge_project_dir,
    read_project_instructions, resolve_project_dir, save_project_instructions,
    update_project_with_instructions_file, ProjectInstructionsDraft, ProjectInstructionsFile,
    StaleProjectInstructionsError, MAX_PROJECT_FILE_BYTES,
};
pub use types::{CreateProjectInput, Project, ProjectMeta, UpdateProjectInput};
