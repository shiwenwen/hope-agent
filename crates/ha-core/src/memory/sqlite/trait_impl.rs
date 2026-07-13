use anyhow::{Context, Result};
use rusqlite::{backup, params, Connection, OpenFlags, OptionalExtension, MAIN_DB};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, io::Read, path::Path, sync::Arc, time::Duration};

use super::backend::{row_to_entry, scope_where, SqliteMemoryBackend};
use super::prompt::format_prompt_summary;
use crate::memory::claims::ResolveClaim;
use crate::memory::helpers::{
    load_dedup_config, load_hybrid_search_config, load_temporal_decay_config,
};
use crate::memory::traits::{EmbeddingProvider, MemoryBackend};
use crate::memory::types::*;

// ── MemoryBackend Implementation ────────────────────────────────

const MEMORY_HISTORY_PREVIEW_CHARS: usize = 500;
const MEMORY_HISTORY_DEFAULT_LIMIT: usize = 20;
const MEMORY_HISTORY_MAX_LIMIT: usize = 500;
const MEMORY_HISTORY_MAX_QUERY_CHARS: usize = 200;
const MEMORY_SEARCH_MAX_LITERAL_CHARS: usize = 200;
const MEMORY_DB_SNAPSHOT_DIR: &str = "memory-repair-snapshots";

fn list_active_claims_for_resolver_health(conn: &Connection) -> Result<Vec<ResolveClaim>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.scope_type, c.scope_id, c.claim_type, c.subject, c.predicate, c.object,
                c.content, c.confidence, c.confidence_source, c.salience,
                c.valid_from, c.valid_until,
                (SELECT COUNT(*) FROM memory_evidence e WHERE e.claim_id = c.id),
                (SELECT COUNT(*) FROM memory_evidence e
                 WHERE e.claim_id = c.id
                   AND e.evidence_class IN ('manual_correction', 'user_confirmed')),
                (SELECT COALESCE(MAX(e.weight), 0.0)
                 FROM memory_evidence e WHERE e.claim_id = c.id),
                c.created_at, c.updated_at
         FROM memory_claims c
         WHERE c.status = 'active'
         ORDER BY c.scope_type, c.scope_id, c.claim_type, c.subject, c.predicate, c.created_at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ResolveClaim {
            id: row.get(0)?,
            scope_type: row.get(1)?,
            scope_id: row.get(2)?,
            claim_type: row.get(3)?,
            subject: row.get(4)?,
            predicate: row.get(5)?,
            object: row.get(6)?,
            content: row.get(7)?,
            confidence: row.get::<_, f64>(8)? as f32,
            confidence_source: row.get(9)?,
            salience: row.get::<_, f64>(10)? as f32,
            valid_from: row.get(11)?,
            valid_until: row.get(12)?,
            evidence_count: row.get::<_, i64>(13)?.max(0) as usize,
            manual_evidence_count: row.get::<_, i64>(14)?.max(0) as usize,
            max_evidence_weight: row.get::<_, f64>(15)? as f32,
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        })
    })?;
    Ok(rows.filter_map(|row| row.ok()).collect())
}

fn type_filter_clause(types: Option<&[MemoryType]>) -> String {
    if let Some(types) = types {
        if types.is_empty() {
            "1=1".to_string()
        } else {
            format!(
                "memory_type IN ({})",
                crate::sql_in_placeholders(types.len())
            )
        }
    } else {
        "1=1".to_string()
    }
}

fn source_filter_clause(sources: Option<&[String]>) -> String {
    source_filter_clause_for("source", sources)
}

fn source_filter_clause_for(column: &str, sources: Option<&[String]>) -> String {
    if let Some(sources) = sources {
        if sources.is_empty() {
            "1=1".to_string()
        } else {
            format!(
                "{column} IN ({})",
                crate::sql_in_placeholders(sources.len())
            )
        }
    } else {
        "1=1".to_string()
    }
}

fn action_filter_clause(actions: Option<&[MemoryHistoryAction]>) -> String {
    if let Some(actions) = actions {
        if actions.is_empty() {
            "1=1".to_string()
        } else {
            format!("action IN ({})", crate::sql_in_placeholders(actions.len()))
        }
    } else {
        "1=1".to_string()
    }
}

fn push_type_params(
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    types: Option<&[MemoryType]>,
) {
    if let Some(types) = types {
        for t in types {
            params.push(Box::new(t.as_str().to_string()));
        }
    }
}

fn push_source_params(
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    sources: Option<&[String]>,
) {
    if let Some(sources) = sources {
        for source in sources {
            params.push(Box::new(source.clone()));
        }
    }
}

fn push_history_action_params(
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    actions: Option<&[MemoryHistoryAction]>,
) {
    if let Some(actions) = actions {
        for action in actions {
            params.push(Box::new(action.as_str().to_string()));
        }
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn history_query_pattern(query: Option<&String>) -> Option<String> {
    literal_like_pattern(query?.as_str(), MEMORY_HISTORY_MAX_QUERY_CHARS)
}

fn literal_like_pattern(query: &str, max_chars: usize) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lowered = trimmed
        .chars()
        .take(max_chars)
        .collect::<String>()
        .to_lowercase();
    Some(format!("%{}%", escape_like_pattern(&lowered)))
}

fn trigram_match_query(query: &str, max_chars: usize) -> Option<String> {
    let bounded = query.trim().chars().take(max_chars).collect::<String>();
    if bounded.chars().count() < 3 {
        return None;
    }
    Some(format!("\"{}\"", bounded.replace('"', "\"\"")))
}

fn sqlite_sidecar_path(db_path: &Path, suffix: &str) -> Result<std::path::PathBuf> {
    let mut file_name = db_path
        .file_name()
        .context("memory DB path has no file name")?
        .to_os_string();
    file_name.push(suffix);
    Ok(db_path.with_file_name(file_name))
}

fn copy_snapshot_file(source: &Path, snapshot_dir: &Path) -> Result<Option<String>> {
    if !source.exists() {
        return Ok(None);
    }
    let file_name = source
        .file_name()
        .context("snapshot source has no file name")?
        .to_owned();
    let target = snapshot_dir.join(&file_name);
    fs::copy(source, &target).with_context(|| {
        format!(
            "copying memory DB snapshot file {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(Some(file_name.to_string_lossy().to_string()))
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("opening snapshot file {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading snapshot file {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn is_safe_snapshot_file_name(file_name: &str) -> bool {
    let mut components = Path::new(file_name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

fn snapshot_file_name_issue(file_name: &str) -> Option<String> {
    if !is_safe_snapshot_file_name(file_name) {
        return Some(format!("invalid snapshot file name: {file_name}"));
    }
    if !matches!(file_name, "memory.db" | "memory.db-wal" | "memory.db-shm") {
        return Some(format!("unsupported snapshot DB file name: {file_name}"));
    }
    None
}

fn is_supported_snapshot_db_file_name(file_name: &str) -> bool {
    snapshot_file_name_issue(file_name).is_none()
}

fn quick_check_snapshot_copy(
    snapshot_dir: &Path,
    files: &[MemoryRepairArtifactFile],
) -> Result<String> {
    let temp_dir = tempfile::tempdir().context("creating temporary DB snapshot check dir")?;
    for file in files {
        if !is_supported_snapshot_db_file_name(&file.name) {
            anyhow::bail!(
                "{}",
                snapshot_file_name_issue(&file.name)
                    .unwrap_or_else(|| format!("invalid snapshot file name: {}", file.name))
            );
        }
        fs::copy(
            snapshot_dir.join(&file.name),
            temp_dir.path().join(&file.name),
        )
        .with_context(|| {
            format!(
                "copying snapshot file {} for quick_check",
                snapshot_dir.join(&file.name).display()
            )
        })?;
    }
    let db_path = temp_dir.path().join("memory.db");
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening temporary snapshot DB {}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .context("running quick_check on temporary snapshot DB")
}

fn snapshot_file_manifest(
    snapshot_dir: &Path,
    file_name: &str,
) -> Result<MemoryRepairArtifactFile> {
    let path = snapshot_dir.join(file_name);
    let metadata = fs::metadata(&path)
        .with_context(|| format!("reading snapshot metadata {}", path.display()))?;
    Ok(MemoryRepairArtifactFile {
        name: file_name.to_string(),
        size_bytes: metadata.len(),
        sha256: sha256_file_hex(&path)?,
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DbSnapshotManifest {
    schema_version: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    copied_files: Vec<String>,
    #[serde(default)]
    files: Vec<MemoryRepairArtifactFile>,
}

impl SqliteMemoryBackend {
    fn canonical_snapshot_dir(&self, snapshot_path: &str) -> Result<std::path::PathBuf> {
        let parent = self
            .db_path
            .parent()
            .context("memory DB path has no parent")?;
        let snapshot_root = parent.join(MEMORY_DB_SNAPSHOT_DIR);
        let root = fs::canonicalize(&snapshot_root).with_context(|| {
            format!(
                "reading memory DB snapshot root {}",
                snapshot_root.display()
            )
        })?;
        let requested = Path::new(snapshot_path);
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            snapshot_root.join(requested)
        };
        let canonical = fs::canonicalize(&candidate)
            .with_context(|| format!("reading DB snapshot path {}", candidate.display()))?;
        if !canonical.starts_with(&root) {
            anyhow::bail!("snapshot path is outside the memory repair snapshot directory");
        }
        if !canonical.is_dir() {
            anyhow::bail!("snapshot path is not a directory: {}", canonical.display());
        }
        Ok(canonical)
    }

    fn read_db_snapshot_manifest(&self, snapshot_dir: &Path) -> Result<DbSnapshotManifest> {
        let manifest_path = snapshot_dir.join("manifest.json");
        if !manifest_path.is_file() {
            anyhow::bail!("DB snapshot manifest is missing");
        }
        let manifest: DbSnapshotManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .with_context(|| format!("reading DB snapshot manifest {}", manifest_path.display()))?;
        if manifest.schema_version != "hope.memory.db_snapshot.v1" {
            anyhow::bail!(
                "unsupported DB snapshot schema version: {}",
                manifest.schema_version
            );
        }
        Ok(manifest)
    }

    fn db_snapshot_restore_preview_inner(
        &self,
        snapshot_path: &str,
    ) -> Result<MemoryDbSnapshotRestorePreview> {
        let snapshot_dir = self.canonical_snapshot_dir(snapshot_path)?;
        let manifest = self.read_db_snapshot_manifest(&snapshot_dir)?;
        let current_db_path = self.db_path.to_string_lossy().to_string();
        let mut issues = Vec::new();
        if manifest.files.is_empty() {
            issues.push(
                "snapshot manifest has no stored file metadata; create a fresh snapshot before restore"
                    .to_string(),
            );
            let mut files = Vec::new();
            for name in &manifest.copied_files {
                if let Some(issue) = snapshot_file_name_issue(name) {
                    issues.push(issue);
                    files.push(MemoryDbSnapshotRestoreFileCheck {
                        name: name.clone(),
                        snapshot_path: snapshot_dir.to_string_lossy().to_string(),
                        target_path: self.db_path.to_string_lossy().to_string(),
                        status: MemoryDbSnapshotFileStatus::Missing,
                        expected_size_bytes: 0,
                        actual_size_bytes: None,
                        expected_sha256: String::new(),
                        actual_sha256: None,
                    });
                    continue;
                }
                files.push(MemoryDbSnapshotRestoreFileCheck {
                    name: name.clone(),
                    snapshot_path: snapshot_dir.join(name).to_string_lossy().to_string(),
                    target_path: self
                        .db_path
                        .with_file_name(name)
                        .to_string_lossy()
                        .to_string(),
                    status: MemoryDbSnapshotFileStatus::Unverified,
                    expected_size_bytes: 0,
                    actual_size_bytes: fs::metadata(snapshot_dir.join(name))
                        .ok()
                        .map(|metadata| metadata.len()),
                    expected_sha256: String::new(),
                    actual_sha256: None,
                });
            }
            return Ok(MemoryDbSnapshotRestorePreview {
                snapshot_path: snapshot_dir.to_string_lossy().to_string(),
                current_db_path,
                created_at: manifest.created_at,
                status: MemoryDbSnapshotRestoreStatus::NoMetadata,
                can_restore: false,
                quick_check: "not_checked".to_string(),
                issues,
                files,
            });
        }

        let mut file_checks = Vec::new();
        for expected in &manifest.files {
            if let Some(issue) = snapshot_file_name_issue(&expected.name) {
                issues.push(issue);
                file_checks.push(MemoryDbSnapshotRestoreFileCheck {
                    name: expected.name.clone(),
                    snapshot_path: snapshot_dir.to_string_lossy().to_string(),
                    target_path: self.db_path.to_string_lossy().to_string(),
                    status: MemoryDbSnapshotFileStatus::Missing,
                    expected_size_bytes: expected.size_bytes,
                    actual_size_bytes: None,
                    expected_sha256: expected.sha256.clone(),
                    actual_sha256: None,
                });
                continue;
            }
            let source_path = snapshot_dir.join(&expected.name);
            let target_path = self.db_path.with_file_name(&expected.name);
            let metadata = fs::metadata(&source_path).ok();
            let actual_size = metadata.as_ref().map(|metadata| metadata.len());
            let actual_sha = if metadata.is_some() {
                sha256_file_hex(&source_path).ok()
            } else {
                None
            };
            let status = if metadata.is_none() {
                issues.push(format!("missing file: {}", expected.name));
                MemoryDbSnapshotFileStatus::Missing
            } else if actual_size != Some(expected.size_bytes) {
                issues.push(format!(
                    "size mismatch: {} expected {} bytes, found {} bytes",
                    expected.name,
                    expected.size_bytes,
                    actual_size.unwrap_or(0)
                ));
                MemoryDbSnapshotFileStatus::SizeMismatch
            } else if actual_sha.as_deref() != Some(expected.sha256.as_str()) {
                issues.push(format!("sha256 mismatch: {}", expected.name));
                MemoryDbSnapshotFileStatus::Sha256Mismatch
            } else {
                MemoryDbSnapshotFileStatus::Ok
            };
            file_checks.push(MemoryDbSnapshotRestoreFileCheck {
                name: expected.name.clone(),
                snapshot_path: source_path.to_string_lossy().to_string(),
                target_path: target_path.to_string_lossy().to_string(),
                status,
                expected_size_bytes: expected.size_bytes,
                actual_size_bytes: actual_size,
                expected_sha256: expected.sha256.clone(),
                actual_sha256: actual_sha,
            });
        }
        if !manifest.files.iter().any(|file| file.name == "memory.db") {
            issues.push("missing manifest entry: memory.db".to_string());
        }

        let mut quick_check = "not_checked".to_string();
        let mut status = if issues.iter().any(|issue| {
            issue.starts_with("missing manifest entry:")
                || issue.starts_with("invalid snapshot file name:")
                || issue.starts_with("unsupported snapshot DB file name:")
        }) || file_checks
            .iter()
            .any(|file| file.status == MemoryDbSnapshotFileStatus::Missing)
        {
            MemoryDbSnapshotRestoreStatus::MissingFiles
        } else if file_checks
            .iter()
            .any(|file| file.status == MemoryDbSnapshotFileStatus::SizeMismatch)
        {
            MemoryDbSnapshotRestoreStatus::SizeMismatch
        } else if file_checks
            .iter()
            .any(|file| file.status == MemoryDbSnapshotFileStatus::Sha256Mismatch)
        {
            MemoryDbSnapshotRestoreStatus::Sha256Mismatch
        } else {
            MemoryDbSnapshotRestoreStatus::Ready
        };
        if status == MemoryDbSnapshotRestoreStatus::Ready {
            match quick_check_snapshot_copy(&snapshot_dir, &manifest.files) {
                Ok(value) => {
                    quick_check = value;
                    if quick_check != "ok" {
                        issues.push(format!("snapshot quick_check returned {quick_check}"));
                        status = MemoryDbSnapshotRestoreStatus::QuickCheckFailed;
                    }
                }
                Err(err) => {
                    quick_check = format!("error: {err}");
                    issues.push(format!("snapshot quick_check failed: {err}"));
                    status = MemoryDbSnapshotRestoreStatus::QuickCheckFailed;
                }
            }
        }
        let can_restore = status == MemoryDbSnapshotRestoreStatus::Ready;
        Ok(MemoryDbSnapshotRestorePreview {
            snapshot_path: snapshot_dir.to_string_lossy().to_string(),
            current_db_path,
            created_at: manifest.created_at,
            status,
            can_restore,
            quick_check,
            issues,
            files: file_checks,
        })
    }

    fn read_db_snapshot_artifact(
        &self,
        snapshot_dir: &Path,
    ) -> Result<Option<MemoryDbSnapshotArtifact>> {
        let manifest_path = snapshot_dir.join("manifest.json");
        if !manifest_path.is_file() {
            return Ok(None);
        }
        let manifest = self.read_db_snapshot_manifest(snapshot_dir)?;
        let mut issues = Vec::new();
        let files = if manifest.files.is_empty() {
            let mut out = Vec::new();
            for file in &manifest.copied_files {
                if let Some(issue) = snapshot_file_name_issue(file) {
                    issues.push(issue);
                    continue;
                }
                match snapshot_file_manifest(snapshot_dir, file) {
                    Ok(metadata) => out.push(metadata),
                    Err(_) => issues.push(format!("missing file: {file}")),
                }
            }
            out
        } else {
            for file in &manifest.files {
                if let Some(issue) = snapshot_file_name_issue(&file.name) {
                    issues.push(issue);
                    continue;
                }
                let path = snapshot_dir.join(&file.name);
                match fs::metadata(&path) {
                    Ok(metadata) if metadata.len() != file.size_bytes => issues.push(format!(
                        "size mismatch: {} expected {} bytes, found {} bytes",
                        file.name,
                        file.size_bytes,
                        metadata.len()
                    )),
                    Ok(_) => {}
                    Err(_) => issues.push(format!("missing file: {}", file.name)),
                }
            }
            manifest.files
        };
        if !files.is_empty() && !files.iter().any(|file| file.name == "memory.db") {
            issues.push("missing manifest entry: memory.db".to_string());
        }
        let status = if files.is_empty() && issues.is_empty() {
            MemoryDbSnapshotStatus::NoMetadata
        } else if issues.iter().any(|issue| {
            issue.starts_with("missing file:")
                || issue.starts_with("missing manifest entry:")
                || issue.starts_with("invalid snapshot file name:")
                || issue.starts_with("unsupported snapshot DB file name:")
        }) {
            MemoryDbSnapshotStatus::MissingFiles
        } else if issues
            .iter()
            .any(|issue| issue.starts_with("size mismatch:"))
        {
            MemoryDbSnapshotStatus::SizeMismatch
        } else {
            MemoryDbSnapshotStatus::Ok
        };
        Ok(Some(MemoryDbSnapshotArtifact {
            path: snapshot_dir.to_string_lossy().to_string(),
            created_at: manifest.created_at,
            status,
            issues,
            files,
        }))
    }

    fn latest_db_snapshot(&self) -> Result<Option<MemoryDbSnapshotArtifact>> {
        let parent = self
            .db_path
            .parent()
            .context("memory DB path has no parent")?;
        let snapshot_root = parent.join(MEMORY_DB_SNAPSHOT_DIR);
        if !snapshot_root.is_dir() {
            return Ok(None);
        }
        let mut latest: Option<MemoryDbSnapshotArtifact> = None;
        for entry in fs::read_dir(&snapshot_root)
            .with_context(|| format!("reading DB snapshot root {}", snapshot_root.display()))?
        {
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(candidate) = self.read_db_snapshot_artifact(&entry.path()).ok().flatten()
            else {
                continue;
            };
            let candidate_key = candidate
                .created_at
                .clone()
                .unwrap_or_else(|| candidate.path.clone());
            let latest_key = latest
                .as_ref()
                .map(|artifact| {
                    artifact
                        .created_at
                        .clone()
                        .unwrap_or_else(|| artifact.path.clone())
                })
                .unwrap_or_default();
            if candidate_key > latest_key {
                latest = Some(candidate);
            }
        }
        Ok(latest)
    }

    fn create_db_snapshot(
        &self,
        before: &MemoryHealth,
    ) -> Result<(String, Vec<MemoryRepairArtifactFile>)> {
        let _writer = self.write_conn()?;
        self.create_db_snapshot_locked(before)
    }

    fn create_db_snapshot_locked(
        &self,
        before: &MemoryHealth,
    ) -> Result<(String, Vec<MemoryRepairArtifactFile>)> {
        let db_path = &self.db_path;
        let parent = db_path.parent().context("memory DB path has no parent")?;
        let now = chrono::Utc::now();
        let dir_name = format!("{}-{}", now.format("%Y%m%dT%H%M%SZ"), uuid::Uuid::new_v4());
        let snapshot_dir = parent.join(MEMORY_DB_SNAPSHOT_DIR).join(dir_name);
        fs::create_dir_all(&snapshot_dir).with_context(|| {
            format!("creating memory DB snapshot dir {}", snapshot_dir.display())
        })?;

        let mut copied_files = Vec::new();
        if let Some(file) = copy_snapshot_file(db_path, &snapshot_dir)? {
            copied_files.push(file);
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = sqlite_sidecar_path(db_path, suffix)?;
            if let Some(file) = copy_snapshot_file(&sidecar, &snapshot_dir)? {
                copied_files.push(file);
            }
        }

        if copied_files.is_empty() {
            anyhow::bail!("memory DB snapshot did not copy any files");
        }
        let file_manifests = copied_files
            .iter()
            .map(|file| snapshot_file_manifest(&snapshot_dir, file))
            .collect::<Result<Vec<_>>>()?;

        let manifest = serde_json::json!({
            "schemaVersion": "hope.memory.db_snapshot.v1",
            "createdAt": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "sourcePath": db_path.to_string_lossy(),
            "quickCheck": before.quick_check,
            "healthStatus": before.status,
            "copiedFiles": copied_files,
            "files": &file_manifests,
            "note": "Raw SQLite safety snapshot created before/around repair; keep all copied files together when restoring manually."
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        crate::platform::write_atomic(&snapshot_dir.join("manifest.json"), &manifest_bytes)
            .with_context(|| format!("writing snapshot manifest in {}", snapshot_dir.display()))?;

        Ok((snapshot_dir.to_string_lossy().to_string(), file_manifests))
    }

    fn lock_all_readers(&self) -> Result<Vec<std::sync::MutexGuard<'_, Connection>>> {
        let mut guards = Vec::with_capacity(self.readers.len());
        for reader in &self.readers {
            guards.push(
                reader
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Read pool lock poisoned: {e}"))?,
            );
        }
        Ok(guards)
    }

    fn restore_snapshot_into_writer(
        &self,
        writer: &mut Connection,
        preview: &MemoryDbSnapshotRestorePreview,
    ) -> Result<()> {
        if !preview.can_restore {
            anyhow::bail!(
                "DB snapshot restore preflight is not ready: {}",
                preview.status.as_str()
            );
        }
        let source_db = Path::new(&preview.snapshot_path).join("memory.db");
        let source = Connection::open_with_flags(&source_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening restore source DB {}", source_db.display()))?;
        {
            let restore = backup::Backup::new_with_names(&source, MAIN_DB, writer, MAIN_DB)
                .context("initializing SQLite restore from snapshot")?;
            restore
                .run_to_completion(100, Duration::from_millis(10), None)
                .context("restoring SQLite memory DB from snapshot")?;
        }
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        writer.busy_timeout(Duration::from_secs(5))?;
        Ok(())
    }

    fn restore_snapshot_without_rollback(
        &self,
        preview: &MemoryDbSnapshotRestorePreview,
    ) -> Result<()> {
        let mut writer = self.write_conn()?;
        let _readers = self.lock_all_readers()?;
        self.restore_snapshot_into_writer(&mut writer, preview)
    }

    fn db_snapshot_restore_inner(
        &self,
        snapshot_path: &str,
    ) -> Result<MemoryDbSnapshotRestoreReport> {
        let preflight = self.db_snapshot_restore_preview_inner(snapshot_path)?;
        if !preflight.can_restore {
            anyhow::bail!(
                "DB snapshot restore preflight blocked restore: {}",
                preflight.status.as_str()
            );
        }
        let before = self.health()?;
        let (rollback_snapshot_path, rollback_snapshot_files) = {
            let mut writer = self.write_conn()?;
            let _readers = self.lock_all_readers()?;
            let rollback = self.create_db_snapshot_locked(&before)?;
            self.restore_snapshot_into_writer(&mut writer, &preflight)?;
            rollback
        };

        let after = self.health()?;
        if after.quick_check != "ok" {
            let rollback_preview =
                self.db_snapshot_restore_preview_inner(&rollback_snapshot_path)?;
            if rollback_preview.can_restore {
                let _ = self.restore_snapshot_without_rollback(&rollback_preview);
            }
            anyhow::bail!(
                "restored DB failed post-restore quick_check ({}); rollback snapshot: {}",
                after.quick_check,
                rollback_snapshot_path
            );
        }

        Ok(MemoryDbSnapshotRestoreReport {
            restored: true,
            snapshot_path: preflight.snapshot_path.clone(),
            rollback_snapshot_path,
            rollback_snapshot_files,
            preflight,
            before,
            after,
        })
    }
}

fn sqlite_table_exists(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    let exists = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'virtual table') AND name = ?1",
        params![name],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists > 0)
}

fn sqlite_count_or_zero(conn: &rusqlite::Connection, sql: &str) -> usize {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .map(|v| v.max(0) as usize)
        .unwrap_or(0)
}

fn sqlite_count_with_text_param_or_zero(
    conn: &rusqlite::Connection,
    sql: &str,
    value: &str,
) -> usize {
    conn.query_row(sql, params![value], |row| row.get::<_, i64>(0))
        .map(|v| v.max(0) as usize)
        .unwrap_or(0)
}

fn sqlite_count_orphan_procedure_episode_refs(conn: &Connection) -> usize {
    let mut episode_ids = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT id FROM memory_episodes") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for id in rows.flatten() {
                episode_ids.insert(id);
            }
        }
    }

    let mut orphan_refs = 0usize;
    if let Ok(mut stmt) = conn.prepare("SELECT source_episode_ids_json FROM memory_procedures") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for raw in rows.flatten() {
                let source_ids: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
                for source_id in source_ids {
                    let source_id = source_id.trim();
                    if !source_id.is_empty() && !episode_ids.contains(source_id) {
                        orphan_refs += 1;
                    }
                }
            }
        }
    }
    orphan_refs
}

fn sqlite_repair_orphan_procedure_episode_refs(conn: &Connection) -> Result<usize> {
    if !sqlite_table_exists(conn, "memory_episodes")?
        || !sqlite_table_exists(conn, "memory_procedures")?
    {
        return Ok(0);
    }

    let mut episode_ids = HashSet::new();
    {
        let mut stmt = conn.prepare("SELECT id FROM memory_episodes")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for id in rows {
            episode_ids.insert(id?);
        }
    }

    let mut updates: Vec<(String, String)> = Vec::new();
    let mut removed = 0usize;
    {
        let mut stmt = conn.prepare("SELECT id, source_episode_ids_json FROM memory_procedures")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (procedure_id, raw) = row?;
            let source_ids: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            let mut retained = Vec::with_capacity(source_ids.len());
            let mut removed_from_row = 0usize;
            for source_id in source_ids {
                let trimmed = source_id.trim();
                if trimmed.is_empty() || episode_ids.contains(trimmed) {
                    retained.push(source_id);
                } else {
                    removed_from_row += 1;
                }
            }
            if removed_from_row > 0 {
                removed += removed_from_row;
                updates.push((procedure_id, serde_json::to_string(&retained)?));
            }
        }
    }

    if !updates.is_empty() {
        let now = crate::util::now_rfc3339();
        for (procedure_id, source_episode_ids_json) in updates {
            conn.execute(
                "UPDATE memory_procedures
                 SET source_episode_ids_json = ?2,
                     updated_at = ?3
                 WHERE id = ?1",
                params![procedure_id, source_episode_ids_json, now],
            )?;
        }
    }

    Ok(removed)
}

fn memory_content_preview(content: &str) -> String {
    let mut preview = content
        .chars()
        .take(MEMORY_HISTORY_PREVIEW_CHARS)
        .collect::<String>();
    if content.chars().nth(MEMORY_HISTORY_PREVIEW_CHARS).is_some() {
        preview.push_str("...");
    }
    preview
}

fn scope_columns(scope: &MemoryScope) -> (&'static str, Option<&str>, Option<&str>) {
    match scope {
        MemoryScope::Global => ("global", None, None),
        MemoryScope::Agent { id } => ("agent", Some(id.as_str()), None),
        MemoryScope::Project { id } => ("project", None, Some(id.as_str())),
    }
}

fn load_memory_entry_for_history(conn: &Connection, id: i64) -> Result<Option<MemoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, scope_type, scope_agent_id, scope_project_id, content, tags,
                source, source_session_id, pinned, created_at, updated_at,
                attachment_path, attachment_mime
         FROM memories WHERE id = ?1",
    )?;
    Ok(stmt.query_row(params![id], row_to_entry).optional()?)
}

fn load_memory_entries_for_history(conn: &Connection, ids: &[i64]) -> Result<Vec<MemoryEntry>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = crate::sql_in_placeholders(ids.len());
    let sql = format!(
        "SELECT id, memory_type, scope_type, scope_agent_id, scope_project_id, content, tags,
                source, source_session_id, pinned, created_at, updated_at,
                attachment_path, attachment_mime
         FROM memories WHERE id IN ({})",
        placeholders
    );
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), row_to_entry)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn row_to_history_record(row: &rusqlite::Row) -> rusqlite::Result<MemoryHistoryRecord> {
    let scope_type: String = row.get("scope_type")?;
    let scope_agent_id: Option<String> = row.get("scope_agent_id")?;
    let scope_project_id: Option<String> = row.get("scope_project_id")?;
    let scope = match scope_type.as_str() {
        "agent" => MemoryScope::Agent {
            id: scope_agent_id.unwrap_or_default(),
        },
        "project" => MemoryScope::Project {
            id: scope_project_id.unwrap_or_default(),
        },
        _ => MemoryScope::Global,
    };
    let action: String = row.get("action")?;
    let memory_type: String = row.get("memory_type")?;
    let pinned: i64 = row.get("pinned")?;

    Ok(MemoryHistoryRecord {
        id: row.get("id")?,
        memory_id: row.get("memory_id")?,
        action: MemoryHistoryAction::from_str(&action),
        memory_type: MemoryType::from_str(&memory_type),
        scope,
        source: row.get("source")?,
        source_session_id: row.get("source_session_id")?,
        content_preview: row.get("content_preview")?,
        pinned: pinned != 0,
        created_at: row.get("created_at")?,
    })
}

