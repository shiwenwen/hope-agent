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

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

/// Cache holds the currently-active patterns as an `Arc<Vec<String>>` so the
/// hot path (`engine::resolve` → `current_patterns()`) only bumps a refcount
/// rather than cloning ~24-80 strings per tool dispatch.
/// `None` = not loaded yet (lazy on first read).
pub type Cache = RwLock<Option<Arc<Vec<String>>>>;

/// Load the list from disk (or defaults), caching the result. Subsequent
/// calls return an `Arc::clone` of the cached snapshot — refcount bump only.
pub fn load_or_defaults(cache: &Cache, file: &str, defaults: &[&'static str]) -> Arc<Vec<String>> {
    {
        let guard = cache.read().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cached) = *guard {
            return Arc::clone(cached);
        }
    }
    let loaded: Arc<Vec<String>> = Arc::new(
        read_from_disk(file).unwrap_or_else(|_| defaults.iter().map(|s| s.to_string()).collect()),
    );
    let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
    // Another caller may have loaded the cache between our read drop + write
    // acquire; honor their value rather than overwriting with ours.
    if let Some(ref cached) = *guard {
        return Arc::clone(cached);
    }
    *guard = Some(Arc::clone(&loaded));
    loaded
}

/// Persist the new list and update the cache. Atomic via tempfile + rename.
pub fn save(cache: &Cache, file: &str, patterns: &[String]) -> Result<()> {
    write_to_disk(file, patterns).with_context(|| format!("failed to save {file}"))?;
    let mut guard = cache.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Arc::new(patterns.to_vec()));
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

fn list_path(file: &str) -> Result<PathBuf> {
    Ok(crate::paths::permission_dir()?.join(file))
}

fn read_from_disk(file: &str) -> Result<Vec<String>> {
    let path = list_path(file)?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            anyhow::bail!("file not present: {}", path.display());
        }
        Err(e) => return Err(anyhow::Error::from(e).context(format!("read {}", path.display()))),
    };
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
