//! Unified file-backed Core Memory repository.
//!
//! V2 canonicalises all scope indexes as uppercase `MEMORY.md`. Global and
//! Agent scopes retain their legacy lowercase files as synchronized mirrors
//! during the compatibility window. A content-free manifest distinguishes an
//! old-version write from a new-version write and fails closed on true
//! two-sided conflicts.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CORE_INDEX_FILE: &str = "MEMORY.md";
pub const CORE_INDEX_MAX_BYTES: usize = 25 * 1024;
pub const CORE_INDEX_MAX_LINES: usize = 200;

const MANIFEST_VERSION: u32 = 1;
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreMemoryScope {
    Global,
    Agent { id: String },
    Project { id: String },
}

impl CoreMemoryScope {
    pub fn key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Agent { id } => format!("agent:{id}"),
            Self::Project { id } => format!("project:{id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoreMemoryPaths {
    pub dir: PathBuf,
    pub canonical: PathBuf,
    pub legacy: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryMigrationState {
    Canonical,
    Mirrored,
    MigratedLegacy,
    RecoveredLegacyChange,
    RecoveredCanonicalChange,
    Conflict,
    Empty,
}

impl CoreMemoryMigrationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Mirrored => "mirrored",
            Self::MigratedLegacy => "migrated_legacy",
            Self::RecoveredLegacyChange => "recovered_legacy_change",
            Self::RecoveredCanonicalChange => "recovered_canonical_change",
            Self::Conflict => "conflict",
            Self::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoreMemoryIndex {
    pub content: Option<String>,
    pub file_hash: Option<String>,
    pub state: CoreMemoryMigrationState,
    pub canonical_path: PathBuf,
    pub legacy_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoreMemoryLayerSnapshot {
    pub content: String,
    pub file_hash: String,
    pub state: CoreMemoryMigrationState,
}

#[derive(Debug, Clone)]
pub(crate) struct CoreMemorySnapshot {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub global: Option<CoreMemoryLayerSnapshot>,
    pub agent: Option<CoreMemoryLayerSnapshot>,
    pub project: Option<CoreMemoryLayerSnapshot>,
    pub fingerprint: String,
}

impl CoreMemorySnapshot {
    pub(crate) fn capture(agent_id: &str, project_id: Option<&str>) -> Result<Self> {
        let global = snapshot_layer(load_index(&CoreMemoryScope::Global)?);
        let agent = snapshot_layer(load_index(&CoreMemoryScope::Agent {
            id: agent_id.to_string(),
        })?);
        let project = project_id
            .map(|id| {
                load_index(&CoreMemoryScope::Project { id: id.to_string() }).map(snapshot_layer)
            })
            .transpose()?
            .flatten();
        let mut hasher = blake3::Hasher::new();
        hasher.update(agent_id.as_bytes());
        if let Some(project_id) = project_id {
            hasher.update(project_id.as_bytes());
        }
        for layer in [&global, &agent, &project].into_iter().flatten() {
            hasher.update(layer.file_hash.as_bytes());
        }
        Ok(Self {
            agent_id: agent_id.to_string(),
            project_id: project_id.map(str::to_string),
            global,
            agent,
            project,
            fingerprint: hasher.finalize().to_hex()[..16].to_string(),
        })
    }

    pub(crate) fn matches_context(&self, agent_id: &str, project_id: Option<&str>) -> bool {
        self.agent_id == agent_id && self.project_id.as_deref() == project_id
    }
}

fn snapshot_layer(index: CoreMemoryIndex) -> Option<CoreMemoryLayerSnapshot> {
    Some(CoreMemoryLayerSnapshot {
        content: index.content?,
        file_hash: index.file_hash?,
        state: index.state,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CoreMemoryMigrationManifest {
    version: u32,
    #[serde(default)]
    scopes: BTreeMap<String, CoreMemoryMigrationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreMemoryMigrationEntry {
    canonical_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_path: Option<String>,
    synced_hash: String,
    snapshot_path: String,
    revision: u64,
    updated_at: String,
}

pub fn paths(scope: &CoreMemoryScope) -> Result<CoreMemoryPaths> {
    match scope {
        CoreMemoryScope::Global => {
            let root = crate::paths::root_dir()?;
            let dir = root.join("memory");
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: Some(root.join("memory.md")),
                dir,
            })
        }
        CoreMemoryScope::Agent { id } => {
            crate::paths::validate_agent_id(id)?;
            let agent_dir = crate::paths::agent_dir(id)?;
            let dir = agent_dir.join("memory");
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: Some(agent_dir.join("memory.md")),
                dir,
            })
        }
        CoreMemoryScope::Project { id } => {
            uuid::Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("invalid project id"))?;
            let dir = crate::project::memory::memory_dir(id)?;
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: None,
                dir,
            })
        }
    }
}

