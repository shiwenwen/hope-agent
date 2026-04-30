//! Shared file IO + caching for the three user-editable pattern lists
//! (protected paths / dangerous commands / edit commands).
//!
//! Each list lives at `~/.hope-agent/permission/<name>.json` as a JSON
//! string array. Missing file → defaults are returned. The "Restore defaults"
//! UI button calls [`reset_to_defaults`] which writes the const defaults
//! back to disk.
//!
//! Cached in a per-list `RwLock<Option<Vec<String>>>` to avoid hitting disk
//! on every `engine::resolve` call. Mutators invalidate the cache.

use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result};

/// Lock-protected cache slot. `None` = not loaded yet (lazy init on first read).
pub type Cache = RwLock<Option<Vec<String>>>;

/// Load the list from disk (or defaults), caching the result. Subsequent
/// calls return a cloned `Vec<String>` from the cache.
pub fn load_or_defaults(cache: &Cache, file: &str, defaults: &[&'static str]) -> Vec<String> {
    {
        let guard = cache.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cached) = *guard {
            return cached.clone();
        }
    }
    let loaded = read_from_disk(file).unwrap_or_else(|_| {
        defaults.iter().map(|s| s.to_string()).collect()
    });
    {
        let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
        *guard = Some(loaded.clone());
    }
    loaded
}

/// Persist the new list and update the cache. Atomic via tempfile + rename.
pub fn save(cache: &Cache, file: &str, patterns: &[String]) -> Result<()> {
    write_to_disk(file, patterns).with_context(|| format!("failed to save {file}"))?;
    let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(patterns.to_vec());
    Ok(())
}

/// Reset to compile-time defaults, writing them back to disk.
pub fn reset_to_defaults(
    cache: &Cache,
    file: &str,
    defaults: &[&'static str],
) -> Result<Vec<String>> {
    let owned: Vec<String> = defaults.iter().map(|s| s.to_string()).collect();
    save(cache, file, &owned)?;
    Ok(owned)
}

/// Drop the in-memory cache so the next `load_or_defaults` re-reads disk.
/// Used by tests + potential future config-watch reload.
#[allow(dead_code)]
pub fn invalidate(cache: &Cache) {
    let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

fn list_path(file: &str) -> Result<PathBuf> {
    Ok(crate::paths::permission_dir()?.join(file))
}

fn read_from_disk(file: &str) -> Result<Vec<String>> {
    let path = list_path(file)?;
    if !path.exists() {
        anyhow::bail!("file not present: {}", path.display());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let parsed: Vec<String> = serde_json::from_str(&raw)
        .with_context(|| format!("parse {} as JSON string array", path.display()))?;
    Ok(parsed)
}

fn write_to_disk(file: &str, patterns: &[String]) -> Result<()> {
    let dir = crate::paths::permission_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let path = dir.join(file);
    let json = serde_json::to_string_pretty(patterns)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} → {}", tmp.display(), path.display()))?;
    Ok(())
}