fn record_memory_history(
    conn: &Connection,
    action: MemoryHistoryAction,
    entry: &MemoryEntry,
    created_at: &str,
) -> Result<()> {
    let (scope_type, scope_agent_id, scope_project_id) = scope_columns(&entry.scope);
    conn.execute(
        "INSERT INTO memory_history
            (id, memory_id, action, memory_type, scope_type, scope_agent_id, scope_project_id,
             source, source_session_id, content_preview, pinned, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            uuid::Uuid::new_v4().to_string(),
            entry.id,
            action.as_str(),
            entry.memory_type.as_str(),
            scope_type,
            scope_agent_id,
            scope_project_id,
            entry.source.as_str(),
            entry.source_session_id.as_deref(),
            memory_content_preview(&entry.content),
            entry.pinned as i64,
            created_at,
        ],
    )?;
    Ok(())
}

fn record_memory_history_best_effort(
    conn: &Connection,
    action: MemoryHistoryAction,
    entry: &MemoryEntry,
    created_at: &str,
) {
    if let Err(e) = record_memory_history(conn, action, entry, created_at) {
        crate::app_warn!(
            "memory",
            "history_record",
            "failed to record memory history for {}: {}",
            entry.id,
            e
        );
    }
}

impl MemoryBackend for SqliteMemoryBackend {
    fn add(&self, entry: NewMemory) -> Result<i64> {
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(&entry.tags)?;

        let (scope_type, scope_agent_id, scope_project_id) = match &entry.scope {
            MemoryScope::Global => ("global", None, None),
            MemoryScope::Agent { id } => ("agent", Some(id.as_str()), None),
            MemoryScope::Project { id } => ("project", None, Some(id.as_str())),
        };

        // Generate embedding: multimodal if attachment present + supported, else text-only
        let embedding = if let (Some(ref att_path), Some(ref att_mime)) =
            (&entry.attachment_path, &entry.attachment_mime)
        {
            self.generate_multimodal_embedding(&entry.content, att_path, att_mime)
        } else {
            self.generate_embedding(&entry.content)
        };
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());
        let embedding_signature = embedding_bytes
            .as_ref()
            .and_then(|_| crate::memory::helpers::active_embedding_signature());

        // `generate_embedding` uses the same backend connections for its cache.
        // Never hold the writer while calling it: the cache-miss path writes to
        // `embedding_cache` and would otherwise re-enter this non-reentrant mutex.
        let conn = self.write_conn()?;