/// Resolve, migrate and load one Core index. This function performs blocking
/// filesystem IO and must be called from the blocking pool in async paths.
pub fn load_index(scope: &CoreMemoryScope) -> Result<CoreMemoryIndex> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    resolve_locked(scope, &resolved)
}

pub fn save_index(
    scope: &CoreMemoryScope,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<CoreMemoryIndex> {
    validate_index_content(content)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    match (current.file_hash.as_deref(), expected_hash) {
        (Some(actual), Some(expected)) if actual != expected => {
            anyhow::bail!("core memory stale-write conflict: read it again before saving")
        }
        (Some(_), None) => anyhow::bail!("expectedHash is required when updating Core Memory"),
        (None, Some(_)) => {
            anyhow::bail!("core memory stale-write conflict: the index no longer exists")
        }
        _ => {}
    }
    synchronize_locked(
        scope,
        &resolved,
        content.as_bytes(),
        SyncKind::CanonicalWrite,
    )?;
    resolve_locked(scope, &resolved)
}

/// Compatibility owner write for existing APIs that predate stale-write
/// hashes. New V2 APIs should use [`save_index`]. Conflict state still blocks
/// the write; otherwise this preserves the existing last-writer-wins surface.
pub fn save_index_owner(scope: &CoreMemoryScope, content: &str) -> Result<CoreMemoryIndex> {
    validate_index_content(content)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    synchronize_locked(
        scope,
        &resolved,
        content.as_bytes(),
        SyncKind::CanonicalWrite,
    )?;
    resolve_locked(scope, &resolved)
}

fn resolve_locked(scope: &CoreMemoryScope, resolved: &CoreMemoryPaths) -> Result<CoreMemoryIndex> {
    let canonical = read_regular_optional(&resolved.canonical)?;
    let legacy = resolved
        .legacy
        .as_deref()
        .map(read_regular_optional)
        .transpose()?
        .flatten();
    let key = scope.key();
    let mut manifest = load_manifest()?;
    let entry = manifest.scopes.get(&key).cloned();

    match (canonical, legacy, entry) {
        (None, None, _) => Ok(index_result(
            resolved,
            None,
            CoreMemoryMigrationState::Empty,
        )),
        (Some(canonical), None, entry) if resolved.legacy.is_none() => {
            let hash = content_hash(&canonical);
            if entry.as_ref().is_none_or(|entry| entry.synced_hash != hash) {
                record_sync_locked(scope, resolved, &canonical, &mut manifest)?;
            }
            Ok(index_result(
                resolved,
                Some(canonical),
                CoreMemoryMigrationState::Canonical,
            ))
        }
        (None, Some(legacy), _) => {
            synchronize_locked(scope, resolved, &legacy, SyncKind::LegacyMigration)?;
            Ok(index_result(
                resolved,
                Some(legacy),
                CoreMemoryMigrationState::MigratedLegacy,
            ))
        }
        (Some(canonical), None, Some(entry)) => {
            if resolved.legacy.is_some() {
                synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalRecovery)?;
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::RecoveredCanonicalChange,
                ))
            } else {
                let _ = entry;
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Canonical,
                ))
            }
        }
        (Some(canonical), None, None) => {
            synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalWrite)?;
            Ok(index_result(
                resolved,
                Some(canonical),
                CoreMemoryMigrationState::Mirrored,
            ))
        }
        (Some(canonical), Some(legacy), None) => {
            if content_hash(&canonical) == content_hash(&legacy) {
                record_sync_locked(scope, resolved, &canonical, &mut manifest)?;
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Mirrored,
                ))
            } else {
                Ok(index_result(
                    resolved,
                    Some(legacy),
                    CoreMemoryMigrationState::Conflict,
                ))
            }
        }
        (Some(canonical), Some(legacy), Some(entry)) => {
            let canonical_hash = content_hash(&canonical);
            let legacy_hash = content_hash(&legacy);
            if canonical_hash == legacy_hash {
                if canonical_hash != entry.synced_hash {
                    record_sync_locked(scope, resolved, &canonical, &mut manifest)?;
                }
                return Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Mirrored,
                ));
            }
            let canonical_changed = canonical_hash != entry.synced_hash;
            let legacy_changed = legacy_hash != entry.synced_hash;
            match (canonical_changed, legacy_changed) {
                (true, false) => {
                    synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalRecovery)?;
                    Ok(index_result(
                        resolved,
                        Some(canonical),
                        CoreMemoryMigrationState::RecoveredCanonicalChange,
                    ))
                }
                (false, true) => {
                    synchronize_locked(scope, resolved, &legacy, SyncKind::LegacyRecovery)?;
                    Ok(index_result(
                        resolved,
                        Some(legacy),
                        CoreMemoryMigrationState::RecoveredLegacyChange,
                    ))
                }
                _ => {
                    let snapshot = read_regular_optional(Path::new(&entry.snapshot_path))?
                        .unwrap_or(canonical);
                    Ok(index_result(
                        resolved,
                        Some(snapshot),
                        CoreMemoryMigrationState::Conflict,
                    ))
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SyncKind {
    LegacyMigration,
    LegacyRecovery,
    CanonicalRecovery,
    CanonicalWrite,
}

fn synchronize_locked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    bytes: &[u8],
    kind: SyncKind,
) -> Result<()> {
    validate_index_bytes(bytes)?;
    if matches!(kind, SyncKind::LegacyMigration) {
        if let Some(legacy) = resolved.legacy.as_deref() {
            backup_legacy_once(scope, legacy, bytes)?;
        }
    }
    crate::platform::write_atomic(&resolved.canonical, bytes)?;
    verify_hash(&resolved.canonical, bytes)?;
    if let Some(legacy) = resolved.legacy.as_deref() {
        crate::platform::write_atomic(legacy, bytes)?;
        verify_hash(legacy, bytes)?;
    }
    let mut manifest = load_manifest()?;
    record_sync_locked(scope, resolved, bytes, &mut manifest)
}

