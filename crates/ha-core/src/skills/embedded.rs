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

/// Age after which another binary version's extraction is reaped. A live
/// older-version process keeps its resolved path cached (OnceLock) and reads
/// SKILL.md lazily from disk, so deleting other versions eagerly would empty
/// its skills mid-flight during mixed-version coexistence (desktop still on
/// vN while an upgraded server/mcp process extracts vN+1). Recency is tracked
/// via [`LAST_USED_MARKER`], refreshed on every reuse.
const STALE_VERSION_AGE: Duration = Duration::from_secs(24 * 3600);

/// Touch file inside each extraction recording the last time a process
/// adopted it (file mtime is the signal; contents are irrelevant).
const LAST_USED_MARKER: &str = ".last-used";

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
            touch_last_used(&target);
            prune_stale(root, &version);
            return Ok(target);
        }
        // The directory-level rename below is atomic, so a hash-named dir is
        // normally complete — this only fires if a user gutted it by hand.
        // Failing to clear it must be a hard error: otherwise the rename
        // below fails on the leftover and gets misread as "a concurrent
        // extraction won", returning the known-broken dir as Ok.
        fs::remove_dir_all(&target)
            .with_context(|| format!("failed to clear gutted {}", target.display()))?;
    }

    fs::create_dir_all(root)
        .with_context(|| format!("failed to create {}", root.display()))?;
    // pid alone is not unique across pid namespaces (two containers sharing a
    // volume are both pid 1); the nanos nonce keeps concurrent extractions
    // from interleaving into one tree. Abandoned tmp dirs are reaped by
    // `prune_stale` after STALE_TMP_AGE.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = root.join(format!(".tmp-{}-{nonce}", std::process::id()));
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
        let mut f = fs::File::create(&dest)
            .with_context(|| format!("failed to create {}", dest.display()))?;
        use std::io::Write;
        f.write_all(data)
            .with_context(|| format!("failed to write {}", dest.display()))?;
        // Match the repo's write_atomic durability bar: without the fsync, a
        // power loss can journal the dir rename while file data is still
        // unflushed, and the torn (zero-length) cache would pass
        // `looks_like_skills_dir` and be adopted forever (same hash → never
        // re-extracted).
        f.sync_all()
            .with_context(|| format!("failed to sync {}", dest.display()))?;
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
    touch_last_used(&target);
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

/// Record that a process just adopted this extraction (see
/// [`STALE_VERSION_AGE`]). Best-effort — the fallback is the dir's own mtime.
fn touch_last_used(dir: &Path) {
    fs::write(dir.join(LAST_USED_MARKER), b"").ok();
}

/// Best-effort removal of extractions from other binary versions and
/// abandoned tmp dirs. Recent `.tmp-*` dirs are spared (they may belong to a
/// concurrent extraction still in flight); other-version dirs are spared
/// until unused for [`STALE_VERSION_AGE`], so a still-running older binary
/// keeps its extraction during mixed-version coexistence.
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
        if name.starts_with(".tmp-") {
            if !stale_since(&path, STALE_TMP_AGE) {
                continue;
            }
        } else if path.is_dir() {
            let marker = path.join(LAST_USED_MARKER);
            let probe = if marker.is_file() { marker } else { path.clone() };
            if !stale_since(&probe, STALE_VERSION_AGE) {
                continue;
            }
        }
        if path.is_dir() {
            fs::remove_dir_all(&path).ok();
        } else {
            fs::remove_file(&path).ok();
        }
    }
}

/// Whether `path`'s mtime is older than `age`. A FUTURE mtime (clock skew,
/// restored backup) counts as stale — otherwise such leftovers would dodge
/// collection forever. An unreadable mtime spares the entry.
fn stale_since(path: &Path, age: Duration) -> bool {
    match fs::metadata(path).and_then(|m| m.modified()) {
        Ok(mtime) => match mtime.elapsed() {
            Ok(elapsed) => elapsed > age,
            Err(_) => true,
        },
        Err(_) => false,
    }
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
    fn spares_recent_other_versions_and_prunes_stale_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bundled-skills");
        // A freshly-used other-version extraction (live older process) must
        // survive the prune during mixed-version coexistence.
        let fresh = root.join("0123456789abcdef");
        fs::create_dir_all(fresh.join("some-skill")).unwrap();
        fs::write(fresh.join("some-skill/SKILL.md"), b"old").unwrap();
        touch_last_used(&fresh);
        // A long-unused version must be reaped.
        let stale = root.join("fedcba9876543210");
        fs::create_dir_all(stale.join("some-skill")).unwrap();
        fs::write(stale.join("some-skill/SKILL.md"), b"older").unwrap();
        let stale_marker = fs::File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(stale.join(LAST_USED_MARKER))
            .unwrap();
        let past = std::time::SystemTime::now() - (STALE_VERSION_AGE + Duration::from_secs(60));
        stale_marker
            .set_times(fs::FileTimes::new().set_modified(past))
            .unwrap();

        let dir = ensure_extracted_in(&root).unwrap();
        assert!(dir.is_dir());
        assert!(fresh.is_dir(), "recently-used other version must be spared");
        assert!(!stale.exists(), "long-unused version should be pruned");
    }

    #[test]
    fn future_mtime_counts_as_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("x");
        fs::write(&f, b"").unwrap();
        let future = std::time::SystemTime::now() + Duration::from_secs(7200);
        fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(future))
            .unwrap();
        assert!(stale_since(&f, Duration::from_secs(3600)));
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
