//! Built-in user manual (Help Center) — thin shells over `ha_core::manual`.

use crate::commands::CmdError;

use ha_core::manual::{ManualBundle, ManualSearchHit};

/// Full manual bundle for the Help window. `lang` optional — defaults to the
/// configured UI language. The embed read (disk in debug) plus the
/// opportunistic mirror are blocking IO, so they run on the blocking pool.
#[tauri::command]
pub async fn get_manual_bundle(lang: Option<String>) -> Result<ManualBundle, CmdError> {
    Ok(ha_core::blocking::run_blocking(move || {
        ha_core::manual::bundle_for_command(lang.as_deref())
    })
    .await)
}

/// Full-text search over the manual in the (resolved) UI language.
#[tauri::command]
pub async fn search_manual(
    lang: Option<String>,
    query: String,
) -> Result<Vec<ManualSearchHit>, CmdError> {
    Ok(ha_core::blocking::run_blocking(move || {
        ha_core::manual::search_for_command(lang.as_deref(), &query)
    })
    .await)
}