fn record_sync_locked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    bytes: &[u8],
    manifest: &mut CoreMemoryMigrationManifest,
) -> Result<()> {
    let key = scope.key();
    let snapshot = snapshot_path(&key)?;
    if let Some(parent) = snapshot.parent() {
        ensure_real_dir(parent)?;
    }
    crate::platform::write_atomic(&snapshot, bytes)?;
    let previous_revision = manifest.scopes.get(&key).map_or(0, |entry| entry.revision);
    manifest.version = MANIFEST_VERSION;
    manifest.scopes.insert(
        key,
        CoreMemoryMigrationEntry {
            canonical_path: resolved.canonical.display().to_string(),
            legacy_path: resolved
                .legacy
                .as_ref()
                .map(|path| path.display().to_string()),
            synced_hash: content_hash(bytes),
            snapshot_path: snapshot.display().to_string(),
            revision: previous_revision.saturating_add(1),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    save_manifest(manifest)
}

fn index_result(
    resolved: &CoreMemoryPaths,
    bytes: Option<Vec<u8>>,
    state: CoreMemoryMigrationState,
) -> CoreMemoryIndex {
    let (content, file_hash) = bytes.map_or((None, None), |bytes| {
        let hash = content_hash(&bytes);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let content = (!text.trim().is_empty()).then_some(text);
        (content, Some(hash))
    });
    CoreMemoryIndex {
        content,
        file_hash,
        state,
        canonical_path: resolved.canonical.clone(),
        legacy_path: resolved.legacy.clone(),
    }
}

fn validate_index_content(content: &str) -> Result<()> {
    validate_index_bytes(content.as_bytes())
}

fn validate_index_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len() > CORE_INDEX_MAX_BYTES {
        anyhow::bail!("Core MEMORY.md exceeds {} bytes", CORE_INDEX_MAX_BYTES);
    }
    std::str::from_utf8(bytes).context("Core MEMORY.md must be UTF-8")?;
    Ok(())
}

fn read_regular_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory path is not a regular file: {}", path.display());
    }
    if metadata.len() as usize > CORE_INDEX_MAX_BYTES {
        anyhow::bail!("Core MEMORY.md exceeds {} bytes", CORE_INDEX_MAX_BYTES);
    }
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((CORE_INDEX_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    validate_index_bytes(&bytes)?;
    Ok(Some(bytes))
}

fn verify_hash(path: &Path, expected: &[u8]) -> Result<()> {
    let actual = fs::read(path)?;
    if content_hash(&actual) != content_hash(expected) {
        anyhow::bail!(
            "Core Memory migration verification failed for {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_repository_dirs(resolved: &CoreMemoryPaths) -> Result<()> {
    ensure_real_dir(&resolved.dir)?;
    ensure_real_dir(&migration_dir()?)?;
    ensure_real_dir(&snapshot_dir()?)
}

fn ensure_real_dir(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            anyhow::bail!(
                "Core Memory directory must be a real directory: {}",
                path.display()
            );
        }
        return Ok(());
    }
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        anyhow::bail!(
            "Core Memory directory must be a real directory: {}",
            path.display()
        );
    }
    Ok(())
}

struct RepositoryLock {
    _file: fs::File,
}