        conn.execute(
            "INSERT INTO memories (memory_type, scope_type, scope_agent_id, scope_project_id, content, tags, source, source_session_id, embedding, embedding_signature, pinned, created_at, updated_at, attachment_path, attachment_mime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                entry.memory_type.as_str(),
                scope_type,
                scope_agent_id,
                scope_project_id,
                entry.content,
                tags_json,
                entry.source,
                entry.source_session_id,
                embedding_bytes,
                embedding_signature,
                entry.pinned as i64,
                now,
                now,
                entry.attachment_path,
                entry.attachment_mime,
            ],
        )?;

        let row_id = conn.last_insert_rowid();

        // Insert into vec0 table if embedding was generated
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![row_id, emb_bytes],
                );
            }
        }

        if let Some(entry) = load_memory_entry_for_history(&conn, row_id)? {
            let action = if entry.source == "import" {
                MemoryHistoryAction::Import
            } else {
                MemoryHistoryAction::Add
            };
            record_memory_history_best_effort(&conn, action, &entry, &now);
        }

        Ok(row_id)
    }

    fn update(&self, id: i64, content: &str, tags: &[String]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let tags_json = serde_json::to_string(tags)?;

        // Regenerate embedding if provider is configured
        let embedding = self.generate_embedding(content);
        let embedding_bytes: Option<Vec<u8>> = embedding
            .as_ref()
            .map(|v| v.iter().flat_map(|f| f.to_le_bytes()).collect());
        let embedding_signature = embedding_bytes
            .as_ref()
            .and_then(|_| crate::memory::helpers::active_embedding_signature());

        // Keep remote/local embedding and its cache IO outside the writer lock.
        let conn = self.write_conn()?;

        let affected = conn.execute(
            "UPDATE memories SET content = ?1, tags = ?2, embedding = ?3, embedding_signature = ?4, updated_at = ?5 WHERE id = ?6",
            params![content, tags_json, embedding_bytes, embedding_signature, now, id],
        )?;

        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }

        // Update vec0 table
        if let Some(ref emb_bytes) = embedding_bytes {
            let dims = self
                .embedding_dims
                .load(std::sync::atomic::Ordering::Relaxed);
            if dims > 0 {
                let _ = self.ensure_vec_table(&conn, dims);
                // Delete old vector + insert new
                let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![id, emb_bytes],
                );
            }
        } else {
            let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
        }

        if let Some(entry) = load_memory_entry_for_history(&conn, id)? {
            record_memory_history_best_effort(&conn, MemoryHistoryAction::Update, &entry, &now);
        }

        Ok(())
    }

    fn toggle_pin(&self, id: i64, pinned: bool) -> Result<()> {
        let conn = self.write_conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let affected = conn.execute(
            "UPDATE memories SET pinned = ?1, updated_at = ?2 WHERE id = ?3",
            params![pinned as i64, now, id],
        )?;
        if affected == 0 {
            anyhow::bail!("Memory with id {} not found", id);
        }
        if let Some(entry) = load_memory_entry_for_history(&conn, id)? {
            record_memory_history_best_effort(
                &conn,
                if pinned {
                    MemoryHistoryAction::Pin
                } else {
                    MemoryHistoryAction::Unpin
                },
                &entry,
                &now,
            );
        }
        Ok(())
    }

    fn delete(&self, id: i64) -> Result<()> {
        let conn = self.write_conn()?;
        let before = load_memory_entry_for_history(&conn, id)?;
        let now = chrono::Utc::now().to_rfc3339();
        // Delete from vec0 first (if table exists)
        let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
        let affected = conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        if affected > 0 {
            if let Some(entry) = before {
                record_memory_history_best_effort(&conn, MemoryHistoryAction::Delete, &entry, &now);
            }
        }
        Ok(())
    }

    fn get(&self, id: i64) -> Result<Option<MemoryEntry>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, memory_type, scope_type, scope_agent_id, scope_project_id, content, tags, source, source_session_id, pinned, created_at, updated_at, attachment_path, attachment_mime
             FROM memories WHERE id = ?1",
        )?;

        let entry = stmt.query_row(params![id], row_to_entry).optional()?;
        Ok(entry)
    }

    fn list(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>> {
        self.list_filtered(scope, types, None, limit, offset)
    }

    fn history(&self, limit: usize, offset: usize) -> Result<Vec<MemoryHistoryRecord>> {
        self.history_filtered(&MemoryHistoryQuery {
            limit: Some(limit),
            offset: Some(offset),
            ..Default::default()
        })
    }

    fn history_filtered(&self, query: &MemoryHistoryQuery) -> Result<Vec<MemoryHistoryRecord>> {
        let conn = self.read_conn()?;
        let limit = query
            .limit
            .unwrap_or(MEMORY_HISTORY_DEFAULT_LIMIT)
            .clamp(1, MEMORY_HISTORY_MAX_LIMIT);
        let offset = query.offset.unwrap_or(0).min(100_000);
        let action_clause = action_filter_clause(query.actions.as_deref());
        let type_clause = type_filter_clause(query.memory_types.as_deref());
        let source_clause = source_filter_clause(query.sources.as_deref());
        let query_pattern = history_query_pattern(query.query.as_ref());
        let text_clause = if query_pattern.is_some() {
            "(lower(content_preview) LIKE ? ESCAPE '\\'
              OR lower(source) LIKE ? ESCAPE '\\'
              OR lower(coalesce(source_session_id, '')) LIKE ? ESCAPE '\\'
              OR lower(action) LIKE ? ESCAPE '\\'
              OR lower(memory_type) LIKE ? ESCAPE '\\')"
        } else {
            "1=1"
        };
        let sql = format!(
            "SELECT id, memory_id, action, memory_type, scope_type, scope_agent_id,
                    scope_project_id, source, source_session_id, content_preview,
                    pinned, created_at
             FROM memory_history
             WHERE {action_clause}
               AND {type_clause}
               AND {source_clause}
               AND {text_clause}
             ORDER BY created_at DESC, rowid DESC
             LIMIT ? OFFSET ?"
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        push_history_action_params(&mut params, query.actions.as_deref());
        push_type_params(&mut params, query.memory_types.as_deref());
        push_source_params(&mut params, query.sources.as_deref());
        if let Some(pattern) = query_pattern {
            for _ in 0..5 {
                params.push(Box::new(pattern.clone()));
            }
        }
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), row_to_history_record)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn history_filtered_page(
        &self,
        query: &MemoryHistoryQuery,
    ) -> Result<MemoryHistoryListResponse> {
        let items = self.history_filtered(query)?;
        let conn = self.read_conn()?;
        let action_clause = action_filter_clause(query.actions.as_deref());
        let type_clause = type_filter_clause(query.memory_types.as_deref());
        let source_clause = source_filter_clause(query.sources.as_deref());
        let query_pattern = history_query_pattern(query.query.as_ref());
        let text_clause = if query_pattern.is_some() {
            "(lower(content_preview) LIKE ? ESCAPE '\\'
              OR lower(source) LIKE ? ESCAPE '\\'
              OR lower(coalesce(source_session_id, '')) LIKE ? ESCAPE '\\'
              OR lower(action) LIKE ? ESCAPE '\\'
              OR lower(memory_type) LIKE ? ESCAPE '\\')"
        } else {
            "1=1"
        };
        let sql = format!(
            "SELECT COUNT(*)
             FROM memory_history
             WHERE {action_clause}
               AND {type_clause}
               AND {source_clause}
               AND {text_clause}"
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        push_history_action_params(&mut params, query.actions.as_deref());
        push_type_params(&mut params, query.memory_types.as_deref());
        push_source_params(&mut params, query.sources.as_deref());
        if let Some(pattern) = query_pattern {
            for _ in 0..5 {
                params.push(Box::new(pattern.clone()));
            }
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(MemoryHistoryListResponse {
            items,
            total: total.max(0) as usize,
            total_truncated: false,
        })
    }

    fn import_history(&self, records: &[MemoryHistoryRecord]) -> Result<usize> {
        if records.is_empty() {
            return Ok(0);
        }
        let conn = self.write_conn()?;
        let mut inserted = 0usize;
        for record in records {
            let (scope_type, scope_agent_id, scope_project_id) = scope_columns(&record.scope);
            inserted += conn.execute(
                "INSERT OR IGNORE INTO memory_history
                    (id, memory_id, action, memory_type, scope_type, scope_agent_id, scope_project_id,
                     source, source_session_id, content_preview, pinned, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    record.id.as_str(),
                    record.memory_id,
                    record.action.as_str(),
                    record.memory_type.as_str(),
                    scope_type,
                    scope_agent_id,
                    scope_project_id,
                    record.source.as_str(),
                    record.source_session_id.as_deref(),
                    memory_content_preview(&record.content_preview),
                    record.pinned as i64,
                    record.created_at.as_str(),
                ],
            )?;
        }
        Ok(inserted)
    }

    fn list_filtered(
        &self,
        scope: Option<&MemoryScope>,
        types: Option<&[MemoryType]>,
        sources: Option<&[String]>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MemoryEntry>> {
        let conn = self.read_conn()?;

        let (scope_clause, mut scope_params) = scope_where(scope, None);
        let type_clause = type_filter_clause(types);
        let source_clause = source_filter_clause(sources);

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, scope_project_id, content, tags, source, source_session_id, pinned, created_at, updated_at, attachment_path, attachment_mime
             FROM memories
             WHERE {} AND {} AND {}
             ORDER BY pinned DESC, updated_at DESC
             LIMIT ? OFFSET ?",
            scope_clause, type_clause, source_clause
        );

        let mut stmt = conn.prepare(&sql)?;

        // Build params: scope_params + type_params + limit + offset
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.append(&mut scope_params);
        push_type_params(&mut all_params, types);
        push_source_params(&mut all_params, sources);
        all_params.push(Box::new(limit as i64));
        all_params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let entries = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    fn search(&self, query: &MemorySearchQuery) -> Result<Vec<MemoryEntry>> {
        let requested_limit = query.limit.unwrap_or(20);
        if requested_limit == 0 {
            return Ok(Vec::new());
        }
        let limit = requested_limit.min(200);
        let candidate_limit = limit.saturating_mul(3).min(600);

        // Load configurable search parameters
        let hybrid_cfg = load_hybrid_search_config();
        let decay_cfg = load_temporal_decay_config();

        // Try hybrid search (FTS5 + vector), fall back to FTS5-only
        let active_signature = crate::memory::helpers::active_embedding_signature();
        let query_embedding = if active_signature.is_some() {
            self.generate_embedding(&query.query)
        } else {
            None
        };
        let has_vec = query_embedding.is_some();

        // `generate_embedding` performs cache reads/writes through this backend.
        // Compute it before reserving a pooled reader so concurrent searches can
        // never exhaust the pool while each waits for a nested cache reader.
        let conn = self.read_conn()?;

        // ── Step 1: FTS5 keyword search (with query expansion) ──
        let mut fts_results: Vec<(i64, f64)> = Vec::new(); // (id, rank)

        if let Some(fts_query) = crate::memory::helpers::expand_query(&query.query) {
            let (fts_scope_clause, mut fts_scope_params) =
                scope_where(query.scope.as_ref(), query.agent_id.as_deref());
            let fts_type_clause = type_filter_clause(query.types.as_deref());
            let fts_source_clause = source_filter_clause_for("m.source", query.sources.as_deref());
            let sql = format!(
                "SELECT fts.rowid, rank
                 FROM memories_fts fts
                 JOIN memories m ON m.id = fts.rowid
                 WHERE memories_fts MATCH ?
                   AND {} AND {} AND {}
                 ORDER BY rank LIMIT ?",
                fts_scope_clause, fts_type_clause, fts_source_clause
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(fts_query)];
            params.append(&mut fts_scope_params);
            push_type_params(&mut params, query.types.as_deref());
            push_source_params(&mut params, query.sources.as_deref());
            params.push(Box::new(candidate_limit as i64));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for r in rows.flatten() {
                fts_results.push(r);
            }
        }

        // ── Step 1b: Literal lexical fallback ──
        //
        // FTS5's unicode61 tokenizer remains the primary word/token path. A
        // rebuildable trigram shadow index covers CJK fragments and identifier
        // infixes; only <3-character queries or an unavailable shadow index
        // fall back to the bounded LIKE compatibility path.
        let mut literal_results: Vec<(i64, f64)> = Vec::new();
        let mut indexed_path_satisfied = !fts_results.is_empty();
        if fts_results.is_empty() {
            if let Some(trigram_query) =
                trigram_match_query(&query.query, MEMORY_SEARCH_MAX_LITERAL_CHARS)
            {
                let (literal_scope_clause, mut literal_scope_params) =
                    scope_where(query.scope.as_ref(), query.agent_id.as_deref());
                let literal_type_clause = type_filter_clause(query.types.as_deref());
                let literal_source_clause =
                    source_filter_clause_for("m.source", query.sources.as_deref());
                let sql = format!(
                    "SELECT fts.rowid, rank
                     FROM memories_literal_fts fts
                     JOIN memories m ON m.id = fts.rowid
                     WHERE memories_literal_fts MATCH ?
                       AND {} AND {} AND {}
                     ORDER BY rank LIMIT ?",
                    literal_scope_clause, literal_type_clause, literal_source_clause
                );
                if let Ok(mut stmt) = conn.prepare(&sql) {
                    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                        vec![Box::new(trigram_query)];
                    params.append(&mut literal_scope_params);
                    push_type_params(&mut params, query.types.as_deref());
                    push_source_params(&mut params, query.sources.as_deref());
                    params.push(Box::new(candidate_limit as i64));
                    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                        params.iter().map(|param| param.as_ref()).collect();
                    if let Ok(rows) = stmt.query_map(param_refs.as_slice(), |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                    }) {
                        indexed_path_satisfied = true;
                        literal_results.extend(rows.flatten());
                    }
                }
            }
        }
        if !indexed_path_satisfied {
            let Some(pattern) = literal_like_pattern(&query.query, MEMORY_SEARCH_MAX_LITERAL_CHARS)
            else {
                return Ok(Vec::new());
            };
            let (literal_scope_clause, mut literal_scope_params) =
                scope_where(query.scope.as_ref(), query.agent_id.as_deref());
            let literal_type_clause = type_filter_clause(query.types.as_deref());
            let literal_source_clause = source_filter_clause(query.sources.as_deref());
            let sql = format!(
                "SELECT id, 0.0 as rank
                 FROM memories
                 WHERE {} AND {} AND {}
                   AND (
                     lower(content) LIKE ? ESCAPE '\\'
                     OR lower(tags) LIKE ? ESCAPE '\\'
                     OR lower(source) LIKE ? ESCAPE '\\'
                     OR lower(coalesce(source_session_id, '')) LIKE ? ESCAPE '\\'
                   )
                 ORDER BY pinned DESC, updated_at DESC
                 LIMIT ?",
                literal_scope_clause, literal_type_clause, literal_source_clause
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            params.append(&mut literal_scope_params);
            push_type_params(&mut params, query.types.as_deref());
            push_source_params(&mut params, query.sources.as_deref());
            for _ in 0..4 {
                params.push(Box::new(pattern.clone()));
            }
            params.push(Box::new(candidate_limit as i64));
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?;
            for r in rows.flatten() {
                literal_results.push(r);
            }
        }

        // ── Step 2: Vector similarity search (if embedder available) ──
        let mut vec_results: Vec<(i64, f64)> = Vec::new(); // (id, distance)

        if let Some(ref emb) = query_embedding {
            let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            if let Some(signature) = active_signature.as_deref() {
                let (vec_scope_clause, mut vec_scope_params) =
                    scope_where(query.scope.as_ref(), query.agent_id.as_deref());
                let vec_type_clause = type_filter_clause(query.types.as_deref());
                let vec_source_clause =
                    source_filter_clause_for("m.source", query.sources.as_deref());
                let overfetch = candidate_limit.saturating_mul(8).min(2_000);
                let fast_sql = format!(
                    "WITH nearest AS (
                        SELECT rowid, distance FROM memories_vec
                        WHERE embedding MATCH ?
                        ORDER BY distance LIMIT ?
                     )
                     SELECT nearest.rowid, nearest.distance
                     FROM nearest
                     JOIN memories m ON m.id = nearest.rowid
                     WHERE m.embedding_signature = ? AND {} AND {} AND {}
                     ORDER BY nearest.distance LIMIT ?",
                    vec_scope_clause, vec_type_clause, vec_source_clause
                );
                if let Ok(mut stmt) = conn.prepare(&fast_sql) {
                    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
                        Box::new(emb_bytes.clone()),
                        Box::new(overfetch as i64),
                        Box::new(signature.to_string()),
                    ];
                    params.append(&mut vec_scope_params);
                    push_type_params(&mut params, query.types.as_deref());
                    push_source_params(&mut params, query.sources.as_deref());
                    params.push(Box::new(candidate_limit as i64));
                    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                        params.iter().map(|param| param.as_ref()).collect();
                    if let Ok(rows) = stmt.query_map(param_refs.as_slice(), |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                    }) {
                        vec_results.extend(rows.flatten());
                    }
                }

                // Rare scopes or source filters can be absent from the nearest
                // overfetch window. Preserve the old pre-filtered query as a
                // correctness fallback so the fast path never weakens scope
                // isolation or filtered recall.
                if vec_results.len() < limit.min(8) {
                    vec_results.clear();
                    let (_, mut safe_scope_params) =
                        scope_where(query.scope.as_ref(), query.agent_id.as_deref());
                    let safe_source_clause = source_filter_clause(query.sources.as_deref());
                    let safe_sql = format!(
                        "SELECT rowid, distance FROM memories_vec
                         WHERE embedding MATCH ?
                           AND rowid IN (
                               SELECT id FROM memories
                               WHERE embedding_signature = ? AND {} AND {} AND {}
                           )
                         ORDER BY distance LIMIT ?",
                        vec_scope_clause, vec_type_clause, safe_source_clause
                    );
                    if let Ok(mut stmt) = conn.prepare(&safe_sql) {
                        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
                            vec![Box::new(emb_bytes), Box::new(signature.to_string())];
                        params.append(&mut safe_scope_params);
                        push_type_params(&mut params, query.types.as_deref());
                        push_source_params(&mut params, query.sources.as_deref());
                        params.push(Box::new(candidate_limit as i64));
                        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                            params.iter().map(|param| param.as_ref()).collect();
                        if let Ok(rows) = stmt.query_map(param_refs.as_slice(), |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                        }) {
                            vec_results.extend(rows.flatten());
                        }
                    }
                }
            }
        }

        let lexical_ids = fts_results
            .iter()
            .chain(literal_results.iter())
            .map(|(id, _)| *id)
            .collect::<std::collections::HashSet<_>>();
        let semantic_similarity_by_id = vec_results
            .iter()
            .filter_map(|(id, distance)| {
                crate::memory::helpers::normalized_l2_to_cosine_similarity(*distance)
                    .map(|similarity| (*id, similarity))
            })
            .collect::<std::collections::HashMap<_, _>>();

        // ── Step 3: Weighted RRF (Reciprocal Rank Fusion) to merge results ──
        use std::collections::HashMap;
        let k = hybrid_cfg.rrf_k;

        let mut scores: HashMap<i64, f64> = HashMap::new();
        let (fts_weight, literal_weight) = crate::memory::helpers::adaptive_lexical_rrf_weights(
            hybrid_cfg.text_weight,
            hybrid_cfg.vector_weight,
            fts_results.len(),
            literal_results.len(),
            limit,
        );

        for (rank, (id, _)) in fts_results.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) += fts_weight / (k + rank as f64 + 1.0);
        }

        if literal_weight > 0.0 {
            for (rank, (id, _)) in literal_results.iter().enumerate() {
                *scores.entry(*id).or_insert(0.0) += literal_weight / (k + rank as f64 + 1.0);
            }
        }

        if has_vec {
            for (rank, (id, _)) in vec_results.iter().enumerate() {
                *scores.entry(*id).or_insert(0.0) +=
                    hybrid_cfg.vector_weight as f64 / (k + rank as f64 + 1.0);
            }
        }

        // ── Step 3b: Apply temporal decay ──
        if decay_cfg.enabled && decay_cfg.half_life_days > 0.0 {
            let lambda = (2.0_f64).ln() / decay_cfg.half_life_days;
            let now = chrono::Utc::now();
            // Need to load updated_at for scored entries to apply decay
            let ids: Vec<i64> = scores.keys().cloned().collect();
            if !ids.is_empty() {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT id, updated_at, pinned FROM memories WHERE id IN ({})",
                    placeholders
                );
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
                    .iter()
                    .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?;
                for r in rows.flatten() {
                    let (id, updated_at, pinned) = r;
                    if pinned {
                        continue;
                    } // Pinned memories are evergreen
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&updated_at) {
                        let age_days =
                            (now - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86400.0;
                        if age_days > 0.0 {
                            if let Some(score) = scores.get_mut(&id) {
                                *score *= (-lambda * age_days).exp();
                            }
                        }
                    }
                }
            }
        }

        // Sort by fused score (descending)
        let mut scored_ids: Vec<(i64, f64)> = scores.into_iter().collect();
        scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_ids.truncate(limit);

        if scored_ids.is_empty() {
            return Ok(Vec::new());
        }

        // ── Step 4: Load full entries for top results ──
        let id_list: Vec<String> = scored_ids.iter().map(|(id, _)| id.to_string()).collect();
        let placeholders = id_list.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // Apply scope and type filters
        let (scope_clause, mut scope_params) =
            scope_where(query.scope.as_ref(), query.agent_id.as_deref());
        let type_clause = type_filter_clause(query.types.as_deref());
        let source_clause = source_filter_clause(query.sources.as_deref());

        let sql = format!(
            "SELECT id, memory_type, scope_type, scope_agent_id, scope_project_id, content, tags,
                    source, source_session_id, pinned, created_at, updated_at,
                    attachment_path, attachment_mime
             FROM memories
             WHERE id IN ({}) AND {} AND {} AND {}",
            placeholders, scope_clause, type_clause, source_clause
        );

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for (id, _) in &scored_ids {
            all_params.push(Box::new(*id));
        }
        all_params.append(&mut scope_params);
        push_type_params(&mut all_params, query.types.as_deref());
        push_source_params(&mut all_params, query.sources.as_deref());

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;

        let score_map: HashMap<i64, f64> = scored_ids.into_iter().collect();
        let mut entries: Vec<MemoryEntry> = stmt
            .query_map(param_refs.as_slice(), row_to_entry)?
            .filter_map(|r| r.ok())
            .map(|mut e| {
                e.relevance_score = score_map.get(&e.id).map(|s| *s as f32);
                e.retrieval_evidence = Some(crate::memory::MemoryRetrievalEvidence {
                    lexical_match: lexical_ids.contains(&e.id),
                    semantic_similarity: semantic_similarity_by_id.get(&e.id).copied(),
                });
                e
            })
            .collect();

        // Sort by relevance score (descending)
        entries.sort_by(|a, b| {
            b.relevance_score
                .unwrap_or(0.0)
                .partial_cmp(&a.relevance_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── Step 5: MMR diversity reranking ──
        let mmr_cfg = crate::memory::helpers::load_mmr_config();
        if mmr_cfg.enabled && entries.len() > 1 {
            let candidates: Vec<(i64, f32, &str)> = entries
                .iter()
                .map(|e| (e.id, e.relevance_score.unwrap_or(0.0), e.content.as_str()))
                .collect();
            let reranked = crate::memory::mmr::mmr_rerank(&candidates, limit, mmr_cfg.lambda);

            // Rebuild entries in MMR order
            let id_order: Vec<i64> = reranked.iter().map(|(id, _)| *id).collect();
            let entry_map: HashMap<i64, MemoryEntry> =
                entries.into_iter().map(|e| (e.id, e)).collect();
            entries = id_order
                .into_iter()
                .filter_map(|id| entry_map.get(&id).cloned())
                .collect();
        }

        Ok(entries)
    }

    fn count(&self, scope: Option<&MemoryScope>) -> Result<usize> {
        self.count_filtered(scope, None)
    }

    fn count_filtered(
        &self,
        scope: Option<&MemoryScope>,
        sources: Option<&[String]>,
    ) -> Result<usize> {
        let conn = self.read_conn()?;
        let (scope_clause, mut scope_params) = scope_where(scope, None);
        let source_clause = source_filter_clause(sources);

        let sql = format!(
            "SELECT COUNT(*) FROM memories WHERE {} AND {}",
            scope_clause, source_clause
        );
        push_source_params(&mut scope_params, sources);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            scope_params.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    fn build_prompt_summary(&self, agent_id: &str, shared: bool, budget: usize) -> Result<String> {
        // Delegate to the project-aware variant with `project_id = None` so
        // the two code paths share the same ordering / filtering logic.
        self.build_prompt_summary_with_project(agent_id, None, shared, budget)
    }

    fn build_prompt_summary_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        shared: bool,
        budget: usize,
    ) -> Result<String> {
        let candidates = self.load_prompt_candidates_with_project(agent_id, project_id, shared)?;
        Ok(format_prompt_summary(&candidates, budget))
    }

    /// Load candidate memories for prompt injection.
    /// Returns agent-scoped + optionally global memories, ordered by updated_at DESC.
    /// Used directly by `build_prompt_summary` and by LLM memory selection.
    fn load_prompt_candidates(&self, agent_id: &str, shared: bool) -> Result<Vec<MemoryEntry>> {
        self.load_prompt_candidates_with_project(agent_id, None, shared)
    }

    fn load_prompt_candidates_with_project(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        shared: bool,
    ) -> Result<Vec<MemoryEntry>> {
        let mut all_memories = Vec::new();

        // Project-scoped memories first — highest priority when a project
        // context exists. This ensures budget-based truncation keeps them.
        if let Some(pid) = project_id {
            let project_scope = MemoryScope::Project {
                id: pid.to_string(),
            };
            let project_mems = self.list(Some(&project_scope), None, 200, 0)?;
            all_memories.extend(project_mems);
        }

        // Agent-scoped memories
        let agent_scope = MemoryScope::Agent {
            id: agent_id.to_string(),
        };
        let agent_mems = self.list(Some(&agent_scope), None, 200, 0)?;
        all_memories.extend(agent_mems);

        // Global memories (if shared)
        if shared {
            let global_mems = self.list(Some(&MemoryScope::Global), None, 200, 0)?;
            all_memories.extend(global_mems);
        }

        // Claim-layer effective-status filter (design §4.5): drop any legacy
        // memory whose managing claim is no longer injectable (superseded /
        // expired / archived / needs_review), so a stale claim can't keep
        // re-injecting its shadow. `user_pinned` links are exempt (kept; the
        // review-queue surfacing is handled elsewhere). Empty/cheap when no
        // claims exist (the dual-track default).
        let read_guard = self.read_conn()?;
        let hidden = hidden_claim_linked_memory_ids(&read_guard)?;
        // Single-source dedup (design §4.8): a legacy memory covered by an
        // effective-active managed claim injects via the Pinned / Relevant
        // Claims segments, so drop it from this legacy section to avoid
        // double-injecting the same fact (and double-spending the budget).
        // `user_pinned` links and directly-pinned memories are exempt — an
        // explicit keep signal outranks the dedup (mirrors the `hidden` query).
        // Shares the read_guard with `hidden` to avoid a second reader acquire.
        let covered = covered_by_active_claim_memory_ids(&read_guard)?;
        drop(read_guard);
        if !hidden.is_empty() {
            all_memories.retain(|m| !hidden.contains(&m.id));
        }
        if !covered.is_empty() {
            all_memories.retain(|m| !covered.contains(&m.id));
        }

        Ok(all_memories)
    }

    fn count_by_project(&self, project_id: &str) -> Result<usize> {
        self.count(Some(&MemoryScope::Project {
            id: project_id.to_string(),
        }))
    }

    fn export_markdown(&self, scope: Option<&MemoryScope>) -> Result<String> {
        let entries = self.list(scope, None, 10000, 0)?;

        if entries.is_empty() {
            return Ok("# Memories\n\nNo memories stored.\n".to_string());
        }

        let mut md = "# Memories\n\n".to_string();

        let type_order = [
            MemoryType::User,
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
        ];

        for mem_type in &type_order {
            let type_entries: Vec<&MemoryEntry> = entries
                .iter()
                .filter(|m| &m.memory_type == mem_type)
                .collect();

            if type_entries.is_empty() {
                continue;
            }

            md.push_str(&format!("## {}\n\n", mem_type.heading()));

            for entry in type_entries {
                md.push_str(&format!(
                    "### {}\n",
                    entry.content.lines().next().unwrap_or("Untitled")
                ));
                if !entry.tags.is_empty() {
                    md.push_str(&format!("Tags: {}\n", entry.tags.join(", ")));
                }
                let scope_label = match &entry.scope {
                    MemoryScope::Global => "global".to_string(),
                    MemoryScope::Agent { id } => format!("agent:{}", id),
                    MemoryScope::Project { id } => format!("project:{}", id),
                };
                md.push_str(&format!(
                    "Scope: {} | Source: {} | Updated: {}\n\n",
                    scope_label, entry.source, entry.updated_at
                ));
                md.push_str(&entry.content);
                md.push_str("\n\n---\n\n");
            }
        }

        Ok(md)
    }

    fn stats(&self, scope: Option<&MemoryScope>) -> Result<MemoryStats> {
        let conn = self.read_conn()?;
        let (scope_clause, scope_params) = scope_where(scope, None);

        // Total count
        let total: usize = conn.query_row(
            &format!("SELECT COUNT(*) FROM memories WHERE {}", scope_clause),
            rusqlite::params_from_iter(scope_params.iter()),
            |row| row.get::<_, i64>(0),
        )? as usize;

        // Count by type
        let mut by_type = std::collections::HashMap::new();
        {
            let (sc, sp) = scope_where(scope, None);
            let mut stmt = conn.prepare(&format!(
                "SELECT memory_type, COUNT(*) FROM memories WHERE {} GROUP BY memory_type",
                sc
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(sp.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })?;
            for row in rows {
                let (t, c) = row?;
                by_type.insert(t, c);
            }
        }

        // Count by source
        let mut by_source = std::collections::HashMap::new();
        {
            let (sc, sp) = scope_where(scope, None);
            let mut stmt = conn.prepare(&format!(
                "SELECT source, COUNT(*) FROM memories WHERE {} GROUP BY source",
                sc
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(sp.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
            })?;
            for row in rows {
                let (source, count) = row?;
                by_source.insert(source, count);
            }
        }

        // Count with embedding
        let with_embedding: usize = if let Some(signature) =
            crate::memory::helpers::active_embedding_signature()
        {
            let (sc, mut sp) = scope_where(scope, None);
            sp.push(Box::new(signature));
            conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM memories WHERE {} AND embedding_signature = ? AND id IN (SELECT rowid FROM memories_vec)",
                        sc
                    ),
                    rusqlite::params_from_iter(sp.iter()),
                    |row| row.get::<_, i64>(0).map(|v| v as usize),
                )
                .unwrap_or(0)
        } else {
            0
        };

        // Oldest and newest
        let (oldest, newest) = {
            let (sc, sp) = scope_where(scope, None);
            let oldest: Option<String> = conn
                .query_row(
                    &format!("SELECT MIN(created_at) FROM memories WHERE {}", sc),
                    rusqlite::params_from_iter(sp.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            let (sc2, sp2) = scope_where(scope, None);
            let newest: Option<String> = conn
                .query_row(
                    &format!("SELECT MAX(created_at) FROM memories WHERE {}", sc2),
                    rusqlite::params_from_iter(sp2.iter()),
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            (oldest, newest)
        };

        Ok(MemoryStats {
            total,
            by_type,
            by_source,
            with_embedding,
            oldest,
            newest,
        })
    }

    fn health(&self) -> Result<MemoryHealth> {
        let stats = self.stats(None)?;
        let conn = self.read_conn()?;
        let mut health = MemoryHealth::new(self.backend_kind(), &stats);

        health.quick_check = conn
            .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
            .unwrap_or_else(|e| format!("error: {e}"));
        if health.quick_check != "ok" {
            health.add_issue(
                "db_quick_check_failed",
                MemoryHealthSeverity::Error,
                format!("SQLite quick_check returned {}", health.quick_check),
                Some("Back up memory.db, then run database repair or restore from backup.".into()),
            );
        }

        health.active_embedding_signature = crate::memory::helpers::active_embedding_signature();
        health.embedding_provider_configured = health.active_embedding_signature.is_some();
        {
            let embedder_guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
            if let Some(provider) = embedder_guard.as_ref() {
                health.embedding_provider_loaded = true;
                health.embedding_provider_dimensions = Some(provider.dimensions());
                health.embedding_provider_multimodal = provider.supports_multimodal();
                health.embedding_provider_batch = provider.supports_batch_api();
            }
        }
        if let Some(signature) = health.active_embedding_signature.clone() {
            if !health.embedding_provider_loaded {
                health.add_issue(
                    "embedding_provider_unavailable",
                    MemoryHealthSeverity::Warning,
                    "Memory embedding is configured, but the runtime provider is not loaded.",
                    Some(
                        "Open Memory settings and re-save or switch the active embedding model."
                            .into(),
                    ),
                );
            }
            health.memories_pending_embedding =
                self.count_memories_pending_embedding(&signature)? as usize;
            if health.memories_pending_embedding > 0 {
                health.add_issue(
                    "memory_reembed_needed",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "{} memories need embeddings for the active model",
                        health.memories_pending_embedding
                    ),
                    Some("Run memory re-embed from Memory settings.".into()),
                );
            }
        }

        let memories_vec_exists = sqlite_table_exists(&conn, "memories_vec")?;
        health.vector_rows = memories_vec_exists
            .then(|| sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memories_vec"));
        if health.active_embedding_signature.is_some()
            && health.total_memories > 0
            && !memories_vec_exists
        {
            health.add_issue(
                "memory_vector_index_missing",
                MemoryHealthSeverity::Warning,
                "Memory embedding is enabled, but the vector index is missing.",
                Some("Run memory re-embed to recreate the vector index.".into()),
            );
        }

        if sqlite_table_exists(&conn, "memories_fts")? {
            health.fts_rows = sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memories_fts");
            health.fts_missing_rows = sqlite_count_or_zero(
                &conn,
                "SELECT COUNT(*) FROM memories m
                 WHERE NOT EXISTS (SELECT 1 FROM memories_fts f WHERE f.rowid = m.id)",
            );
            if health.fts_missing_rows > 0 {
                health.add_issue(
                    "memory_fts_missing_rows",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "{} memories are missing from the keyword index",
                        health.fts_missing_rows
                    ),
                    Some("Export a backup, then rebuild the memory index.".into()),
                );
            }
        } else {
            health.fts_missing_rows = health.total_memories;
            health.add_issue(
                "memory_fts_missing",
                MemoryHealthSeverity::Error,
                "Memory keyword index table is missing.",
                Some("Export a backup, then rebuild or restore memory.db.".into()),
            );
        }
        let literal_fts_missing_rows = if sqlite_table_exists(&conn, "memories_literal_fts")? {
            sqlite_count_or_zero(
                &conn,
                "SELECT COUNT(*) FROM memories m
                 WHERE NOT EXISTS (
                     SELECT 1 FROM memories_literal_fts f WHERE f.rowid = m.id
                 )",
            )
        } else {
            health.total_memories
        };
        if literal_fts_missing_rows > 0 {
            health.add_issue(
                "memory_literal_fts_missing_rows",
                MemoryHealthSeverity::Warning,
                format!("{literal_fts_missing_rows} memories are missing from the substring index"),
                Some("Export a backup, then rebuild the memory index.".into()),
            );
        }

        health.claims_total = sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_claims");
        health.claims_needs_review = sqlite_count_or_zero(
            &conn,
            "SELECT COUNT(*) FROM memory_claims WHERE status = 'needs_review'",
        );
        health.claims_without_evidence = sqlite_count_or_zero(
            &conn,
            "SELECT COUNT(*) FROM memory_claims c
             WHERE NOT EXISTS (SELECT 1 FROM memory_evidence e WHERE e.claim_id = c.id)",
        );
        if sqlite_table_exists(&conn, "memory_claims_fts")? {
            health.claim_fts_rows =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_claims_fts");
            health.claim_fts_missing_rows = sqlite_count_or_zero(
                &conn,
                "SELECT COUNT(*) FROM memory_claims c
                 WHERE NOT EXISTS (SELECT 1 FROM memory_claims_fts f WHERE f.rowid = c.rowid)",
            );
            if health.claim_fts_missing_rows > 0 {
                health.add_issue(
                    "claim_fts_missing_rows",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "{} structured memories are missing from the keyword index",
                        health.claim_fts_missing_rows
                    ),
                    Some("Export a backup, then rebuild the structured memory index.".into()),
                );
            }
        } else {
            health.claim_fts_missing_rows = health.claims_total;
            health.add_issue(
                "claim_fts_missing",
                MemoryHealthSeverity::Error,
                "Structured memory keyword index table is missing.",
                Some("Export a backup, then rebuild the structured memory index.".into()),
            );
        }
        let claim_literal_fts_missing_rows =
            if sqlite_table_exists(&conn, "memory_claims_literal_fts")? {
                sqlite_count_or_zero(
                    &conn,
                    "SELECT COUNT(*) FROM memory_claims c
                     WHERE NOT EXISTS (
                         SELECT 1 FROM memory_claims_literal_fts f WHERE f.rowid = c.rowid
                     )",
                )
            } else {
                health.claims_total
            };
        if claim_literal_fts_missing_rows > 0 {
            health.add_issue(
                "claim_literal_fts_missing_rows",
                MemoryHealthSeverity::Warning,
                format!(
                    "{claim_literal_fts_missing_rows} structured memories are missing from the substring index"
                ),
                Some("Export a backup, then rebuild the structured memory index.".into()),
            );
        }
        if sqlite_table_exists(&conn, "memory_evidence_fts")? {
            health.evidence_fts_rows =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_evidence_fts");
            health.evidence_fts_missing_rows = sqlite_count_or_zero(
                &conn,
                "SELECT COUNT(*) FROM memory_evidence e
                 WHERE NOT EXISTS (SELECT 1 FROM memory_evidence_fts f WHERE f.rowid = e.rowid)",
            );
            if health.evidence_fts_missing_rows > 0 {
                health.add_issue(
                    "evidence_fts_missing_rows",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "{} evidence rows are missing from the structured keyword index",
                        health.evidence_fts_missing_rows
                    ),
                    Some("Export a backup, then rebuild the structured memory index.".into()),
                );
            }
        } else {
            health.evidence_fts_missing_rows =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_evidence");
            health.add_issue(
                "evidence_fts_missing",
                MemoryHealthSeverity::Error,
                "Structured memory evidence keyword index table is missing.",
                Some("Export a backup, then rebuild the structured memory index.".into()),
            );
        }
        if health.claims_without_evidence > 0 {
            health.add_issue(
                "claims_without_evidence",
                MemoryHealthSeverity::Warning,
                format!(
                    "{} structured memories have no evidence rows",
                    health.claims_without_evidence
                ),
                Some("Review these structured memories before trusting them in prompts.".into()),
            );
        }

        health.orphan_evidence_rows = sqlite_count_or_zero(
            &conn,
            "SELECT COUNT(*) FROM memory_evidence e
             WHERE NOT EXISTS (SELECT 1 FROM memory_claims c WHERE c.id = e.claim_id)",
        );
        health.orphan_claim_links = sqlite_count_or_zero(
            &conn,
            "SELECT COUNT(*) FROM memory_claim_links l
             WHERE NOT EXISTS (SELECT 1 FROM memory_claims c WHERE c.id = l.claim_id)
                OR NOT EXISTS (SELECT 1 FROM memories m WHERE m.id = l.memory_id)",
        );
        if health.orphan_evidence_rows > 0 || health.orphan_claim_links > 0 {
            health.add_issue(
                "orphan_claim_graph_rows",
                MemoryHealthSeverity::Warning,
                format!(
                    "{} orphan evidence rows and {} orphan claim links found",
                    health.orphan_evidence_rows, health.orphan_claim_links
                ),
                Some("Export a backup, then repair claim graph links.".into()),
            );
        }

        let episodes_table_exists = sqlite_table_exists(&conn, "memory_episodes")?;
        let procedures_table_exists = sqlite_table_exists(&conn, "memory_procedures")?;
        if episodes_table_exists {
            health.episodes_total =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_episodes");
        }
        if procedures_table_exists {
            health.procedures_total =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM memory_procedures");
        }
        if episodes_table_exists && procedures_table_exists {
            health.orphan_procedure_episode_refs =
                sqlite_count_orphan_procedure_episode_refs(&conn);
            if health.orphan_procedure_episode_refs > 0 {
                health.add_issue(
                    "orphan_procedure_episode_refs",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "{} procedure source episode reference(s) point to missing episodes",
                        health.orphan_procedure_episode_refs
                    ),
                    Some(
                        "Restore the missing episode backup or review and recreate the affected procedures."
                            .into(),
                    ),
                );
            }
        } else {
            health.add_issue(
                "memory_experience_tables_missing",
                MemoryHealthSeverity::Warning,
                "Episode / procedure memory tables are missing.",
                Some(
                    "Restart the app to run memory migrations, then export a fresh backup.".into(),
                ),
            );
        }

        if health.claims_needs_review > 0 {
            health.add_issue(
                "claims_need_review",
                MemoryHealthSeverity::Info,
                format!(
                    "{} structured memories are waiting for review",
                    health.claims_needs_review
                ),
                Some("Open Memory Inbox and approve, edit, or archive them.".into()),
            );
        }

        let now = crate::util::now_rfc3339();
        let app_cfg = crate::config::cached_config();
        let resolver_claims_result = if sqlite_table_exists(&conn, "memory_claims")? {
            list_active_claims_for_resolver_health(&conn).map_err(|e| e.to_string())
        } else {
            Ok(Vec::new())
        };
        let resolver_preflight = crate::memory::dreaming::resolver_preflight_from_claims(
            &app_cfg.dreaming,
            app_cfg
                .memory
                .effective_enabled(app_cfg.memory_extract.enabled),
            resolver_claims_result,
            &now,
        );
        health.deep_resolver_active_claims = resolver_preflight.active_claim_count;
        health.deep_resolver_expired_candidates = resolver_preflight.expired_candidate_count;
        health.deep_resolver_conflict_groups = resolver_preflight.conflict_group_count;
        health.deep_resolver_groups_to_analyze = resolver_preflight.groups_to_analyze;
        health.deep_resolver_group_cap = resolver_preflight.group_cap;
        health.deep_resolver_truncated = resolver_preflight.truncated;
        health.deep_resolver_would_call_llm = resolver_preflight.would_call_llm;
        health.deep_resolver_blocking_reasons = resolver_preflight
            .blocking_reasons
            .iter()
            .map(|reason| reason.as_str().to_string())
            .collect();
        if resolver_preflight.load_error.is_some() {
            health.add_issue(
                "deep_resolver_preflight_unavailable",
                MemoryHealthSeverity::Warning,
                "Deep Resolver preflight could not load active structured memories.",
                Some(
                    "Retry Memory Health; if it persists, inspect memory.db and claim tables."
                        .into(),
                ),
            );
        } else if resolver_preflight.can_run_manual
            && (resolver_preflight.expired_candidate_count > 0
                || resolver_preflight.conflict_group_count > 0)
        {
            health.add_issue(
                "deep_resolver_backlog",
                MemoryHealthSeverity::Info,
                format!(
                    "{} expired structured memor{} and {} conflict candidate group(s) are waiting for Deep Resolver.",
                    resolver_preflight.expired_candidate_count,
                    if resolver_preflight.expired_candidate_count == 1 { "y is" } else { "ies are" },
                    resolver_preflight.conflict_group_count
                ),
                Some("Open Dashboard → Dreaming and review the Deep Resolver preflight before running it.".into()),
            );
        }

        if sqlite_table_exists(&conn, "dreaming_runs")? {
            health.dreaming_running_runs = sqlite_count_or_zero(
                &conn,
                "SELECT COUNT(*) FROM dreaming_runs WHERE status = 'running'",
            );
            health.dreaming_stale_runs = sqlite_count_with_text_param_or_zero(
                &conn,
                "SELECT COUNT(*) FROM dreaming_runs
                 WHERE status = 'running'
                   AND (lease_expires_at IS NULL OR lease_expires_at < ?1)",
                &now,
            );
        }
        if sqlite_table_exists(&conn, "dreaming_locks")? {
            health.dreaming_locks =
                sqlite_count_or_zero(&conn, "SELECT COUNT(*) FROM dreaming_locks");
            health.dreaming_stale_locks = sqlite_count_with_text_param_or_zero(
                &conn,
                "SELECT COUNT(*) FROM dreaming_locks WHERE lease_expires_at < ?1",
                &now,
            );
        }
        if health.dreaming_stale_runs > 0 || health.dreaming_stale_locks > 0 {
            health.add_issue(
                "dreaming_state_stale",
                MemoryHealthSeverity::Warning,
                format!(
                    "{} stale Dreaming run(s) and {} expired lock(s) found",
                    health.dreaming_stale_runs, health.dreaming_stale_locks
                ),
                Some("Recover Dreaming maintenance state so background memory consolidation can resume cleanly.".into()),
            );
        }

        health.latest_db_snapshot = self.latest_db_snapshot().unwrap_or(None);
        if let Some(snapshot) = health.latest_db_snapshot.as_ref() {
            if snapshot.status != MemoryDbSnapshotStatus::Ok {
                health.add_issue(
                    "db_snapshot_incomplete",
                    MemoryHealthSeverity::Warning,
                    format!(
                        "Latest DB safety snapshot is not fully verifiable ({})",
                        snapshot.status.as_str()
                    ),
                    Some(
                        "Create a fresh database snapshot before attempting manual recovery."
                            .into(),
                    ),
                );
            }
        }

        let external_provider_config = crate::config::cached_config().memory_providers.clone();
        health.external_providers_enabled = external_provider_config.enabled;
        health.external_provider_count = external_provider_config.providers.len();
        health.external_provider_active_count = external_provider_config
            .providers
            .iter()
            .filter(|provider| {
                external_provider_config.enabled
                    && provider.enabled
                    && provider.sync_policy.is_active()
            })
            .count();
        health.external_providers = external_provider_config
            .providers
            .iter()
            .map(|provider| {
                ExternalMemoryProviderHealth::from_config(
                    provider,
                    external_provider_config.enabled,
                )
            })
            .collect();
        if external_provider_config.enabled {
            let external_provider_health = health.external_providers.clone();
            for provider in &external_provider_health {
                if provider.enabled
                    && provider.capabilities.requires_endpoint
                    && !provider.endpoint_configured
                {
                    health.add_issue(
                        "external_memory_provider_incomplete",
                        MemoryHealthSeverity::Warning,
                        format!(
                            "External memory provider '{}' is enabled but not fully configured.",
                            provider.display_name
                        ),
                        Some(
                            "Finish provider setup or disable it; local memory remains available."
                                .into(),
                        ),
                    );
                }
                if provider.enabled && !provider.capabilities.adapter_available {
                    health.add_issue(
                        "external_memory_provider_adapter_unavailable",
                        MemoryHealthSeverity::Warning,
                        format!(
                            "External memory provider '{}' is configured, but its runtime adapter is not available yet.",
                            provider.display_name
                        ),
                        Some(
                            "Local memory remains the source of truth; disable this provider until a concrete adapter ships."
                                .into(),
                        ),
                    );
                }
                if provider.enabled && !provider.policy_supported {
                    health.add_issue(
                        "external_memory_provider_policy_unsupported",
                        MemoryHealthSeverity::Warning,
                        format!(
                            "External memory provider '{}' does not support the selected sync policy.",
                            provider.display_name
                        ),
                        Some(
                            "Choose a supported sync policy before enabling external provider sync."
                                .into(),
                        ),
                    );
                }
                if provider.enabled {
                    if let Some(error) = provider.last_error.as_ref() {
                        health.add_issue(
                            "external_memory_provider_error",
                            MemoryHealthSeverity::Warning,
                            format!(
                                "External memory provider '{}' reported an error: {}",
                                provider.display_name, error
                            ),
                            Some(
                                "Review provider credentials and sync settings; local memory remains available."
                                    .into(),
                            ),
                        );
                    }
                }
            }
        }

        health.refresh_status();
        Ok(health)
    }

    fn repair(&self, action: MemoryRepairAction) -> Result<MemoryRepairReport> {
        let before = self.health()?;
        let mut artifact_path = None;
        let mut artifact_files = Vec::new();
        match action {
            MemoryRepairAction::RebuildFts => {
                let conn = self.write_conn()?;
                conn.execute_batch(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                        content, tags,
                        content='memories',
                        content_rowid='id',
                        tokenize='unicode61'
                    );

                    CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                        INSERT INTO memories_fts(rowid, content, tags)
                        VALUES (new.id, new.content, new.tags);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                        INSERT INTO memories_fts(memories_fts, rowid, content, tags)
                        VALUES ('delete', old.id, old.content, old.tags);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                        INSERT INTO memories_fts(memories_fts, rowid, content, tags)
                        VALUES ('delete', old.id, old.content, old.tags);
                        INSERT INTO memories_fts(rowid, content, tags)
                        VALUES (new.id, new.content, new.tags);
                    END;

                    INSERT INTO memories_fts(memories_fts) VALUES('rebuild');

                    CREATE VIRTUAL TABLE IF NOT EXISTS memories_literal_fts USING fts5(
                        content, tags, source, source_session_id,
                        content='memories',
                        content_rowid='id',
                        tokenize='trigram'
                    );

                    CREATE TRIGGER IF NOT EXISTS memories_literal_ai AFTER INSERT ON memories BEGIN
                        INSERT INTO memories_literal_fts(rowid, content, tags, source, source_session_id)
                        VALUES (new.id, new.content, new.tags, new.source, new.source_session_id);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memories_literal_ad AFTER DELETE ON memories BEGIN
                        INSERT INTO memories_literal_fts(memories_literal_fts, rowid, content, tags, source, source_session_id)
                        VALUES ('delete', old.id, old.content, old.tags, old.source, old.source_session_id);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memories_literal_au AFTER UPDATE ON memories BEGIN
                        INSERT INTO memories_literal_fts(memories_literal_fts, rowid, content, tags, source, source_session_id)
                        VALUES ('delete', old.id, old.content, old.tags, old.source, old.source_session_id);
                        INSERT INTO memories_literal_fts(rowid, content, tags, source, source_session_id)
                        VALUES (new.id, new.content, new.tags, new.source, new.source_session_id);
                    END;

                    INSERT INTO memories_literal_fts(memories_literal_fts) VALUES('rebuild');",
                )?;
            }
            MemoryRepairAction::RebuildClaimFts => {
                let conn = self.write_conn()?;
                conn.execute_batch(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS memory_claims_fts USING fts5(
                        content, subject, object,
                        content='memory_claims',
                        content_rowid='rowid',
                        tokenize='unicode61'
                    );

                    CREATE TRIGGER IF NOT EXISTS memory_claims_ai AFTER INSERT ON memory_claims BEGIN
                        INSERT INTO memory_claims_fts(rowid, content, subject, object)
                        VALUES (new.rowid, new.content, new.subject, new.object);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_claims_ad AFTER DELETE ON memory_claims BEGIN
                        INSERT INTO memory_claims_fts(memory_claims_fts, rowid, content, subject, object)
                        VALUES ('delete', old.rowid, old.content, old.subject, old.object);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_claims_au AFTER UPDATE ON memory_claims BEGIN
                        INSERT INTO memory_claims_fts(memory_claims_fts, rowid, content, subject, object)
                        VALUES ('delete', old.rowid, old.content, old.subject, old.object);
                        INSERT INTO memory_claims_fts(rowid, content, subject, object)
                        VALUES (new.rowid, new.content, new.subject, new.object);
                    END;

                    INSERT INTO memory_claims_fts(memory_claims_fts) VALUES('rebuild');

                    CREATE VIRTUAL TABLE IF NOT EXISTS memory_claims_literal_fts USING fts5(
                        content, claim_type, subject, predicate, object, tags_json,
                        content='memory_claims',
                        content_rowid='rowid',
                        tokenize='trigram'
                    );

                    CREATE TRIGGER IF NOT EXISTS memory_claims_literal_ai AFTER INSERT ON memory_claims BEGIN
                        INSERT INTO memory_claims_literal_fts(rowid, content, claim_type, subject, predicate, object, tags_json)
                        VALUES (new.rowid, new.content, new.claim_type, new.subject, new.predicate, new.object, new.tags_json);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_claims_literal_ad AFTER DELETE ON memory_claims BEGIN
                        INSERT INTO memory_claims_literal_fts(memory_claims_literal_fts, rowid, content, claim_type, subject, predicate, object, tags_json)
                        VALUES ('delete', old.rowid, old.content, old.claim_type, old.subject, old.predicate, old.object, old.tags_json);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_claims_literal_au AFTER UPDATE ON memory_claims BEGIN
                        INSERT INTO memory_claims_literal_fts(memory_claims_literal_fts, rowid, content, claim_type, subject, predicate, object, tags_json)
                        VALUES ('delete', old.rowid, old.content, old.claim_type, old.subject, old.predicate, old.object, old.tags_json);
                        INSERT INTO memory_claims_literal_fts(rowid, content, claim_type, subject, predicate, object, tags_json)
                        VALUES (new.rowid, new.content, new.claim_type, new.subject, new.predicate, new.object, new.tags_json);
                    END;

                    INSERT INTO memory_claims_literal_fts(memory_claims_literal_fts) VALUES('rebuild');

                    CREATE VIRTUAL TABLE IF NOT EXISTS memory_evidence_fts USING fts5(
                        source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote,
                        content='memory_evidence',
                        content_rowid='rowid',
                        tokenize='unicode61'
                    );

                    CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_ai AFTER INSERT ON memory_evidence BEGIN
                        INSERT INTO memory_evidence_fts(rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                        VALUES (new.rowid, new.source_type, new.evidence_class, new.source_id, new.session_id, new.message_id, new.file_path, new.url, new.quote);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_ad AFTER DELETE ON memory_evidence BEGIN
                        INSERT INTO memory_evidence_fts(memory_evidence_fts, rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                        VALUES ('delete', old.rowid, old.source_type, old.evidence_class, old.source_id, old.session_id, old.message_id, old.file_path, old.url, old.quote);
                    END;

                    CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_au AFTER UPDATE ON memory_evidence BEGIN
                        INSERT INTO memory_evidence_fts(memory_evidence_fts, rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                        VALUES ('delete', old.rowid, old.source_type, old.evidence_class, old.source_id, old.session_id, old.message_id, old.file_path, old.url, old.quote);
                        INSERT INTO memory_evidence_fts(rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                        VALUES (new.rowid, new.source_type, new.evidence_class, new.source_id, new.session_id, new.message_id, new.file_path, new.url, new.quote);
                    END;

                    INSERT INTO memory_evidence_fts(memory_evidence_fts) VALUES('rebuild');",
                )?;
            }
            MemoryRepairAction::RepairClaimGraph => {
                let conn = self.write_conn()?;
                conn.execute_batch(
                    "DELETE FROM memory_evidence
                     WHERE NOT EXISTS (
                         SELECT 1 FROM memory_claims c
                         WHERE c.id = memory_evidence.claim_id
                     );
                     DELETE FROM memory_claim_links
                     WHERE NOT EXISTS (
                         SELECT 1 FROM memory_claims c
                         WHERE c.id = memory_claim_links.claim_id
                     )
                        OR NOT EXISTS (
                         SELECT 1 FROM memories m
                         WHERE m.id = memory_claim_links.memory_id
                     );",
                )?;
            }
            MemoryRepairAction::RepairExperienceGraph => {
                let conn = self.write_conn()?;
                sqlite_repair_orphan_procedure_episode_refs(&conn)?;
            }
            MemoryRepairAction::RecoverDreamingState => {
                let conn = self.write_conn()?;
                let now = crate::util::now_rfc3339();
                conn.execute(
                    "UPDATE dreaming_runs
                     SET status = 'failed',
                         finished_at = ?1,
                         note = COALESCE(note, 'interrupted before completion')
                     WHERE status = 'running'
                       AND (lease_expires_at IS NULL OR lease_expires_at < ?1)",
                    params![now],
                )?;
                conn.execute(
                    "DELETE FROM dreaming_locks WHERE lease_expires_at < ?1",
                    params![now],
                )?;
            }
            MemoryRepairAction::CreateDbSnapshot => {
                let (path, files) = self.create_db_snapshot(&before)?;
                artifact_path = Some(path);
                artifact_files = files;
            }
        }
        let after = self.health()?;
        let health_changed = before.fts_missing_rows != after.fts_missing_rows
            || before.fts_rows != after.fts_rows
            || before.claim_fts_missing_rows != after.claim_fts_missing_rows
            || before.claim_fts_rows != after.claim_fts_rows
            || before.evidence_fts_missing_rows != after.evidence_fts_missing_rows
            || before.evidence_fts_rows != after.evidence_fts_rows
            || before.orphan_evidence_rows != after.orphan_evidence_rows
            || before.orphan_claim_links != after.orphan_claim_links
            || before.orphan_procedure_episode_refs != after.orphan_procedure_episode_refs
            || before.dreaming_stale_runs != after.dreaming_stale_runs
            || before.dreaming_stale_locks != after.dreaming_stale_locks
            || before
                .issues
                .iter()
                .any(|issue| issue.code == "memory_literal_fts_missing_rows")
                != after
                    .issues
                    .iter()
                    .any(|issue| issue.code == "memory_literal_fts_missing_rows")
            || before
                .issues
                .iter()
                .any(|issue| issue.code == "claim_literal_fts_missing_rows")
                != after
                    .issues
                    .iter()
                    .any(|issue| issue.code == "claim_literal_fts_missing_rows")
            || before.status != after.status;
        Ok(MemoryRepairReport {
            action,
            changed: health_changed || artifact_path.is_some(),
            artifact_path,
            artifact_files,
            before,
            after,
        })
    }

    fn db_snapshot_restore_preview(
        &self,
        snapshot_path: &str,
    ) -> Result<MemoryDbSnapshotRestorePreview> {
        self.db_snapshot_restore_preview_inner(snapshot_path)
    }

    fn db_snapshot_restore(&self, snapshot_path: &str) -> Result<MemoryDbSnapshotRestoreReport> {
        self.db_snapshot_restore_inner(snapshot_path)
    }

    fn find_similar(
        &self,
        content: &str,
        memory_type: Option<&MemoryType>,
        scope: Option<&MemoryScope>,
        threshold: f32,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        // Reuse search() to get candidates via FTS5 + vector hybrid
        let types = memory_type.map(|t| vec![t.clone()]);
        let query = MemorySearchQuery {
            query: content.to_string(),
            types,
            sources: None,
            scope: scope.cloned(),
            agent_id: None,
            limit: Some(limit * 3), // fetch extra to filter by threshold
        };
        let results = self.search(&query)?;

        // Filter by threshold
        Ok(results
            .into_iter()
            .filter(|e| e.relevance_score.unwrap_or(0.0) >= threshold)
            .take(limit)
            .collect())
    }

    fn add_with_dedup(
        &self,
        entry: NewMemory,
        threshold_high: f32,
        threshold_merge: f32,
    ) -> Result<AddResult> {
        // Find similar entries of the same type and scope
        let similar = self.find_similar(
            &entry.content,
            Some(&entry.memory_type),
            Some(&entry.scope),
            threshold_merge,
            5,
        )?;

        if let Some(best) = similar.first() {
            let score = best.relevance_score.unwrap_or(0.0);
            if score >= threshold_high {
                // Very similar — treat as duplicate, skip
                return Ok(AddResult::Duplicate {
                    existing_id: best.id,
                    score,
                });
            }
            // Moderately similar — update existing entry by appending new content
            let merged_content = format!("{}\n{}", best.content, entry.content);
            let mut merged_tags = best.tags.clone();
            for tag in &entry.tags {
                if !merged_tags.contains(tag) {
                    merged_tags.push(tag.clone());
                }
            }
            self.update(best.id, &merged_content, &merged_tags)?;
            return Ok(AddResult::Updated { id: best.id });
        }

        // No similar entries — create new
        let id = self.add(entry)?;
        Ok(AddResult::Created { id })
    }

    fn list_distinct_project_scope_ids(&self) -> Result<Vec<String>> {
        let conn = self.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT scope_project_id
             FROM memories
             WHERE scope_type = 'project' AND scope_project_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn delete_batch(&self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.write_conn()?;
        let before = load_memory_entries_for_history(&conn, ids)?;
        let now = chrono::Utc::now().to_rfc3339();
        let placeholders = crate::sql_in_placeholders(ids.len());
        let sql = format!("DELETE FROM memories WHERE id IN ({})", placeholders);
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let deleted = conn.execute(&sql, param_refs.as_slice())?;

        // Also clean vec0 table
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims > 0 {
            let vec_sql = format!("DELETE FROM memories_vec WHERE rowid IN ({})", placeholders);
            let _ = conn.execute(&vec_sql, param_refs.as_slice());
        }

        if deleted > 0 {
            for entry in before {
                record_memory_history_best_effort(&conn, MemoryHistoryAction::Delete, &entry, &now);
            }
        }

        Ok(deleted)
    }

    fn import_entries(&self, entries: Vec<NewMemory>, dedup: bool) -> Result<ImportResult> {
        let mut result = ImportResult {
            created: 0,
            skipped_duplicate: 0,
            failed: 0,
            errors: Vec::new(),
        };

        let dedup_cfg = load_dedup_config();
        for entry in entries {
            if dedup {
                match self.add_with_dedup(
                    entry,
                    dedup_cfg.threshold_high,
                    dedup_cfg.threshold_merge,
                ) {
                    Ok(AddResult::Created { .. }) => result.created += 1,
                    Ok(AddResult::Duplicate { .. }) => result.skipped_duplicate += 1,
                    Ok(AddResult::Updated { .. }) => result.created += 1, // count merge as created
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            } else {
                match self.add(entry) {
                    Ok(_) => result.created += 1,
                    Err(e) => {
                        result.failed += 1;
                        result.errors.push(e.to_string());
                    }
                }
            }
        }

        Ok(result)
    }

    fn reembed_all(&self) -> Result<usize> {
        let cancel = tokio_util::sync::CancellationToken::new();
        let mut on_progress = |_done: usize, _total: usize| {};
        self.reembed_all_with_progress(&cancel, &mut on_progress, 16)
    }

    fn reembed_batch(&self, ids: &[i64]) -> Result<usize> {
        let mut entries = Vec::new();
        for id in ids {
            if let Some(entry) = self.get(*id)? {
                entries.push(entry);
            }
        }
        self.reembed_entries(&entries)
    }

    fn reembed_all_with_progress(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
        on_progress: &mut dyn FnMut(usize, usize),
        batch_size: usize,
    ) -> Result<usize> {
        let batch_size = batch_size.max(1);
        // Snapshot the ids up front. Offset pagination over
        // `pinned DESC, updated_at DESC` can drift while embedding network
        // calls are in flight, causing rows to be skipped silently.
        let ids = {
            let conn = self.read_conn()?;
            let mut stmt =
                conn.prepare("SELECT id FROM memories ORDER BY pinned DESC, updated_at DESC")?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let total = ids.len();
        on_progress(0, total);
        let mut processed = 0usize;
        let mut reembedded = 0usize;

        for chunk in ids.chunks(batch_size) {
            if cancel.is_cancelled() {
                return Err(anyhow::anyhow!("Reembed job cancelled"));
            }

            let mut entries = Vec::with_capacity(chunk.len());
            for id in chunk {
                if cancel.is_cancelled() {
                    return Err(anyhow::anyhow!("Reembed job cancelled"));
                }
                if let Some(entry) = self.get(*id)? {
                    entries.push(entry);
                }
            }

            reembedded += self.reembed_entries(&entries)?;
            processed += chunk.len();
            on_progress(processed, total);
        }

        // Claims share the memory embedding model (PR #8). Re-embed them after
        // memories so a model switch refreshes claim vectors too. Best-effort:
        // a failure here must not fail the (already-completed) memory reembed.
        let _ = self.reembed_claims(cancel);

        Ok(reembedded)
    }

    fn clear_all_embeddings(&self) -> Result<usize> {
        let conn = self.write_conn()?;
        let updated = conn.execute(
            "UPDATE memories SET embedding = NULL, embedding_signature = NULL",
            [],
        )?;
        let _ = conn.execute("DELETE FROM memories_vec", []);
        // Claims share the memory embedding model (PR #8): clear their vectors
        // too so a model switch doesn't leave stale-signature claim rows behind
        // (`reembed_claims` repopulates them right after).
        let _ = conn.execute("UPDATE memory_claims SET embedding_signature = NULL", []);
        let _ = conn.execute("DELETE FROM memory_claims_vec", []);
        Ok(updated)
    }

    fn count_memories_pending_embedding(&self, target_signature: &str) -> Result<u64> {
        let conn = self.read_conn()?;
        // 两类「pending」都得算：(1) embedding_signature 缺失或不匹配（add_memory
        // 在向量检索禁用期间写的 NULL row）；(2) signature 匹配但 `memories_vec`
        // 虚表没有对应 rowid 的 row（写入虚表失败 / vec 表未建 / 老数据漏建）。
        // 后者用 LEFT JOIN 检查——`memories_vec` 是 sqlite-vec 虚表，schema 缺失
        // 时 LEFT JOIN 会触发错误，所以先用 sqlite_master 探测是否存在再决定查询。
        let vec_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories_vec'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let count: i64 = if vec_table_exists > 0 {
            conn.query_row(
                "SELECT COUNT(*) FROM memories m
                  WHERE m.embedding_signature IS NULL
                     OR m.embedding_signature != ?1
                     OR NOT EXISTS (SELECT 1 FROM memories_vec v WHERE v.rowid = m.id)",
                rusqlite::params![target_signature],
                |row| row.get(0),
            )?
        } else {
            // 没有 memories_vec 虚表（首次启用 / dim=0 时未建），所有 memory 都
            // 算 pending —— set_embedder 会通过 ensure_vec_table 重建虚表，
            // 真正的 reembed 任务会逐行写回。
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?
        };
        Ok(count.max(0) as u64)
    }

    fn set_embedder(&self, provider: Arc<dyn EmbeddingProvider>) {
        let dims = provider.dimensions();
        self.embedding_dims
            .store(dims, std::sync::atomic::Ordering::Relaxed);
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = Some(provider);

        // Fast path: try_lock so settings/install flows aren't blocked by an
        // in-flight long memory write. On contention, retry on a background
        // thread so recall can use vector search before the next
        // add/update/reembed lazily creates the table.
        match self.writer.try_lock() {
            Ok(conn) => {
                let _ = self.ensure_vec_table(&conn, dims);
            }
            Err(_) => {
                std::thread::spawn(move || {
                    if let Some(backend) = crate::get_memory_backend() {
                        let _ = backend.ensure_vec_table_blocking(dims);
                    }
                });
            }
        }
    }

    fn clear_embedder(&self) {
        *self.embedder.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.embedding_dims
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn has_embedder(&self) -> bool {
        self.embedder
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    fn ensure_vec_table_blocking(&self, dims: u32) -> Result<()> {
        let conn = self.write_conn()?;
        self.ensure_vec_table(&conn, dims)
    }

    fn prune_embedding_cache_to_signature(&self, active_signature: &str) -> Result<usize> {
        let conn = self.write_conn()?;
        let deleted = conn.execute(
            "DELETE FROM embedding_cache WHERE signature != ?1",
            params![active_signature],
        )?;
        Ok(deleted)
    }

    fn backend_kind(&self) -> &'static str {
        "sqlite"
    }

    fn count_profile_memories(&self, window_days: u32) -> Result<u64> {
        // `tags` is a JSON array string; the exact-quoted `"profile"` LIKE
        // match keeps `profile_lead` or similar from false-positive. The
        // created_at column is ISO8601 text, so we compare via strftime('%s')
        // in SQL to avoid pulling rows into userspace.
        let cutoff = crate::util::epoch_cutoff_secs(window_days);
        let conn = self
            .readers
            .first()
            .unwrap_or(&self.writer)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories
                 WHERE tags LIKE '%\"profile\"%'
                   AND CAST(strftime('%s', created_at) AS INTEGER) >= ?1",
                params![cutoff],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(n as u64)
    }
}

// ── Convenience: open default DB ────────────────────────────────

/// Open the default memory database at ~/.hope-agent/memory.db
#[allow(dead_code)]
pub fn open_default() -> Result<SqliteMemoryBackend> {
    let db_path = crate::paths::memory_db_path()?;
    SqliteMemoryBackend::open(&db_path)
}

/// Memory ids that must NOT be injected into the system prompt because every
/// managing claim is non-injectable and no `user_pinned` link or injectable
/// claim keeps them alive (design §4.5). A claim is injectable when
/// `status='active'` and `valid_until` is unset or still in the future
/// (RFC3339 lexical compare, mirroring `claims::write::effective_status`).
/// Single set query — no N+1. Returns an empty set when the claim tables hold
/// nothing relevant (the dual-track default), so this is a cheap no-op until
/// claim dual-write is enabled.
fn hidden_claim_linked_memory_ids(
    conn: &rusqlite::Connection,
) -> Result<std::collections::HashSet<i64>> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut stmt = conn.prepare(
        // Only `managed` links participate in hiding: a managed link means the
        // claim OWNS that shadow memory (created by the dual-write). `detached`
        // links (dedup hits onto a pre-existing memory) and `user_pinned` links
        // never let a claim's lifecycle hide the memory — that was the
        // over-reach fix. So the candidate set is `managed` links only, and the
        // dead/alive EXISTS checks below also look at `managed` links only.
        "SELECT DISTINCT l.memory_id
         FROM memory_claim_links l
         WHERE l.sync_mode = 'managed'
           AND EXISTS (
                 SELECT 1 FROM memory_claim_links lx
                 JOIN memory_claims cx ON cx.id = lx.claim_id
                 WHERE lx.memory_id = l.memory_id
                   AND lx.sync_mode = 'managed'
                   AND NOT (cx.status = 'active'
                            AND (cx.valid_until IS NULL OR cx.valid_until = ''
                                 OR cx.valid_until >= ?1)))
           AND NOT EXISTS (
                 SELECT 1 FROM memory_claim_links lp
                 WHERE lp.memory_id = l.memory_id AND lp.sync_mode = 'user_pinned')
           AND NOT EXISTS (
                 SELECT 1 FROM memory_claim_links la
                 JOIN memory_claims ca ON ca.id = la.claim_id
                 WHERE la.memory_id = l.memory_id
                   AND la.sync_mode = 'managed'
                   AND ca.status = 'active'
                   AND (ca.valid_until IS NULL OR ca.valid_until = ''
                        OR ca.valid_until >= ?1))
           -- A user-pinned memory is never auto-hidden by claim status — pin
           -- is an explicit user keep signal (mirrors the user_pinned LINK
           -- exemption above, but for a memory the user pinned directly, e.g.
           -- one a managed claim dedup-attached onto).
           AND NOT EXISTS (
                 SELECT 1 FROM memories mm
                 WHERE mm.id = l.memory_id AND mm.pinned = 1)",
    )?;
    let ids = stmt
        .query_map(params![now], |row| row.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// Single-source dedup set for Context Pack (design §4.8): `memory_id`s that ARE
/// covered by an effective-active managed claim that clears the Pinned salience
/// bar (`PINNED_MIN_SALIENCE`) — they inject via the Pinned segment, so they must
/// be dropped from the legacy `# Memory`
/// section. The positive mirror of [`hidden_claim_linked_memory_ids`] (which
/// drops memories whose claims are all DEAD); here we drop those still owned by a
/// LIVE claim. The two sets are disjoint (presence vs absence of a live managed
/// link). Same exemptions: only `managed` links count, and a `user_pinned` link
/// or a directly-pinned memory (`memories.pinned = 1`) keeps the memory in the
/// legacy section (explicit keep outranks dedup). Empty/cheap when no claims
/// exist (the dual-track default).
fn covered_by_active_claim_memory_ids(
    conn: &rusqlite::Connection,
) -> Result<std::collections::HashSet<i64>> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    // Threshold-aligned with the Pinned segment (PINNED_MIN_SALIENCE): only a
    // claim that actually clears the pin bar — and therefore injects via Pinned —
    // may drop its shadow memory from the legacy section. A lower-salience managed
    // claim does NOT inject (Pinned needs >= the threshold), so its shadow stays
    // in the legacy section as the fallback and the fact keeps a static prompt
    // outlet (dedup must never be more aggressive than injection).
    let min_salience = crate::memory::dreaming::PINNED_MIN_SALIENCE as f64;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT l.memory_id
         FROM memory_claim_links l
         WHERE l.sync_mode = 'managed'
           AND EXISTS (
                 SELECT 1 FROM memory_claim_links la
                 JOIN memory_claims ca ON ca.id = la.claim_id
                 WHERE la.memory_id = l.memory_id
                   AND la.sync_mode = 'managed'
                   AND ca.status = 'active'
                   AND ca.salience >= ?2
                   AND (ca.valid_until IS NULL OR ca.valid_until = ''
                        OR ca.valid_until >= ?1))
           AND NOT EXISTS (
                 SELECT 1 FROM memory_claim_links lp
                 WHERE lp.memory_id = l.memory_id AND lp.sync_mode = 'user_pinned')
           AND NOT EXISTS (
                 SELECT 1 FROM memories mm
                 WHERE mm.id = l.memory_id AND mm.pinned = 1)",
    )?;
    let ids = stmt
        .query_map(params![now, min_salience], |row| row.get::<_, i64>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod claim_injection_tests {
    use super::*;

    struct FixedEmbedder;

    impl EmbeddingProvider for FixedEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>> {
            Ok(vec![0.25, 0.5, 0.75])
        }

        fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.25, 0.5, 0.75]).collect())
        }

        fn dimensions(&self) -> u32 {
            3
        }
    }

    fn temp_backend() -> SqliteMemoryBackend {
        let dir = std::env::temp_dir().join(format!("ha-inject-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        SqliteMemoryBackend::open(&dir.join("memory.db")).unwrap()
    }

    #[test]
    fn embedding_cache_crud_does_not_reenter_writer_lock() {
        let backend = Arc::new(temp_backend());
        backend.set_embedder(Arc::new(FixedEmbedder));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<i64> {
                let id = backend.add(NewMemory {
                    memory_type: MemoryType::User,
                    scope: MemoryScope::Global,
                    content: "embedding cache lock-order fixture".to_string(),
                    tags: vec!["lock-order".to_string()],
                    source: "test".to_string(),
                    source_session_id: None,
                    pinned: false,
                    attachment_path: None,
                    attachment_mime: None,
                })?;
                backend.update(
                    id,
                    "embedding cache lock-order fixture updated",
                    &["lock-order".to_string()],
                )?;
                Ok(id)
            })();
            let _ = tx.send(result);
        });

        let id = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("embedding cache CRUD deadlocked on the writer")
            .expect("embedding cache CRUD failed");
        assert!(id > 0);
    }

    #[test]
    fn memory_history_records_legacy_owner_events() {
        let backend = temp_backend();
        let user_id = backend
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "User prefers concise answers.".to_string(),
                tags: vec!["preference".to_string()],
                source: "user".to_string(),
                source_session_id: Some("session-a".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        let import_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Imported API token rotation note.".to_string(),
                tags: vec!["imported".to_string()],
                source: "import".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .update(
                user_id,
                "User prefers concise answers with implementation details.",
                &["preference".to_string(), "style".to_string()],
            )
            .unwrap();
        backend.toggle_pin(user_id, true).unwrap();
        backend.delete(import_id).unwrap();
        let batch_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "Batch delete audit fixture.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        assert_eq!(backend.delete_batch(&[batch_id, 999_999]).unwrap(), 1);

        let history = backend.history(20, 0).unwrap();
        let user_actions: Vec<MemoryHistoryAction> = history
            .iter()
            .filter(|event| event.memory_id == user_id)
            .map(|event| event.action.clone())
            .collect();
        assert!(user_actions.contains(&MemoryHistoryAction::Add));
        assert!(user_actions.contains(&MemoryHistoryAction::Update));
        assert!(user_actions.contains(&MemoryHistoryAction::Pin));

        let import_actions: Vec<MemoryHistoryAction> = history
            .iter()
            .filter(|event| event.memory_id == import_id)
            .map(|event| event.action.clone())
            .collect();
        assert!(import_actions.contains(&MemoryHistoryAction::Import));
        assert!(import_actions.contains(&MemoryHistoryAction::Delete));

        let batch_actions: Vec<MemoryHistoryAction> = history
            .iter()
            .filter(|event| event.memory_id == batch_id)
            .map(|event| event.action.clone())
            .collect();
        assert!(batch_actions.contains(&MemoryHistoryAction::Add));
        assert!(batch_actions.contains(&MemoryHistoryAction::Delete));

        let deleted_import = history
            .iter()
            .find(|event| {
                event.memory_id == import_id && event.action == MemoryHistoryAction::Delete
            })
            .expect("delete history for imported memory");
        assert_eq!(deleted_import.source, "import");
        assert_eq!(
            deleted_import.content_preview,
            "Imported API token rotation note."
        );
    }

    #[test]
    fn memory_history_filters_owner_audit_stream() {
        let backend = temp_backend();
        let percent_id = backend
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Literal 100% audit marker.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: Some("session-filter".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        let plain_id = backend
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Literal 100x audit marker.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: Some("session-other".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        let import_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Imported audit reference.".to_string(),
                tags: Vec::new(),
                source: "import".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .update(
                percent_id,
                "Literal 100% audit marker updated.",
                &["audit".to_string()],
            )
            .unwrap();

        let percent_adds = backend
            .history_filtered(&MemoryHistoryQuery {
                query: Some("100%".to_string()),
                actions: Some(vec![MemoryHistoryAction::Add]),
                memory_types: Some(vec![MemoryType::User]),
                sources: Some(vec!["user".to_string()]),
                limit: Some(20),
                offset: Some(0),
            })
            .unwrap();
        assert!(percent_adds
            .iter()
            .any(|event| event.memory_id == percent_id));
        assert!(!percent_adds.iter().any(|event| event.memory_id == plain_id));

        let update_only = backend
            .history_filtered(&MemoryHistoryQuery {
                query: Some("updated".to_string()),
                actions: Some(vec![MemoryHistoryAction::Update]),
                memory_types: Some(vec![MemoryType::User]),
                sources: Some(vec!["user".to_string()]),
                limit: Some(20),
                offset: Some(0),
            })
            .unwrap();
        assert_eq!(update_only.len(), 1);
        assert_eq!(update_only[0].memory_id, percent_id);
        assert_eq!(update_only[0].action, MemoryHistoryAction::Update);

        let session_hits = backend
            .history_filtered(&MemoryHistoryQuery {
                query: Some("session-filter".to_string()),
                actions: None,
                memory_types: None,
                sources: None,
                limit: Some(20),
                offset: Some(0),
            })
            .unwrap();
        assert!(session_hits
            .iter()
            .all(|event| event.memory_id == percent_id));

        let imported_refs = backend
            .history_filtered(&MemoryHistoryQuery {
                query: None,
                actions: Some(vec![MemoryHistoryAction::Import]),
                memory_types: Some(vec![MemoryType::Reference]),
                sources: Some(vec!["import".to_string()]),
                limit: Some(20),
                offset: Some(0),
            })
            .unwrap();
        assert_eq!(imported_refs.len(), 1);
        assert_eq!(imported_refs[0].memory_id, import_id);

        let user_add_page = backend
            .history_filtered_page(&MemoryHistoryQuery {
                query: Some("audit marker".to_string()),
                actions: Some(vec![MemoryHistoryAction::Add]),
                memory_types: Some(vec![MemoryType::User]),
                sources: Some(vec!["user".to_string()]),
                limit: Some(1),
                offset: Some(0),
            })
            .unwrap();
        assert_eq!(user_add_page.items.len(), 1);
        assert_eq!(user_add_page.total, 2);
        assert!(!user_add_page.total_truncated);
    }

    /// Insert a memory + a claim + a link with the given claim status / valid_until
    /// / link sync_mode, so the effective-status filter can be exercised.
    fn seed(
        conn: &rusqlite::Connection,
        mem_id: i64,
        claim_id: &str,
        status: &str,
        valid_until: Option<&str>,
        sync_mode: &str,
    ) {
        conn.execute(
            "INSERT INTO memories (id, memory_type, scope_type, content, source, created_at, updated_at)
             VALUES (?1, 'user', 'global', 'm', 'auto-claim', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            params![mem_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_claims
                (id, scope_type, claim_type, subject, predicate, object, content, status, valid_until, created_at, updated_at)
             VALUES (?1, 'global', 'preference', 'user', 'prefers', 'x', 'c', ?2, ?3, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            params![claim_id, status, valid_until],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_claim_links (claim_id, memory_id, sync_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            params![claim_id, mem_id, sync_mode],
        ).unwrap();
    }

    #[test]
    fn effective_status_filter_hides_only_dead_managed_links() {
        let backend = temp_backend();
        let conn = backend.write_conn().unwrap();
        // 1: active claim → visible.
        seed(&conn, 1, "c1", "active", None, "managed");
        // 2: superseded claim → hidden.
        seed(&conn, 2, "c2", "superseded", None, "managed");
        // 3: active but valid_until in the past → effective expired → hidden.
        seed(
            &conn,
            3,
            "c3",
            "active",
            Some("2020-01-01T00:00:00.000Z"),
            "managed",
        );
        // 4: superseded BUT user_pinned link → exempt, visible.
        seed(&conn, 4, "c4", "superseded", None, "user_pinned");
        // 6: superseded managed link, but the MEMORY is user-pinned → exempt.
        seed(&conn, 6, "c6", "superseded", None, "managed");
        conn.execute("UPDATE memories SET pinned = 1 WHERE id = 6", [])
            .unwrap();
        // 5: managed superseded link, but ALSO a managed active link → kept alive.
        seed(&conn, 5, "c5", "superseded", None, "managed");
        conn.execute(
            "INSERT INTO memory_claims (id, scope_type, claim_type, subject, predicate, object, content, status, created_at, updated_at)
             VALUES ('c5b', 'global', 'preference', 'user', 'prefers', 'y', 'c', 'active', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_claim_links (claim_id, memory_id, sync_mode, created_at, updated_at)
             VALUES ('c5b', 5, 'managed', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        ).unwrap();
        drop(conn);

        let read = backend.read_conn().unwrap();
        let hidden = hidden_claim_linked_memory_ids(&read).unwrap();
        assert!(!hidden.contains(&1), "active claim memory must stay");
        assert!(hidden.contains(&2), "superseded claim memory must hide");
        assert!(
            hidden.contains(&3),
            "expired-by-valid_until memory must hide"
        );
        assert!(!hidden.contains(&4), "user_pinned link is exempt");
        assert!(
            !hidden.contains(&5),
            "an active link keeps the memory alive"
        );
        assert!(
            !hidden.contains(&6),
            "a user-pinned memory is never auto-hidden"
        );
    }

    #[test]
    fn covered_by_active_claim_excludes_live_managed_links() {
        // Single-source dedup (design §4.8): a legacy memory covered by an
        // effective-active managed claim is dropped from the legacy section (it
        // injects via the Pinned/Relevant Claims segments). The positive mirror
        // of the `hidden` (dead-claim) set, with the same exemptions.
        let backend = temp_backend();
        let conn = backend.write_conn().unwrap();
        // 1: active managed claim, salience >= the Pinned bar → covered (it
        //    injects via Pinned, so its shadow drops from legacy).
        seed(&conn, 1, "c1", "active", None, "managed");
        conn.execute(
            "UPDATE memory_claims SET salience = 0.9 WHERE id = 'c1'",
            [],
        )
        .unwrap();
        // 2: superseded managed claim → NOT covered (that's the `hidden` set).
        seed(&conn, 2, "c2", "superseded", None, "managed");
        // 3: active but valid_until past → effective expired → NOT covered.
        seed(
            &conn,
            3,
            "c3",
            "active",
            Some("2020-01-01T00:00:00.000Z"),
            "managed",
        );
        // 4: active claim but a user_pinned link → exempt (explicit keep
        //    outranks dedup), NOT covered.
        seed(&conn, 4, "c4", "active", None, "user_pinned");
        // 5: active managed claim but the MEMORY is user-pinned → exempt.
        seed(&conn, 5, "c5", "active", None, "managed");
        conn.execute("UPDATE memories SET pinned = 1 WHERE id = 5", [])
            .unwrap();
        // 6: active managed claim BELOW the Pinned salience bar (default 0.5 <
        //    0.7) → NOT covered: it never injects via Pinned, so its shadow must
        //    stay in legacy as the fallback (dedup aligned to injection).
        seed(&conn, 6, "c6", "active", None, "managed");
        drop(conn);

        let read = backend.read_conn().unwrap();
        let covered = covered_by_active_claim_memory_ids(&read).unwrap();
        assert!(covered.contains(&1), "live managed claim covers its memory");
        assert!(
            !covered.contains(&2),
            "superseded claim does not cover (that's the hidden set)"
        );
        assert!(
            !covered.contains(&3),
            "expired-by-valid_until claim does not cover"
        );
        assert!(
            !covered.contains(&4),
            "user_pinned link is exempt from dedup"
        );
        assert!(
            !covered.contains(&5),
            "a user-pinned memory is never deduped away"
        );
        assert!(
            !covered.contains(&6),
            "a sub-pin-threshold claim does not cover (no Pinned outlet → legacy fallback)"
        );

        // covered (live) and hidden (dead) are disjoint: a memory is in at most
        // one set, so dedup + hide never fight over the same row.
        let hidden = hidden_claim_linked_memory_ids(&read).unwrap();
        assert!(
            covered.is_disjoint(&hidden),
            "covered and hidden sets must be disjoint"
        );
    }

    #[test]
    fn detached_link_never_hides_preexisting_memory() {
        let backend = temp_backend();
        let conn = backend.write_conn().unwrap();
        // 7: a pre-existing memory a claim dedup-merged onto (detached link).
        // Even though that claim later died (superseded), a detached link must
        // never let the claim's lifecycle hide the pre-existing memory — it was
        // never the claim's owned shadow (Codex adversarial finding #2).
        seed(&conn, 7, "c7", "superseded", None, "detached");
        // 8: same memory shape but a MANAGED dead link → the claim owns it, so
        // it IS hidden (control, mirrors id=2 above).
        seed(&conn, 8, "c8", "superseded", None, "managed");
        drop(conn);

        let read = backend.read_conn().unwrap();
        let hidden = hidden_claim_linked_memory_ids(&read).unwrap();
        assert!(
            !hidden.contains(&7),
            "a detached (dedup-hit) link must never hide a pre-existing memory"
        );
        assert!(
            hidden.contains(&8),
            "a managed dead link still hides the claim's owned shadow"
        );
    }

    #[test]
    fn no_claims_means_no_hidden() {
        let backend = temp_backend();
        let read = backend.read_conn().unwrap();
        assert!(hidden_claim_linked_memory_ids(&read).unwrap().is_empty());
    }

    #[test]
    fn search_matches_chinese_short_query() {
        let backend = temp_backend();

        let target_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Global,
                content: "用户偏好：中文 回复；提交前先跑 typecheck。".to_string(),
                tags: vec!["cjk-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "Release checklist mentions cargo fmt and pnpm lint.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let results = backend
            .search(&MemorySearchQuery {
                query: "中文".to_string(),
                types: None,
                sources: None,
                scope: Some(MemoryScope::Global),
                agent_id: None,
                limit: Some(5),
            })
            .unwrap();

        assert!(
            results.iter().any(|entry| entry.id == target_id),
            "Chinese short query should recall the CJK memory; got ids {:?}",
            results.iter().map(|entry| entry.id).collect::<Vec<_>>()
        );
        assert!(
            results
                .iter()
                .find(|entry| entry.id == target_id)
                .and_then(|entry| entry.retrieval_evidence.as_ref())
                .is_some_and(|evidence| evidence.lexical_match),
            "literal/FTS search must expose absolute lexical evidence"
        );
    }

    #[test]
    fn search_matches_code_identifier_prefix_query() {
        let backend = temp_backend();

        let target_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "The prepare_messages_for_api helper strips internal metadata before provider calls.".to_string(),
                tags: vec!["code-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "The release checklist mentions cargo fmt and pnpm lint.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let results = backend
            .search(&MemorySearchQuery {
                query: "prepare_messages".to_string(),
                types: None,
                sources: None,
                scope: Some(MemoryScope::Global),
                agent_id: None,
                limit: Some(5),
            })
            .unwrap();

        assert!(
            results.iter().any(|entry| entry.id == target_id),
            "Identifier prefix query should recall the full code identifier memory; got ids {:?}",
            results.iter().map(|entry| entry.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_literal_fallback_matches_cjk_substring_without_fts_row() {
        let backend = temp_backend();

        let target_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Global,
                content: "用户偏好：请默认使用中文回复，并保持说明简洁。".to_string(),
                tags: vec!["cjk-literal-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: Some("literal-cjk-session".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Global,
                content: "用户偏好：英文回答时保留术语。".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        backend
            .write_conn()
            .unwrap()
            .execute(
                "DELETE FROM memories_fts WHERE rowid = ?1",
                params![target_id],
            )
            .unwrap();

        let results = backend
            .search(&MemorySearchQuery {
                query: "文回".to_string(),
                types: Some(vec![MemoryType::Feedback]),
                sources: Some(vec!["user".to_string()]),
                scope: Some(MemoryScope::Global),
                agent_id: None,
                limit: Some(5),
            })
            .unwrap();

        assert!(
            results.iter().any(|entry| entry.id == target_id),
            "Literal CJK substring should recall the memory when FTS row is missing; got ids {:?}",
            results.iter().map(|entry| entry.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_literal_fallback_matches_identifier_infix_without_fts_row() {
        let backend = temp_backend();

        let target_id = backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "The prepare_messages_for_api helper strips internal metadata before provider calls.".to_string(),
                tags: vec!["code-literal-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: Some("literal-code-session".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "The release checklist mentions cargo fmt and pnpm lint.".to_string(),
                tags: Vec::new(),
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        backend
            .write_conn()
            .unwrap()
            .execute(
                "DELETE FROM memories_fts WHERE rowid = ?1",
                params![target_id],
            )
            .unwrap();

        let results = backend
            .search(&MemorySearchQuery {
                query: "messages_for".to_string(),
                types: Some(vec![MemoryType::Project]),
                sources: Some(vec!["user".to_string()]),
                scope: Some(MemoryScope::Global),
                agent_id: None,
                limit: Some(5),
            })
            .unwrap();

        assert!(
            results.iter().any(|entry| entry.id == target_id),
            "Literal identifier infix should recall the memory when FTS row is missing; got ids {:?}",
            results.iter().map(|entry| entry.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn repair_rebuilds_missing_fts_index() {
        let backend = temp_backend();
        let id = backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "repairable keyword index fixture".to_string(),
                tags: vec!["repair-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch("DROP TABLE memories_fts; DROP TABLE memories_literal_fts;")
                .unwrap();
        }
        let broken = backend.health().unwrap();
        assert_eq!(broken.fts_missing_rows, 1);
        assert!(broken
            .issues
            .iter()
            .any(|issue| issue.code == "memory_literal_fts_missing_rows"));

        let report = backend.repair(MemoryRepairAction::RebuildFts).unwrap();
        assert_eq!(report.after.fts_missing_rows, 0);
        assert!(!report
            .after
            .issues
            .iter()
            .any(|issue| issue.code == "memory_literal_fts_missing_rows"));
        assert!(
            report.changed,
            "repair should report a changed health state"
        );

        let results = backend
            .search(&MemorySearchQuery {
                query: "repairable".to_string(),
                types: None,
                sources: None,
                scope: Some(MemoryScope::Global),
                agent_id: None,
                limit: Some(5),
            })
            .unwrap();
        assert!(
            results.iter().any(|entry| entry.id == id),
            "rebuilt FTS index should make the memory searchable again"
        );
    }

    #[test]
    fn repair_creates_db_safety_snapshot() {
        let backend = temp_backend();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "snapshot repair fixture".to_string(),
                tags: vec!["snapshot-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let report = backend
            .repair(MemoryRepairAction::CreateDbSnapshot)
            .unwrap();
        assert!(report.changed);
        assert_eq!(report.before.total_memories, report.after.total_memories);

        let artifact_path = report
            .artifact_path
            .as_deref()
            .expect("snapshot repair should return an artifact path");
        let snapshot_dir = std::path::Path::new(artifact_path);
        assert!(snapshot_dir.is_dir(), "snapshot dir should exist");
        assert!(
            snapshot_dir.join("memory.db").is_file(),
            "snapshot should include memory.db"
        );
        assert!(
            snapshot_dir.join("manifest.json").is_file(),
            "snapshot should include a manifest"
        );
        let manifest_path = snapshot_dir.join("manifest.json");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(
            manifest["schemaVersion"].as_str(),
            Some("hope.memory.db_snapshot.v1")
        );
        let copied_files = manifest["copiedFiles"]
            .as_array()
            .expect("manifest should keep legacy copiedFiles");
        let files = manifest["files"]
            .as_array()
            .expect("manifest should include verifiable file metadata");
        assert_eq!(files.len(), copied_files.len());
        let db_file = files
            .iter()
            .find(|entry| entry["name"].as_str() == Some("memory.db"))
            .expect("manifest should include memory.db metadata");
        assert_eq!(
            db_file["sizeBytes"].as_u64().unwrap(),
            std::fs::metadata(snapshot_dir.join("memory.db"))
                .unwrap()
                .len()
        );
        let sha256 = db_file["sha256"].as_str().unwrap();
        assert_eq!(sha256.len(), 64);
        assert!(sha256.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(report.artifact_files.len(), files.len());
        let report_db_file = report
            .artifact_files
            .iter()
            .find(|entry| entry.name == "memory.db")
            .expect("repair report should include memory.db metadata");
        assert_eq!(
            report_db_file.size_bytes,
            db_file["sizeBytes"].as_u64().unwrap()
        );
        assert_eq!(report_db_file.sha256, sha256);

        let health = backend.health().unwrap();
        let latest = health
            .latest_db_snapshot
            .expect("health should discover latest DB safety snapshot");
        assert_eq!(latest.path, artifact_path);
        assert_eq!(latest.status, MemoryDbSnapshotStatus::Ok);
        assert!(latest.issues.is_empty());
        assert!(
            latest.files.iter().any(|entry| entry.name == "memory.db"
                && entry.size_bytes == report_db_file.size_bytes
                && entry.sha256 == report_db_file.sha256),
            "latest health snapshot should carry memory.db verification metadata"
        );
        let preview = backend.db_snapshot_restore_preview(artifact_path).unwrap();
        assert_eq!(preview.status, MemoryDbSnapshotRestoreStatus::Ready);
        assert!(preview.can_restore);
        assert_eq!(preview.quick_check, "ok");
        assert_eq!(
            preview.current_db_path,
            backend.db_path.to_string_lossy().to_string()
        );
        assert!(
            preview.files.iter().any(|entry| entry.name == "memory.db"
                && entry.status == MemoryDbSnapshotFileStatus::Ok
                && entry.expected_size_bytes == report_db_file.size_bytes
                && entry.actual_size_bytes == Some(report_db_file.size_bytes)
                && entry.expected_sha256 == report_db_file.sha256
                && entry.actual_sha256.as_deref() == Some(report_db_file.sha256.as_str())),
            "restore preview should verify memory.db size and sha256 before restore"
        );

        let unsupported_file = snapshot_dir.join("memory.db-extra");
        std::fs::write(&unsupported_file, b"extra").unwrap();
        let mut unsupported_manifest = manifest.clone();
        unsupported_manifest["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "memory.db-extra",
                "sizeBytes": std::fs::metadata(&unsupported_file).unwrap().len(),
                "sha256": sha256_file_hex(&unsupported_file).unwrap(),
            }));
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&unsupported_manifest).unwrap(),
        )
        .unwrap();
        let unsupported_preview = backend.db_snapshot_restore_preview(artifact_path).unwrap();
        assert_eq!(
            unsupported_preview.status,
            MemoryDbSnapshotRestoreStatus::MissingFiles
        );
        assert!(!unsupported_preview.can_restore);
        assert_eq!(unsupported_preview.quick_check, "not_checked");
        assert!(
            unsupported_preview
                .issues
                .iter()
                .any(|issue| issue.contains("unsupported snapshot DB file name")),
            "restore preview should reject snapshot manifests with extra file names"
        );
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(unsupported_file).unwrap();

        std::fs::remove_file(snapshot_dir.join("memory.db")).unwrap();
        let broken_health = backend.health().unwrap();
        let broken_snapshot = broken_health
            .latest_db_snapshot
            .expect("health should keep reporting incomplete latest snapshot");
        assert_eq!(broken_snapshot.status, MemoryDbSnapshotStatus::MissingFiles);
        assert!(
            broken_snapshot
                .issues
                .iter()
                .any(|issue| issue.contains("memory.db")),
            "snapshot issue should name the missing DB file"
        );
        assert!(
            broken_health
                .issues
                .iter()
                .any(|issue| issue.code == "db_snapshot_incomplete"),
            "health should surface incomplete latest snapshot as an actionable warning"
        );
        let broken_preview = backend.db_snapshot_restore_preview(artifact_path).unwrap();
        assert_eq!(
            broken_preview.status,
            MemoryDbSnapshotRestoreStatus::MissingFiles
        );
        assert!(!broken_preview.can_restore);
        assert_eq!(broken_preview.quick_check, "not_checked");
        assert!(broken_preview
            .issues
            .iter()
            .any(|issue| issue.contains("memory.db")));
    }

    #[test]
    fn db_snapshot_restore_restores_verified_snapshot_with_rollback() {
        let backend = temp_backend();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "kept before snapshot".to_string(),
                tags: vec!["restore-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        let snapshot = backend
            .repair(MemoryRepairAction::CreateDbSnapshot)
            .unwrap()
            .artifact_path
            .expect("snapshot repair should return an artifact path");

        backend
            .add(NewMemory {
                memory_type: MemoryType::Project,
                scope: MemoryScope::Global,
                content: "added after snapshot".to_string(),
                tags: vec!["restore-fixture".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        assert_eq!(backend.stats(None).unwrap().total, 2);

        let report = backend.db_snapshot_restore(&snapshot).unwrap();
        assert!(report.restored);
        assert_eq!(
            report.preflight.status,
            MemoryDbSnapshotRestoreStatus::Ready
        );
        assert_eq!(report.preflight.quick_check, "ok");
        assert_eq!(report.after.quick_check, "ok");
        assert!(
            std::path::Path::new(&report.rollback_snapshot_path).is_dir(),
            "restore should create a rollback snapshot before changing memory.db"
        );
        assert!(report
            .rollback_snapshot_files
            .iter()
            .any(|file| file.name == "memory.db"));

        let memories = backend.list(None, None, 10, 0).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "kept before snapshot");
    }

    #[test]
    fn repair_rebuilds_missing_claim_fts_index() {
        let backend = temp_backend();
        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch(
                "INSERT INTO memory_claims
                    (id, scope_type, claim_type, subject, predicate, object, content,
                     status, created_at, updated_at)
                 VALUES (
                    'claim-fts-repair',
                    'global',
                    'preference',
                    'user',
                    'prefers',
                    'claimftsrepair',
                    'User prefers claimftsrepair workflows',
                    'active',
                    '2026-01-01T00:00:00.000Z',
                    '2026-01-01T00:00:00.000Z'
                 );
                 INSERT INTO memory_evidence
                    (id, claim_id, source_type, evidence_class, source_id, quote, created_at)
                 VALUES (
                    'evidence-fts-repair',
                    'claim-fts-repair',
                    'session_message',
                    'explicit_user_statement',
                    'message:claim-fts-repair',
                    'Evidence mentions evidenceftsrepair',
                    '2026-01-01T00:00:00.000Z'
                 );
                 DROP TABLE memory_claims_fts;
                 DROP TABLE memory_claims_literal_fts;
                 DROP TABLE memory_evidence_fts;",
            )
            .unwrap();
        }

        let broken = backend.health().unwrap();
        assert_eq!(broken.claim_fts_missing_rows, 1);
        assert_eq!(broken.evidence_fts_missing_rows, 1);
        assert!(broken
            .issues
            .iter()
            .any(|issue| issue.code == "claim_fts_missing"));
        assert!(broken
            .issues
            .iter()
            .any(|issue| issue.code == "evidence_fts_missing"));
        assert!(broken
            .issues
            .iter()
            .any(|issue| issue.code == "claim_literal_fts_missing_rows"));

        let report = backend.repair(MemoryRepairAction::RebuildClaimFts).unwrap();
        assert!(report.changed);
        assert_eq!(report.after.claim_fts_missing_rows, 0);
        assert_eq!(report.after.evidence_fts_missing_rows, 0);
        assert!(!report
            .after
            .issues
            .iter()
            .any(|issue| issue.code == "claim_literal_fts_missing_rows"));

        let conn = backend.read_conn().unwrap();
        let hits = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_claims_fts
                 WHERE memory_claims_fts MATCH 'claimftsrepair'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(
            hits, 1,
            "rebuilt claim FTS index should make the structured memory searchable"
        );
        let evidence_hits = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_evidence_fts
                 WHERE memory_evidence_fts MATCH 'evidenceftsrepair'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(
            evidence_hits, 1,
            "rebuilding structured index should also restore evidence FTS"
        );
    }

    #[test]
    fn repair_claim_graph_removes_orphan_evidence_and_links() {
        let backend = temp_backend();
        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch(
                "PRAGMA foreign_keys=OFF;
                 INSERT INTO memory_evidence
                    (id, claim_id, source_type, evidence_class, source_id, created_at)
                 VALUES
                    ('ev-orphan', 'missing-claim', 'manual', 'manual_correction', 'manual:1',
                     '2026-01-01T00:00:00.000Z');
                 INSERT INTO memory_claim_links
                    (claim_id, memory_id, sync_mode, created_at, updated_at)
                 VALUES
                    ('missing-claim', 99999, 'managed',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 PRAGMA foreign_keys=ON;",
            )
            .unwrap();
        }

        let broken = backend.health().unwrap();
        assert_eq!(broken.orphan_evidence_rows, 1);
        assert_eq!(broken.orphan_claim_links, 1);

        let report = backend
            .repair(MemoryRepairAction::RepairClaimGraph)
            .unwrap();
        assert!(report.changed);
        assert_eq!(report.after.orphan_evidence_rows, 0);
        assert_eq!(report.after.orphan_claim_links, 0);

        let second = backend
            .repair(MemoryRepairAction::RepairClaimGraph)
            .unwrap();
        assert!(!second.changed, "second repair should be a no-op");
    }

    #[test]
    fn health_reports_orphan_procedure_episode_refs() {
        let backend = temp_backend();
        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch(
                "INSERT INTO memory_episodes
                    (id, scope_type, title, situation, actions_json, outcome, lesson,
                     success_score, tags_json, status, created_at, updated_at)
                 VALUES
                    ('episode-health-ok', 'global', 'Health fixture', 'Situation', '[]',
                     'Outcome', 'Lesson', 0.8, '[]', 'active',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO memory_procedures
                    (id, scope_type, title, trigger, steps_markdown, constraints_markdown,
                     confidence, status, source_episode_ids_json, tags_json, created_at, updated_at)
                 VALUES
                    ('procedure-health-orphan', 'global', 'Procedure fixture', 'When testing health',
                     '1. Check links', '', 0.7, 'active',
                     '[\"episode-health-ok\",\"missing-episode\"]', '[]',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
            )
            .unwrap();
        }

        let health = backend.health().unwrap();
        assert_eq!(health.episodes_total, 1);
        assert_eq!(health.procedures_total, 1);
        assert_eq!(health.orphan_procedure_episode_refs, 1);
        assert!(health
            .issues
            .iter()
            .any(|issue| issue.code == "orphan_procedure_episode_refs"));
    }

    #[test]
    fn repair_experience_graph_prunes_missing_episode_refs() {
        let backend = temp_backend();
        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch(
                "INSERT INTO memory_episodes
                    (id, scope_type, title, situation, actions_json, outcome, lesson,
                     success_score, tags_json, status, created_at, updated_at)
                 VALUES
                    ('episode-health-ok', 'global', 'Health fixture', 'Situation', '[]',
                     'Outcome', 'Lesson', 0.8, '[]', 'active',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
                 INSERT INTO memory_procedures
                    (id, scope_type, title, trigger, steps_markdown, constraints_markdown,
                     confidence, status, source_episode_ids_json, tags_json, created_at, updated_at)
                 VALUES
                    ('procedure-health-orphan', 'global', 'Procedure fixture', 'When testing health',
                     '1. Check links', '', 0.7, 'active',
                     '[\"episode-health-ok\",\"missing-episode\"]', '[]',
                     '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
            )
            .unwrap();
        }

        let broken = backend.health().unwrap();
        assert_eq!(broken.orphan_procedure_episode_refs, 1);

        let report = backend
            .repair(MemoryRepairAction::RepairExperienceGraph)
            .unwrap();
        assert!(report.changed);
        assert_eq!(report.after.orphan_procedure_episode_refs, 0);

        let conn = backend.read_conn().unwrap();
        let raw: String = conn
            .query_row(
                "SELECT source_episode_ids_json
                 FROM memory_procedures
                 WHERE id = 'procedure-health-orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "[\"episode-health-ok\"]");

        let second = backend
            .repair(MemoryRepairAction::RepairExperienceGraph)
            .unwrap();
        assert!(!second.changed, "second repair should be a no-op");
    }

    #[test]
    fn repair_recovers_stale_dreaming_state() {
        let backend = temp_backend();
        {
            let conn = backend.write_conn().unwrap();
            conn.execute_batch(
                "INSERT INTO dreaming_runs
                    (id, trigger, phase, status, owner_instance_id, heartbeat_at,
                     lease_expires_at, started_at, scope_json)
                 VALUES
                    ('stale-run', 'manual', 'light', 'running', 'owner-1',
                     '2020-01-01T00:00:00.000Z',
                     '2020-01-01T00:00:00.000Z',
                     '2020-01-01T00:00:00.000Z',
                     '{}');
                 INSERT INTO dreaming_locks
                    (lock_key, run_id, owner_instance_id, heartbeat_at, lease_expires_at)
                 VALUES
                    ('light:global', 'stale-run', 'owner-1',
                     '2020-01-01T00:00:00.000Z',
                     '2020-01-01T00:00:00.000Z');",
            )
            .unwrap();
        }

        let broken = backend.health().unwrap();
        assert_eq!(broken.dreaming_stale_runs, 1);
        assert_eq!(broken.dreaming_stale_locks, 1);
        assert!(broken
            .issues
            .iter()
            .any(|issue| issue.code == "dreaming_state_stale"));

        let report = backend
            .repair(MemoryRepairAction::RecoverDreamingState)
            .unwrap();
        assert!(report.changed);
        assert_eq!(report.after.dreaming_stale_runs, 0);
        assert_eq!(report.after.dreaming_stale_locks, 0);

        let conn = backend.read_conn().unwrap();
        let status = conn
            .query_row(
                "SELECT status FROM dreaming_runs WHERE id = 'stale-run'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let locks = conn
            .query_row("SELECT COUNT(*) FROM dreaming_locks", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(locks, 0);

        let second = backend
            .repair(MemoryRepairAction::RecoverDreamingState)
            .unwrap();
        assert!(!second.changed, "second repair should be a no-op");
    }
}
