//! Project — optional containers that group sessions with shared memories,
//! custom instructions, and uploaded files.
//!
//! See `AGENTS.md` (architecture section) for the full design.

mod db;
mod types;

pub use db::ProjectDB;
pub use types::{
    CreateProjectInput, Project, ProjectFile, ProjectMeta, UpdateProjectInput,
};