fn acquire_repository_lock() -> Result<RepositoryLock> {
    let dir = migration_dir()?;
    ensure_real_dir(&dir)?;
    let path = dir.join(".core-memory-v2.lock");
    let started = Instant::now();
    loop {
        match crate::platform::try_acquire_exclusive_lock(&path)? {
            Some(file) => return Ok(RepositoryLock { _file: file }),
            None if started.elapsed() < LOCK_TIMEOUT => std::thread::sleep(LOCK_POLL),
            None => anyhow::bail!("timed out waiting for the Core Memory repository lock"),
        }
    }
}

fn manifest_path() -> Result<PathBuf> {
    Ok(migration_dir()?.join("core-memory-v2.json"))
}

fn migration_dir() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("memory").join("migrations"))
}

fn snapshot_dir() -> Result<PathBuf> {
    Ok(migration_dir()?.join("snapshots"))
}

fn snapshot_path(scope_key: &str) -> Result<PathBuf> {
    let digest = blake3::hash(scope_key.as_bytes()).to_hex();
    Ok(snapshot_dir()?.join(format!("{}.md", &digest[..20])))
}

fn load_manifest() -> Result<CoreMemoryMigrationManifest> {
    let path = manifest_path()?;
    let Some(bytes) = read_regular_optional_unbounded(&path)? else {
        return Ok(CoreMemoryMigrationManifest {
            version: MANIFEST_VERSION,
            scopes: BTreeMap::new(),
        });
    };
    let manifest: CoreMemoryMigrationManifest =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    if manifest.version > MANIFEST_VERSION {
        anyhow::bail!("Core Memory migration manifest is from a newer version");
    }
    Ok(manifest)
}

fn save_manifest(manifest: &CoreMemoryMigrationManifest) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    Ok(crate::platform::write_atomic(&manifest_path()?, &bytes)?)
}

fn read_regular_optional_unbounded(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory metadata path is not a regular file");
    }
    Ok(Some(fs::read(path)?))
}

fn backup_legacy_once(scope: &CoreMemoryScope, _legacy: &Path, bytes: &[u8]) -> Result<()> {
    let digest = blake3::hash(scope.key().as_bytes()).to_hex();
    let backup = migration_dir()?.join(format!(
        "legacy-{}-{}.bak",
        &digest[..12],
        content_hash(bytes)
    ));
    if backup.exists() {
        return Ok(());
    }
    crate::platform::write_atomic(&backup, bytes)?;
    Ok(())
}

fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global_scope() -> CoreMemoryScope {
        CoreMemoryScope::Global
    }

    #[test]
    fn legacy_file_migrates_and_stays_mirrored() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "legacy content").unwrap();
            let index = load_index(&global_scope()).unwrap();
            assert_eq!(index.content.as_deref(), Some("legacy content"));
            assert_eq!(index.state, CoreMemoryMigrationState::MigratedLegacy);
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "legacy content"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("memory.md")).unwrap(),
                "legacy content"
            );
        });
    }

    #[test]
    fn old_version_legacy_write_is_recovered_into_canonical() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "v1").unwrap();
            load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "old binary changed this").unwrap();
            let index = load_index(&global_scope()).unwrap();
            assert_eq!(index.state, CoreMemoryMigrationState::RecoveredLegacyChange);
            assert_eq!(index.content.as_deref(), Some("old binary changed this"));
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "old binary changed this"
            );
        });
    }

    #[test]
    fn two_sided_change_returns_last_synced_snapshot_and_blocks_write() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "synced").unwrap();
            let initial = load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();
            let conflict = load_index(&global_scope()).unwrap();
            assert_eq!(conflict.state, CoreMemoryMigrationState::Conflict);
            assert_eq!(conflict.content.as_deref(), Some("synced"));
            assert!(save_index(&global_scope(), "new", initial.file_hash.as_deref()).is_err());
        });
    }

    #[test]
    fn new_write_updates_both_files_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "v1").unwrap();
            let initial = load_index(&global_scope()).unwrap();
            let saved = save_index(&global_scope(), "v2", initial.file_hash.as_deref()).unwrap();
            assert_eq!(saved.content.as_deref(), Some("v2"));
            assert_eq!(
                fs::read_to_string(temp.path().join("memory.md")).unwrap(),
                "v2"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "v2"
            );
            assert!(temp
                .path()
                .join("memory/migrations/core-memory-v2.json")
                .is_file());
        });
    }

    #[cfg(unix)]
    #[test]
    fn canonical_directory_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let outside = tempfile::tempdir().unwrap();
            symlink(outside.path(), temp.path().join("memory")).unwrap();
            assert!(load_index(&global_scope()).is_err());
        });
    }
}
