//! Built-in skills embedded into the binary.
//!
//! The repo's `skills/` tree is compiled in via `rust-embed` and extracted on
//! demand to `<data_dir>/bundled-skills/<content-hash>/`. This makes bundled
//! skills survive every distribution shape from the same code path — desktop
//! bundles, Docker, and the bare-binary tarball (which ships nothing next to
//! the executable) — and a self-updated binary automatically extracts its own
//! fresh copy because the content hash changes.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rust_embed::RustEmbed;

use crate::paths;

use super::discovery::looks_like_skills_dir;

/// In release builds the files are baked into the binary; in debug builds
/// rust-embed reads the workspace `skills/` directory at call time. The
/// resolver prefers the workspace directory directly in dev, so this module
/// is effectively release-only there.
#[derive(RustEmbed)]
#[folder = "../../skills"]
struct BundledSkillAssets;

/// Age after which a leftover `.tmp-*` extraction directory is considered
/// abandoned (crashed process) rather than a concurrent writer.
const STALE_TMP_AGE: Duration = Duration::from_secs(3600);

/// Extract the embedded skills and return the content-addressed directory.
/// Reuses an existing extraction of the same hash; prunes extractions left
/// behind by older binaries.
pub fn ensure_extracted() -> Result<PathBuf> {
    ensure_extracted_in(&paths::bundled_skills_cache_dir()?)
}

fn ensure_extracted_in(root: &Path) -> Result<PathBuf> {
    let files = collect_embedded_files()?;
    let version = content_hash(&files);
    let target = root.join(&version);

    if target.is_dir() {
        if looks_like_skills_dir(&target) {
            prune_stale(root, &version);
            return Ok(target);
        }
        // The directory-level rename below is atomic, so a hash-named dir is
        // normally complete — this only fires if a user gutted it by hand.
        fs::remove_dir_all(&target).ok();
    }

    fs::create_dir_all(root)
        .with_context(|| format!("failed to create {}", root.display()))?;
    let tmp = root.join(format!(".tmp-{}", std::process::id()));
    fs::remove_dir_all(&tmp).ok();
    for (rel, data) in &files {
        // Embedded paths come from our own repo, but stay defensive.
        if rel.split('/').any(|seg| seg.is_empty() || seg == ".." || seg == ".") {
            continue;
        }
        let dest = rel.split('/').fold(tmp.clone(), |p, seg| p.join(seg));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&dest, data).with_context(|| format!("failed to write {}", dest.display()))?;
        // rust-embed drops file modes; restore +x for shebang scripts so
        // skills that execute helpers directly keep working.
        #[cfg(unix)]
        if data.starts_with(b"#!") {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).ok();
        }
    }

    if let Err(e) = fs::rename(&tmp, &target) {
        fs::remove_dir_all(&tmp).ok();
        if !target.is_dir() {
            return Err(e).with_context(|| {
                format!("failed to move extracted bundled skills to {}", target.display())
            });
        }
        // Lost the race to a concurrent process; its copy is complete.
    }
    prune_stale(root, &version);
    crate::app_info!(
        "skills",
        "embedded",
        "extracted {} bundled skill files to {}",
        files.len(),
        target.display()
    );
    Ok(target)
}

fn collect_embedded_files() -> Result<Vec<(String, Cow<'static, [u8]>)>> {
    let mut names: Vec<_> = BundledSkillAssets::iter().collect();
    names.sort();
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        if let Some(f) = BundledSkillAssets::get(&name) {
            files.push((name.into_owned(), f.data));
        }
    }
    if files.is_empty() {
        bail!("no bundled skill assets embedded in this build");
    }
    Ok(files)
}

/// Stable digest over (path, content) pairs; length-prefixed to keep field
/// boundaries unambiguous. Truncated hex is plenty for a version label.
fn content_hash(files: &[(String, Cow<'static, [u8]>)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (name, data) in files {
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&(data.len() as u64).to_le_bytes());
        hasher.update(data);
    }
    hasher.finalize().to_hex()[..16].to_string()
}

/// Best-effort removal of extractions from other binary versions and
/// abandoned tmp dirs. Recent `.tmp-*` dirs are spared — they may belong to a
/// concurrent extraction still in flight.
fn prune_stale(root: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == keep {
            continue;
        }
        let path = entry.path();
        if name.starts_with(".tmp-") && !older_than(&path, STALE_TMP_AGE) {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path).ok();
        } else {
            fs::remove_file(&path).ok();
        }
    }
}

fn older_than(path: &Path, age: Duration) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|elapsed| elapsed > age)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_embedded_skills_and_reuses_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bundled-skills");

        let dir = ensure_extracted_in(&root).unwrap();
        assert!(looks_like_skills_dir(&dir), "extraction should contain */SKILL.md");
        assert_eq!(dir.parent().unwrap(), root);

        // A second call must reuse the extraction, not rebuild it.
        let marker = dir.join(".reuse-marker");
        fs::write(&marker, b"x").unwrap();
        let dir2 = ensure_extracted_in(&root).unwrap();
        assert_eq!(dir, dir2);
        assert!(marker.is_file(), "existing extraction was rebuilt");
    }

    #[test]
    fn prunes_stale_versions_and_old_tmp_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bundled-skills");
        let old_version = root.join("0123456789abcdef");
        fs::create_dir_all(old_version.join("some-skill")).unwrap();
        fs::write(old_version.join("some-skill/SKILL.md"), b"old").unwrap();

        let dir = ensure_extracted_in(&root).unwrap();
        assert!(dir.is_dir());
        assert!(!old_version.exists(), "stale version should be pruned");
    }

    #[test]
    fn tolerates_concurrent_winner() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bundled-skills");
        // First extraction stands in for a concurrent process that won the
        // rename; a fresh call must adopt it as-is.
        let dir = ensure_extracted_in(&root).unwrap();
        let dir2 = ensure_extracted_in(&root).unwrap();
        assert_eq!(dir, dir2);
    }

    #[test]
    fn content_hash_is_order_and_boundary_sensitive() {
        let a = vec![("a".to_string(), Cow::Owned(b"bc".to_vec()))];
        let b = vec![("ab".to_string(), Cow::Owned(b"c".to_vec()))];
        assert_ne!(content_hash(&a), content_hash(&b));
        assert_eq!(content_hash(&a), content_hash(&a));
    }
}
