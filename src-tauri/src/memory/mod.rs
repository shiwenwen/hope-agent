pub mod types;
pub mod traits;
pub mod sqlite;
pub mod embedding;
pub mod import;
pub mod helpers;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original memory.rs is re-exported here
// so that `crate::memory::XXX` continues to work.

pub use types::*;
pub use traits::*;
pub use sqlite::SqliteMemoryBackend;
pub use embedding::*;
pub use import::*;
pub use helpers::{load_dedup_config, load_extract_config};
