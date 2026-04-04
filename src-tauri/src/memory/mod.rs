pub mod embedding;
pub mod helpers;
pub mod import;
pub mod mmr;
pub(crate) mod selection;
pub mod sqlite;
pub mod traits;
pub mod types;

// ── Re-exports for backward compatibility ───────────────────────
// Everything that was `pub` in the original memory.rs is re-exported here
// so that `crate::memory::XXX` continues to work.

pub use embedding::*;
pub use helpers::{load_dedup_config, load_extract_config};
pub use import::*;
pub use sqlite::SqliteMemoryBackend;
pub use traits::*;
pub use types::*;
