use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::json;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

use crate::memory::claims::{self, ClaimDetail, ClaimListFilter, ClaimRecord};
use crate::memory::dreaming::ProfileSnapshotRecord;
use crate::memory::{
    ImportResult, MemoryBackend, MemoryBackupAttachmentChunkPayload,
    MemoryBackupAttachmentExternalPayload, MemoryBackupAttachmentPayload, MemoryBackupBundle,
    MemoryBackupCipherManifest, MemoryBackupClaimConflictExample, MemoryBackupClaimRestorePlan,
    MemoryBackupImportPreview, MemoryBackupKdfManifest, MemoryBackupManifest,
    MemoryBackupPreviewIssue, MemoryBackupProfileRestorePlan, MemoryBackupRestoreOptions,
    MemoryBackupRestoreResult, MemoryBackupStructuredRestoreOptions,
    MemoryBackupStructuredRestoreResult, MemoryEncryptedBackupBundle, MemoryEntry,
    MemoryEpisodeQuery, MemoryEpisodeRecord, MemoryExperienceHistoryQuery,
    MemoryExperienceHistoryRecord, MemoryHealthSeverity, MemoryHistoryRecord, MemoryProcedureQuery,
    MemoryProcedureRecord, NewMemory,
};
use crate::util::now_rfc3339;

pub const MEMORY_BACKUP_SCHEMA_VERSION: &str = "hope.memory.bundle.v1";
pub const MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION: &str = "hope.memory.encrypted_bundle.v1";
pub const MEMORY_BACKUP_ARCHIVE_MIME: &str = "application/zip";
const MEMORY_PAGE_SIZE: usize = 1000;
const MEMORY_HISTORY_PAGE_SIZE: usize = 200;
const CLAIM_PAGE_SIZE: usize = 500;
const MAX_ATTACHMENT_PAYLOAD_BYTES: u64 = 10 * 1024 * 1024;
const ATTACHMENT_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHUNKED_ATTACHMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ATTACHMENT_SIDECAR_BYTES: u64 = 512 * 1024 * 1024;
const ENCRYPTED_BACKUP_KDF_NAME: &str = "pbkdf2-hmac-sha256";
const ENCRYPTED_BACKUP_CIPHER_NAME: &str = "aes-256-ctr-hmac-sha256";
const ENCRYPTED_BACKUP_KDF_ITERATIONS: u32 = 210_000;
const ENCRYPTED_BACKUP_SALT_BYTES: usize = 16;
const ENCRYPTED_BACKUP_NONCE_BYTES: usize = 16;
type HmacSha256 = Hmac<Sha256>;

/// Export a portable, read-only JSON snapshot of the memory system. Restore is
/// deliberately not part of this path; future imports should first build a
/// preview/backfill plan from this bundle.
pub fn export_backup_bundle(backend: &dyn MemoryBackend) -> Result<MemoryBackupBundle> {
    let stats = backend.stats(None)?;
    let legacy_memories = collect_all_memories(backend)?;
    let mut warnings = Vec::new();
    let legacy_history = collect_all_history(backend, &mut warnings)?;
    let legacy_markdown = backend.export_markdown(None)?;
    let health = match backend.health() {
        Ok(health) => Some(health),
        Err(e) => {
            warnings.push(format!("health export skipped: {e}"));
            None
        }
    };
    let claims = match collect_all_claim_details() {
        Ok(claims) => claims,
        Err(e) => {
            warnings.push(format!("claim graph export skipped: {e}"));
            Vec::new()
        }
    };
    let profile_snapshots = match crate::memory::dreaming::list_profile_snapshots() {
        Ok(snapshots) => snapshots,
        Err(e) => {
            warnings.push(format!("profile snapshot export skipped: {e}"));
            Vec::new()
        }
    };
    let episodes = match collect_all_episodes() {
        Ok(episodes) => episodes,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            warnings.push(format!("episode export skipped: {e}"));
            Vec::new()
        }
    };
    let procedures = match collect_all_procedures() {
        Ok(procedures) => procedures,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            warnings.push(format!("procedure export skipped: {e}"));
            Vec::new()
        }
    };
    let experience_history = match collect_all_experience_history() {
        Ok(history) => history,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            warnings.push(format!("experience history export skipped: {e}"));
            Vec::new()
        }
    };

    let evidence_count = claims.iter().map(|c| c.evidence.len()).sum();
    let claim_link_count = claims.iter().map(|c| c.links.len()).sum();
    let attachment_ref_count = legacy_memories
        .iter()
        .filter(|m| m.attachment_path.is_some())
        .count();
    let (attachment_payloads, attachment_payload_chunks, attachment_external_payloads) =
        collect_attachment_payloads(&legacy_memories, &mut warnings);
    let attachment_payload_count = attachment_payloads.len();
    let attachment_chunk_count = attachment_payload_chunks.len();
    let attachment_chunked_ref_count = unique_chunk_memory_count(&attachment_payload_chunks);
    let attachment_external_ref_count = attachment_external_payloads.len();
    let attachment_payload_bytes = attachment_payloads
        .iter()
        .map(|p| p.size_bytes)
        .sum::<u64>()
        + chunked_payload_total_bytes(&attachment_payload_chunks);
    let packed_attachment_count = attachment_payload_count + attachment_chunked_ref_count;
    let attachment_missing_count = attachment_ref_count.saturating_sub(packed_attachment_count);
    let mut unsupported_sections = Vec::new();
    if attachment_missing_count > 0 {
        unsupported_sections.push("memory_attachment_binary_payloads_missing".to_string());
        warnings.push(format!(
            "{attachment_missing_count} memory attachment reference(s) could not be packed"
        ));
    }

    let manifest = MemoryBackupManifest {
        complete: warnings.is_empty(),
        legacy_memory_count: legacy_memories.len(),
        legacy_history_count: legacy_history.len(),
        attachment_ref_count,
        attachment_payload_count,
        attachment_chunk_count,
        attachment_chunked_ref_count,
        attachment_external_ref_count,
        attachment_payload_bytes,
        attachment_missing_count,
        claim_count: claims.len(),
        evidence_count,
        claim_link_count,
        profile_snapshot_count: profile_snapshots.len(),
        episode_count: episodes.len(),
        procedure_count: procedures.len(),
        experience_history_count: experience_history.len(),
        unsupported_sections,
        warnings,
    };

    Ok(MemoryBackupBundle {
        schema_version: MEMORY_BACKUP_SCHEMA_VERSION.to_string(),
        exported_at: now_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        manifest,
        stats,
        health,
        config_manifest: memory_config_manifest(),
        legacy_memories,
        legacy_history,
        attachment_payloads,
        attachment_payload_chunks,
        attachment_external_payloads,
        legacy_markdown,
        claims,
        profile_snapshots,
        episodes,
        procedures,
        experience_history,
    })
}

/// Export a ZIP backup package containing `memory-backup.json` plus external
/// attachment sidecars under `attachments/`. The JSON bundle stays the canonical
/// manifest; sidecars are verified by size + sha256 during future restore.
pub fn export_backup_archive(backend: &dyn MemoryBackend) -> Result<Vec<u8>> {
    let bundle = export_backup_bundle(backend)?;
    build_backup_archive_from_bundle(&bundle)
}

/// Preview a ZIP memory backup package. The archive must contain
/// `memory-backup.json`; optional large attachment sidecars are read from
/// `attachments/` and verified against the JSON metadata before they are marked
/// restorable.
pub fn preview_backup_archive(
    backend: &dyn MemoryBackend,
    archive_bytes: &[u8],
) -> Result<MemoryBackupImportPreview> {
    let current_stats = backend.stats(None)?;
    match parse_backup_archive(archive_bytes) {
        Ok(parsed) => preview_backup_archive_from_parsed(backend, &parsed),
        Err(e) => {
            let mut preview = MemoryBackupImportPreview::empty(current_stats);
            preview.issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Error,
                code: "invalid_archive".to_string(),
                message: format!("Backup package is not a valid Hope Agent memory archive: {e}"),
            });
            preview.next_steps = build_preview_next_steps(&preview);
            Ok(preview)
        }
    }
}

fn build_backup_archive_from_bundle(bundle: &MemoryBackupBundle) -> Result<Vec<u8>> {
    let sidecars = collect_archive_sidecar_sources(bundle);
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        zip.start_file("memory-backup.json", options)
            .context("starting memory-backup.json in archive")?;
        let bundle_json = serde_json::to_vec_pretty(bundle)?;
        zip.write_all(&bundle_json)
            .context("writing memory-backup.json to archive")?;

        let stored_options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o600);
        for sidecar in sidecars {
            let mut file = std::fs::File::open(&sidecar.source_path)
                .with_context(|| format!("opening {}", sidecar.source_path.display()))?;
            zip.start_file(sidecar.archive_name, stored_options)
                .context("starting attachment sidecar in archive")?;
            std::io::copy(&mut file, &mut zip).context("writing attachment sidecar to archive")?;
        }
        zip.finish().context("finishing memory backup archive")?;
    }
    Ok(cursor.into_inner())
}

/// Export an encrypted memory backup envelope. The inner plaintext is exactly
/// the normal backup bundle JSON, so future restore logic has one canonical
/// schema to validate after decryption.
pub fn export_encrypted_backup_bundle(
    backend: &dyn MemoryBackend,
    passphrase: &str,
) -> Result<MemoryEncryptedBackupBundle> {
    validate_backup_export_passphrase(passphrase)?;
    let bundle = export_backup_bundle(backend)?;
    let plaintext = serde_json::to_vec(&bundle)?;
    encrypt_backup_plaintext(
        &plaintext,
        passphrase,
        bundle.exported_at,
        bundle.app_version,
    )
}

/// Validate a backup bundle and build a read-only import preview. This never
/// writes to memory.db; later restore work must consume the preview as a plan.
pub fn preview_backup_bundle(
    backend: &dyn MemoryBackend,
    content: &str,
) -> Result<MemoryBackupImportPreview> {
    preview_backup_bundle_with_passphrase(backend, content, None)
}

pub fn preview_backup_bundle_with_passphrase(
    backend: &dyn MemoryBackend,
    content: &str,
    passphrase: Option<&str>,
) -> Result<MemoryBackupImportPreview> {
    let current_stats = backend.stats(None)?;
    let mut preview = MemoryBackupImportPreview::empty(current_stats);
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            preview.issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Error,
                code: "invalid_json".to_string(),
                message: format!("Backup file is not valid JSON: {e}"),
            });
            preview.next_steps = build_preview_next_steps(&preview);
            return Ok(preview);
        }
    };

    preview.schema_version = parsed
        .get("schemaVersion")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if preview.schema_version.as_deref() == Some(MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION) {
        let Some(passphrase) = passphrase else {
            preview.issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Error,
                code: "encrypted_passphrase_required".to_string(),
                message: "This backup is encrypted; enter its passphrase to preview or restore."
                    .to_string(),
            });
            preview.next_steps.push(
                "Enter the backup passphrase to preview or restore this encrypted backup."
                    .to_string(),
            );
            return Ok(preview);
        };
        let decrypted = match decrypt_encrypted_backup_value(parsed, passphrase) {
            Ok(bytes) => bytes,
            Err(e) => {
                preview.issues.push(MemoryBackupPreviewIssue {
                    severity: MemoryHealthSeverity::Error,
                    code: "encrypted_decrypt_failed".to_string(),
                    message: format!("Encrypted backup could not be decrypted: {e}"),
                });
                preview.next_steps.push(
                    "Check the backup passphrase or choose an uncorrupted encrypted backup."
                        .to_string(),
                );
                return Ok(preview);
            }
        };
        let decrypted_content = match std::str::from_utf8(&decrypted) {
            Ok(content) => content,
            Err(e) => {
                preview.issues.push(MemoryBackupPreviewIssue {
                    severity: MemoryHealthSeverity::Error,
                    code: "encrypted_plaintext_invalid".to_string(),
                    message: format!(
                        "Encrypted backup decrypted, but the decrypted bundle is not a valid Hope Agent memory backup: {e}"
                    ),
                });
                preview.next_steps.push(
                    "Choose an uncorrupted encrypted backup or export the backup again."
                        .to_string(),
                );
                return Ok(preview);
            }
        };
        return preview_backup_bundle_with_passphrase(backend, decrypted_content, None);
    }
    if preview.schema_version.as_deref() != Some(MEMORY_BACKUP_SCHEMA_VERSION) {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Error,
            code: "unsupported_schema".to_string(),
            message: format!(
                "Unsupported memory backup schema: {}",
                preview
                    .schema_version
                    .as_deref()
                    .unwrap_or("<missing schemaVersion>")
            ),
        });
        preview.next_steps = build_preview_next_steps(&preview);
        return Ok(preview);
    }

    let bundle: MemoryBackupBundle = match serde_json::from_value(parsed) {
        Ok(bundle) => bundle,
        Err(e) => {
            preview.issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Error,
                code: "invalid_bundle_shape".to_string(),
                message: format!("Backup schema is recognized but the bundle is incomplete: {e}"),
            });
            preview.next_steps = build_preview_next_steps(&preview);
            return Ok(preview);
        }
    };

    preview.valid = true;
    preview.schema_version = Some(bundle.schema_version.clone());
    preview.exported_at = Some(bundle.exported_at.clone());
    preview.app_version = Some(bundle.app_version.clone());
    preview.source_manifest = Some(bundle.manifest.clone());
    preview.legacy_memory_count = bundle.legacy_memories.len();
    preview.legacy_history_count = bundle.legacy_history.len();
    preview.attachment_ref_count = bundle
        .legacy_memories
        .iter()
        .filter(|m| m.attachment_path.is_some())
        .count();
    preview.attachment_payload_count = bundle.attachment_payloads.len();
    preview.attachment_chunk_count = bundle.attachment_payload_chunks.len();
    preview.attachment_chunked_ref_count =
        unique_chunk_memory_count(&bundle.attachment_payload_chunks);
    preview.attachment_external_ref_count = bundle.attachment_external_payloads.len();
    preview.attachment_payload_bytes = bundle
        .attachment_payloads
        .iter()
        .map(|payload| payload.size_bytes)
        .sum::<u64>()
        + chunked_payload_total_bytes(&bundle.attachment_payload_chunks);
    preview.attachment_missing_count = preview
        .attachment_ref_count
        .saturating_sub(preview.attachment_payload_count + preview.attachment_chunked_ref_count);
    preview.claim_count = bundle.claims.len();
    preview.evidence_count = bundle.claims.iter().map(|c| c.evidence.len()).sum();
    preview.claim_link_count = bundle.claims.iter().map(|c| c.links.len()).sum();
    preview.profile_snapshot_count = bundle.profile_snapshots.len();
    preview.episode_count = bundle.episodes.len();
    preview.procedure_count = bundle.procedures.len();
    preview.experience_history_count = bundle.experience_history.len();
    preview.unsupported_sections = bundle.manifest.unsupported_sections.clone();

    let current_memories = collect_all_memories(backend)?;
    let current_fingerprints: HashSet<String> =
        current_memories.iter().map(memory_fingerprint).collect();
    let mut bundle_fingerprints = HashSet::new();
    for memory in &bundle.legacy_memories {
        let fingerprint = memory_fingerprint(memory);
        if current_fingerprints.contains(&fingerprint) {
            preview.legacy_exact_matches += 1;
        } else {
            preview.legacy_import_candidates += 1;
        }
        if !bundle_fingerprints.insert(fingerprint) {
            preview.legacy_duplicate_in_bundle += 1;
        }
    }
    let (history_restorable, history_skipped_unmapped) =
        legacy_history_restore_counts(&bundle, &current_memories, true);
    preview.legacy_history_restorable = history_restorable;
    preview.legacy_history_skipped_unmapped = history_skipped_unmapped;

    let claim_restore_plan = build_claim_restore_plan(&bundle.claims, &mut preview.issues);
    preview.claim_id_matches = claim_restore_plan.existing_by_id;
    preview.claim_restore_plan = claim_restore_plan;
    preview.profile_restore_plan =
        build_profile_restore_plan(&bundle.profile_snapshots, &mut preview.issues);
    let (episode_id_matches, episode_exact_matches, episode_import_candidates) =
        episode_restore_counts(&bundle.episodes, &mut preview.issues);
    preview.episode_id_matches = episode_id_matches;
    preview.episode_exact_matches = episode_exact_matches;
    preview.episode_import_candidates = episode_import_candidates;
    let (procedure_id_matches, procedure_exact_matches, procedure_import_candidates) =
        procedure_restore_counts(&bundle.procedures, &mut preview.issues);
    preview.procedure_id_matches = procedure_id_matches;
    preview.procedure_exact_matches = procedure_exact_matches;
    preview.procedure_import_candidates = procedure_import_candidates;
    let (experience_history_restorable, experience_history_skipped_unmapped) =
        experience_history_restore_counts(&bundle, &mut preview.issues, true);
    preview.experience_history_restorable = experience_history_restorable;
    preview.experience_history_skipped_unmapped = experience_history_skipped_unmapped;

    if !bundle.manifest.complete {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Warning,
            code: "source_bundle_partial".to_string(),
            message: "Source bundle was exported with warnings; inspect manifest.warnings before restoring.".to_string(),
        });
    }
    for warning in &bundle.manifest.warnings {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "source_bundle_warning".to_string(),
            message: warning.clone(),
        });
    }
    if preview.legacy_duplicate_in_bundle > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Warning,
            code: "duplicate_legacy_memories_in_bundle".to_string(),
            message: format!(
                "{} duplicate legacy memory row(s) appear inside the backup",
                preview.legacy_duplicate_in_bundle
            ),
        });
    }
    if preview.attachment_missing_count > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "attachments_reference_only".to_string(),
            message: format!(
                "{} attachment path(s) are present as references only and cannot be restored on this machine",
                preview.attachment_missing_count
            ),
        });
    }
    if preview.attachment_chunked_ref_count > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "attachments_chunked_payloads".to_string(),
            message: format!(
                "{} attachment payload(s) are stored as {} verified chunk(s)",
                preview.attachment_chunked_ref_count, preview.attachment_chunk_count
            ),
        });
    }
    if preview.attachment_external_ref_count > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "attachments_external_sidecar_required".to_string(),
            message: format!(
                "{} attachment payload(s) require an external sidecar file before they can be restored",
                preview.attachment_external_ref_count
            ),
        });
    }
    if preview.legacy_history_skipped_unmapped > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "legacy_history_partially_unmapped".to_string(),
            message: format!(
                "{} legacy memory history event(s) cannot be safely mapped to a local memory row",
                preview.legacy_history_skipped_unmapped
            ),
        });
    }

    preview.next_steps = build_preview_next_steps(&preview);
    Ok(preview)
}

fn preview_backup_archive_from_parsed(
    backend: &dyn MemoryBackend,
    parsed: &ParsedBackupArchive,
) -> Result<MemoryBackupImportPreview> {
    let content = serde_json::to_string(&parsed.bundle)?;
    let mut preview = preview_backup_bundle(backend, &content)?;
    if !preview.valid {
        return Ok(preview);
    }

    let available_sidecar_count = parsed.external_payloads_by_memory_id.len();
    let sidecar_bytes = parsed
        .external_payloads_by_memory_id
        .values()
        .map(|payload| payload.metadata.size_bytes)
        .sum::<u64>();
    preview.attachment_external_available_count = available_sidecar_count;
    preview.attachment_payload_bytes = preview
        .attachment_payload_bytes
        .saturating_add(sidecar_bytes);
    preview.attachment_missing_count = preview.attachment_ref_count.saturating_sub(
        preview.attachment_payload_count
            + preview.attachment_chunked_ref_count
            + preview.attachment_external_available_count,
    );
    preview.issues.retain(|issue| {
        !matches!(
            issue.code.as_str(),
            "attachments_reference_only" | "attachments_external_sidecar_required"
        )
    });
    if preview.attachment_external_available_count > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "attachments_external_sidecars_available".to_string(),
            message: format!(
                "{} large attachment sidecar payload(s) are present and checksum-verified",
                preview.attachment_external_available_count
            ),
        });
    }
    let missing_sidecars = preview
        .attachment_external_ref_count
        .saturating_sub(preview.attachment_external_available_count);
    if missing_sidecars > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Warning,
            code: "attachments_external_sidecar_missing".to_string(),
            message: format!(
                "{} large attachment sidecar payload(s) are missing or failed verification",
                missing_sidecars
            ),
        });
    }
    if preview.attachment_missing_count > 0 {
        preview.issues.push(MemoryBackupPreviewIssue {
            severity: MemoryHealthSeverity::Info,
            code: "attachments_reference_only".to_string(),
            message: format!(
                "{} attachment path(s) are present as references only and cannot be restored on this machine",
                preview.attachment_missing_count
            ),
        });
    }
    preview.issues.extend(parsed.sidecar_issues.clone());
    preview.next_steps = build_preview_next_steps(&preview);
    Ok(preview)
}

/// Apply the safe subset of a backup bundle: missing legacy memory rows only.
/// Structured claims, profile snapshots, episodes, procedures, and attachment
/// binaries remain preview-only until their restore plans can preserve graph
/// integrity and payload provenance.
pub fn restore_backup_legacy_memories(
    backend: &dyn MemoryBackend,
    content: &str,
    options: MemoryBackupRestoreOptions,
) -> Result<MemoryBackupRestoreResult> {
    restore_backup_legacy_memories_with_passphrase(backend, content, options, None)
}

pub fn restore_backup_legacy_memories_with_passphrase(
    backend: &dyn MemoryBackend,
    content: &str,
    options: MemoryBackupRestoreOptions,
    passphrase: Option<&str>,
) -> Result<MemoryBackupRestoreResult> {
    let initial_preview = preview_backup_bundle_with_passphrase(backend, content, passphrase)?;
    let bundle = parse_supported_bundle_with_passphrase(content, passphrase)?;
    let external_payloads_by_memory_id = HashMap::new();
    restore_backup_legacy_memories_from_bundle(
        backend,
        &bundle,
        options,
        initial_preview,
        || preview_backup_bundle_with_passphrase(backend, content, passphrase),
        &external_payloads_by_memory_id,
    )
}

pub fn restore_backup_legacy_memories_from_archive(
    backend: &dyn MemoryBackend,
    archive_bytes: &[u8],
    options: MemoryBackupRestoreOptions,
) -> Result<MemoryBackupRestoreResult> {
    let parsed = parse_backup_archive(archive_bytes)?;
    let initial_preview = preview_backup_archive_from_parsed(backend, &parsed)?;
    restore_backup_legacy_memories_from_bundle(
        backend,
        &parsed.bundle,
        options,
        initial_preview,
        || preview_backup_archive_from_parsed(backend, &parsed),
        &parsed.external_payloads_by_memory_id,
    )
}

fn restore_backup_legacy_memories_from_bundle<F>(
    backend: &dyn MemoryBackend,
    bundle: &MemoryBackupBundle,
    options: MemoryBackupRestoreOptions,
    initial_preview: MemoryBackupImportPreview,
    final_preview: F,
    external_payloads_by_memory_id: &HashMap<i64, ArchiveAttachmentPayload>,
) -> Result<MemoryBackupRestoreResult>
where
    F: FnOnce() -> Result<MemoryBackupImportPreview>,
{
    if !initial_preview.valid {
        anyhow::bail!("memory backup is not valid; run preview and inspect issues first");
    }
    let current_memories = collect_all_memories(backend)?;
    let current_fingerprints: HashSet<String> =
        current_memories.iter().map(memory_fingerprint).collect();
    let mut bundle_fingerprints = HashSet::new();
    let mut skipped_exact_matches = 0usize;
    let mut skipped_duplicate_in_bundle = 0usize;
    let payload_by_memory_id: HashMap<i64, &MemoryBackupAttachmentPayload> = bundle
        .attachment_payloads
        .iter()
        .map(|payload| (payload.memory_id, payload))
        .collect();
    let chunk_payloads_by_memory_id =
        group_attachment_chunks_by_memory_id(&bundle.attachment_payload_chunks);
    let mut skipped_attachment_refs = 0usize;
    let mut restored_attachments = 0usize;
    let mut entries = Vec::new();

    for memory in &bundle.legacy_memories {
        let fingerprint = memory_fingerprint(memory);
        if !bundle_fingerprints.insert(fingerprint.clone()) {
            skipped_duplicate_in_bundle += 1;
            continue;
        }
        if current_fingerprints.contains(&fingerprint) {
            skipped_exact_matches += 1;
            continue;
        }
        let (attachment_path, attachment_mime) = match memory.attachment_path.as_ref() {
            Some(_) => {
                if let Some(payload) = payload_by_memory_id.get(&memory.id) {
                    match restore_attachment_payload(payload) {
                        Ok(path) => {
                            restored_attachments += 1;
                            (
                                Some(path.to_string_lossy().to_string()),
                                payload.mime.clone(),
                            )
                        }
                        Err(_) => {
                            skipped_attachment_refs += 1;
                            (None, None)
                        }
                    }
                } else if let Some(chunks) = chunk_payloads_by_memory_id.get(&memory.id) {
                    match restore_attachment_chunk_payloads(chunks) {
                        Ok((path, mime)) => {
                            restored_attachments += 1;
                            (Some(path.to_string_lossy().to_string()), mime)
                        }
                        Err(_) => {
                            skipped_attachment_refs += 1;
                            (None, None)
                        }
                    }
                } else if let Some(external_payload) =
                    external_payloads_by_memory_id.get(&memory.id)
                {
                    match restore_attachment_external_payload(external_payload) {
                        Ok((path, mime)) => {
                            restored_attachments += 1;
                            (Some(path.to_string_lossy().to_string()), mime)
                        }
                        Err(_) => {
                            skipped_attachment_refs += 1;
                            (None, None)
                        }
                    }
                } else {
                    skipped_attachment_refs += 1;
                    (None, None)
                }
            }
            None => (None, None),
        };
        entries.push(NewMemory {
            memory_type: memory.memory_type.clone(),
            scope: memory.scope.clone(),
            content: memory.content.clone(),
            tags: memory.tags.clone(),
            source: memory.source.clone(),
            source_session_id: memory.source_session_id.clone(),
            pinned: memory.pinned,
            attachment_path,
            attachment_mime,
        });
    }

    let attempted_legacy_memories = entries.len();
    let import_result = if attempted_legacy_memories > 0 {
        backend.import_entries(entries, options.dedup)?
    } else {
        ImportResult {
            created: 0,
            skipped_duplicate: 0,
            failed: 0,
            errors: Vec::new(),
        }
    };

    let current_memories_after_restore = collect_all_memories(backend)?;
    let (history_records, skipped_legacy_history_unmapped) =
        remap_legacy_history_records(bundle, &current_memories_after_restore);
    let restored_legacy_history = backend.import_history(&history_records)?;
    let final_preview = final_preview()?;

    Ok(MemoryBackupRestoreResult {
        preview: final_preview,
        import_result,
        attempted_legacy_memories,
        skipped_exact_matches,
        skipped_duplicate_in_bundle,
        skipped_attachment_refs,
        restored_attachments,
        restored_legacy_history,
        skipped_legacy_history_unmapped,
        preview_only_claims: bundle.claims.len(),
        preview_only_profile_snapshots: bundle.profile_snapshots.len(),
    })
}

/// Apply the structured subset of a backup bundle. This is intentionally
/// additive and conservative: it never overwrites existing claims/profile
/// snapshots, maps claim links by exact legacy-memory fingerprint, and downgrades
/// restored active claims with local conflicts into `needs_review`.
pub fn restore_backup_structured_memory(
    backend: &dyn MemoryBackend,
    content: &str,
    options: MemoryBackupStructuredRestoreOptions,
) -> Result<MemoryBackupStructuredRestoreResult> {
    restore_backup_structured_memory_with_passphrase(backend, content, options, None)
}

pub fn restore_backup_structured_memory_with_passphrase(
    backend: &dyn MemoryBackend,
    content: &str,
    options: MemoryBackupStructuredRestoreOptions,
    passphrase: Option<&str>,
) -> Result<MemoryBackupStructuredRestoreResult> {
    let initial_preview = preview_backup_bundle_with_passphrase(backend, content, passphrase)?;
    let bundle = parse_supported_bundle_with_passphrase(content, passphrase)?;
    restore_backup_structured_memory_from_bundle(backend, &bundle, options, initial_preview, || {
        preview_backup_bundle_with_passphrase(backend, content, passphrase)
    })
}

pub fn restore_backup_structured_memory_from_archive(
    backend: &dyn MemoryBackend,
    archive_bytes: &[u8],
    options: MemoryBackupStructuredRestoreOptions,
) -> Result<MemoryBackupStructuredRestoreResult> {
    let parsed = parse_backup_archive(archive_bytes)?;
    let initial_preview = preview_backup_archive_from_parsed(backend, &parsed)?;
    restore_backup_structured_memory_from_bundle(
        backend,
        &parsed.bundle,
        options,
        initial_preview,
        || preview_backup_archive_from_parsed(backend, &parsed),
    )
}

fn restore_backup_structured_memory_from_bundle<F>(
    backend: &dyn MemoryBackend,
    bundle: &MemoryBackupBundle,
    options: MemoryBackupStructuredRestoreOptions,
    initial_preview: MemoryBackupImportPreview,
    final_preview: F,
) -> Result<MemoryBackupStructuredRestoreResult>
where
    F: FnOnce() -> Result<MemoryBackupImportPreview>,
{
    if !initial_preview.valid {
        anyhow::bail!("memory backup is not valid; run preview and inspect issues first");
    }
    let current_memories = collect_all_memories(backend)?;
    let local_memory_id_by_backup_id =
        build_local_memory_id_map(&bundle.legacy_memories, &current_memories);

    let mut restored_claims = 0usize;
    let mut restored_claims_needing_review = 0usize;
    let mut skipped_claim_id_matches = 0usize;
    let mut skipped_claim_exact_matches = 0usize;
    let mut restored_evidence_rows = 0usize;
    let mut restored_claim_links = 0usize;
    let mut skipped_claim_links = 0usize;
    let mut failed_claims = 0usize;
    let mut restored_profile_snapshots = 0usize;
    let mut skipped_profile_exact_matches = 0usize;
    let mut skipped_profile_scope_conflicts = 0usize;
    let mut failed_profile_snapshots = 0usize;
    let mut restored_episodes = 0usize;
    let mut skipped_episode_id_matches = 0usize;
    let mut skipped_episode_exact_matches = 0usize;
    let mut failed_episodes = 0usize;
    let mut restored_procedures = 0usize;
    let mut skipped_procedure_id_matches = 0usize;
    let mut skipped_procedure_exact_matches = 0usize;
    let mut failed_procedures = 0usize;
    let mut restored_experience_history = 0usize;
    let mut skipped_experience_history_unmapped = 0usize;
    let mut errors = Vec::new();

    if options.restore_claims {
        let current_claims = collect_all_claim_records()?;
        let mut current_ids: HashSet<String> = current_claims
            .iter()
            .map(|claim| claim.id.clone())
            .collect();
        let mut current_fingerprints: HashSet<String> =
            current_claims.iter().map(claim_fingerprint).collect();
        let mut conflict_keys: HashSet<String> = current_claims
            .iter()
            .filter(|claim| claim_status_can_conflict(&claim.status))
            .map(claim_conflict_key)
            .collect();

        for detail in &bundle.claims {
            let claim = &detail.claim;
            if current_ids.contains(&claim.id) {
                skipped_claim_id_matches += 1;
                continue;
            }
            let fingerprint = claim_fingerprint(claim);
            if current_fingerprints.contains(&fingerprint) {
                skipped_claim_exact_matches += 1;
                continue;
            }

            let conflict_key = claim_conflict_key(claim);
            let conflicts_with_local =
                claim.status == "active" && conflict_keys.contains(&conflict_key);
            let status_override = conflicts_with_local.then_some("needs_review");
            match claims::restore_claim_detail(
                detail,
                &local_memory_id_by_backup_id,
                status_override,
            ) {
                Ok(outcome) => {
                    restored_claims += 1;
                    restored_evidence_rows += outcome.evidence_rows;
                    restored_claim_links += outcome.claim_links;
                    skipped_claim_links += outcome.skipped_claim_links;
                    current_ids.insert(claim.id.clone());
                    current_fingerprints.insert(fingerprint);
                    let final_status = status_override.unwrap_or(&claim.status);
                    if final_status == "needs_review" {
                        restored_claims_needing_review += 1;
                    }
                    if claim_status_can_conflict(final_status) {
                        conflict_keys.insert(conflict_key);
                    }
                }
                Err(e) => {
                    failed_claims += 1;
                    errors.push(format!("claim {} skipped: {e}", claim.id));
                }
            }
        }
    }

    if options.restore_profile_snapshots {
        let current_profiles = crate::memory::dreaming::list_profile_snapshots()?;
        let mut current_scope_keys: HashSet<String> =
            current_profiles.iter().map(profile_scope_key).collect();
        let mut current_profile_fingerprints: HashSet<String> =
            current_profiles.iter().map(profile_fingerprint).collect();

        for snapshot in &bundle.profile_snapshots {
            let fingerprint = profile_fingerprint(snapshot);
            if current_profile_fingerprints.contains(&fingerprint) {
                skipped_profile_exact_matches += 1;
                continue;
            }
            let scope_key = profile_scope_key(snapshot);
            if current_scope_keys.contains(&scope_key) && !options.allow_profile_scope_conflicts {
                skipped_profile_scope_conflicts += 1;
                continue;
            }
            match restore_profile_snapshot(snapshot) {
                Ok(()) => {
                    restored_profile_snapshots += 1;
                    current_scope_keys.insert(scope_key);
                    current_profile_fingerprints.insert(fingerprint);
                }
                Err(e) => {
                    failed_profile_snapshots += 1;
                    errors.push(format!(
                        "profile {} skipped: {e}",
                        profile_scope_label(snapshot)
                    ));
                }
            }
        }
    }

    if options.restore_episodes {
        let current = collect_all_episodes().unwrap_or_default();
        let mut current_ids: HashSet<String> =
            current.iter().map(|episode| episode.id.clone()).collect();
        let mut current_fingerprints: HashSet<String> =
            current.iter().map(episode_fingerprint).collect();

        for episode in &bundle.episodes {
            if current_ids.contains(&episode.id) {
                skipped_episode_id_matches += 1;
                continue;
            }
            let fingerprint = episode_fingerprint(episode);
            if current_fingerprints.contains(&fingerprint) {
                skipped_episode_exact_matches += 1;
                continue;
            }
            match crate::memory::episodes::restore_episode_record(episode) {
                Ok(true) => {
                    restored_episodes += 1;
                    current_ids.insert(episode.id.clone());
                    current_fingerprints.insert(fingerprint);
                }
                Ok(false) => {
                    skipped_episode_id_matches += 1;
                }
                Err(e) => {
                    failed_episodes += 1;
                    errors.push(format!("episode {} skipped: {e}", episode.id));
                }
            }
        }
    }

    if options.restore_procedures {
        let current = collect_all_procedures().unwrap_or_default();
        let mut current_ids: HashSet<String> = current
            .iter()
            .map(|procedure| procedure.id.clone())
            .collect();
        let mut current_fingerprints: HashSet<String> =
            current.iter().map(procedure_fingerprint).collect();

        for procedure in &bundle.procedures {
            if current_ids.contains(&procedure.id) {
                skipped_procedure_id_matches += 1;
                continue;
            }
            let fingerprint = procedure_fingerprint(procedure);
            if current_fingerprints.contains(&fingerprint) {
                skipped_procedure_exact_matches += 1;
                continue;
            }
            match crate::memory::episodes::restore_procedure_record(procedure) {
                Ok(true) => {
                    restored_procedures += 1;
                    current_ids.insert(procedure.id.clone());
                    current_fingerprints.insert(fingerprint);
                }
                Ok(false) => {
                    skipped_procedure_id_matches += 1;
                }
                Err(e) => {
                    failed_procedures += 1;
                    errors.push(format!("procedure {} skipped: {e}", procedure.id));
                }
            }
        }
    }

    if options.restore_experience_history {
        let current_episodes = collect_all_episodes().unwrap_or_default();
        let current_procedures = collect_all_procedures().unwrap_or_default();
        let (history_records, skipped) = remap_experience_history_records(
            bundle,
            &current_episodes,
            &current_procedures,
            options.restore_episodes,
            options.restore_procedures,
        );
        skipped_experience_history_unmapped = skipped;
        for record in history_records {
            match crate::memory::episodes::restore_experience_history_record(&record) {
                Ok(true) => {
                    restored_experience_history += 1;
                }
                Ok(false) => {}
                Err(e) => {
                    errors.push(format!("experience history {} skipped: {e}", record.id));
                }
            }
        }
    }

    let final_preview = final_preview()?;
    Ok(MemoryBackupStructuredRestoreResult {
        preview: final_preview,
        restored_claims,
        restored_claims_needing_review,
        skipped_claim_id_matches,
        skipped_claim_exact_matches,
        restored_evidence_rows,
        restored_claim_links,
        skipped_claim_links,
        failed_claims,
        restored_profile_snapshots,
        skipped_profile_exact_matches,
        skipped_profile_scope_conflicts,
        failed_profile_snapshots,
        restored_episodes,
        skipped_episode_id_matches,
        skipped_episode_exact_matches,
        failed_episodes,
        restored_procedures,
        skipped_procedure_id_matches,
        skipped_procedure_exact_matches,
        failed_procedures,
        restored_experience_history,
        skipped_experience_history_unmapped,
        errors,
    })
}

fn collect_all_memories(backend: &dyn MemoryBackend) -> Result<Vec<MemoryEntry>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = backend.list(None, None, MEMORY_PAGE_SIZE, offset)?;
        let page_len = page.len();
        out.extend(page);
        if page_len < MEMORY_PAGE_SIZE {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn collect_all_history(
    backend: &dyn MemoryBackend,
    warnings: &mut Vec<String>,
) -> Result<Vec<MemoryHistoryRecord>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = match backend.history(MEMORY_HISTORY_PAGE_SIZE, offset) {
            Ok(page) => page,
            Err(e) => {
                warnings.push(format!("legacy memory history export skipped: {e}"));
                return Ok(Vec::new());
            }
        };
        let page_len = page.len();
        out.extend(page);
        if page_len < MEMORY_HISTORY_PAGE_SIZE {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn collect_all_episodes() -> Result<Vec<MemoryEpisodeRecord>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = crate::memory::episodes::list_episodes_page(MemoryEpisodeQuery {
            limit: Some(MEMORY_PAGE_SIZE.min(100)),
            offset: Some(offset),
            ..Default::default()
        })?;
        let page_len = page.items.len();
        out.extend(page.items);
        if page_len < MEMORY_PAGE_SIZE.min(100) {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn collect_all_procedures() -> Result<Vec<MemoryProcedureRecord>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = crate::memory::episodes::list_procedures_page(MemoryProcedureQuery {
            limit: Some(MEMORY_PAGE_SIZE.min(100)),
            offset: Some(offset),
            ..Default::default()
        })?;
        let page_len = page.items.len();
        out.extend(page.items);
        if page_len < MEMORY_PAGE_SIZE.min(100) {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn collect_all_experience_history() -> Result<Vec<MemoryExperienceHistoryRecord>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page =
            crate::memory::episodes::list_experience_history_page(MemoryExperienceHistoryQuery {
                limit: Some(MEMORY_PAGE_SIZE.min(100)),
                offset: Some(offset),
                ..Default::default()
            })?;
        let page_len = page.items.len();
        out.extend(page.items);
        if page_len < MEMORY_PAGE_SIZE.min(100) {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn parse_supported_bundle_with_passphrase(
    content: &str,
    passphrase: Option<&str>,
) -> Result<MemoryBackupBundle> {
    let parsed: serde_json::Value = serde_json::from_str(content)?;
    let schema = parsed.get("schemaVersion").and_then(|v| v.as_str());
    if schema == Some(MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION) {
        let passphrase =
            passphrase.ok_or_else(|| anyhow!("encrypted backup requires passphrase"))?;
        let plaintext = decrypt_encrypted_backup_value(parsed, passphrase)?;
        return Ok(serde_json::from_slice(&plaintext)?);
    }
    if schema != Some(MEMORY_BACKUP_SCHEMA_VERSION) {
        anyhow::bail!(
            "unsupported memory backup schema: {}",
            schema.unwrap_or("<missing schemaVersion>")
        );
    }
    Ok(serde_json::from_value(parsed)?)
}

fn validate_backup_export_passphrase(passphrase: &str) -> Result<()> {
    let char_count = passphrase.chars().count();
    if char_count < 12 {
        anyhow::bail!("backup passphrase must be at least 12 characters");
    }
    let compact = passphrase
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let weak_patterns = [
        "password",
        "passphrase",
        "qwerty",
        "123456",
        "hopeagent",
        "memorybackup",
    ];
    if weak_patterns
        .iter()
        .any(|pattern| compact.contains(pattern))
    {
        anyhow::bail!("backup passphrase contains a common weak pattern");
    }

    let unique_chars = passphrase.chars().collect::<HashSet<_>>().len();
    if unique_chars < 6 {
        anyhow::bail!("backup passphrase is too repetitive");
    }
    let has_lower = passphrase.chars().any(|ch| ch.is_ascii_lowercase());
    let has_upper = passphrase.chars().any(|ch| ch.is_ascii_uppercase());
    let has_digit = passphrase.chars().any(|ch| ch.is_ascii_digit());
    let has_symbol = passphrase
        .chars()
        .any(|ch| !ch.is_ascii_alphanumeric() && !ch.is_whitespace());
    let has_space = passphrase.chars().any(char::is_whitespace);
    let character_classes = [has_lower, has_upper, has_digit, has_symbol || has_space]
        .into_iter()
        .filter(|present| *present)
        .count();
    let word_count = passphrase
        .split_whitespace()
        .filter(|word| word.chars().count() >= 3)
        .count();
    let long_phrase = char_count >= 24 && word_count >= 4;
    let long_mixed = char_count >= 16 && character_classes >= 2 && unique_chars >= 8;
    let mixed = char_count >= 12 && character_classes >= 3 && unique_chars >= 8;
    if !(long_phrase || long_mixed || mixed) {
        anyhow::bail!("backup passphrase needs more variety, or a longer four-word phrase");
    }
    Ok(())
}

fn validate_backup_unlock_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.is_empty() {
        anyhow::bail!("encrypted backup requires passphrase");
    }
    Ok(())
}

fn encrypt_backup_plaintext(
    plaintext: &[u8],
    passphrase: &str,
    exported_at: String,
    app_version: String,
) -> Result<MemoryEncryptedBackupBundle> {
    let mut salt = [0u8; ENCRYPTED_BACKUP_SALT_BYTES];
    let mut nonce = [0u8; ENCRYPTED_BACKUP_NONCE_BYTES];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce);
    let key_material = derive_backup_keys(passphrase, &salt, ENCRYPTED_BACKUP_KDF_ITERATIONS)?;
    let mut enc_key = [0u8; 32];
    let mut mac_key = [0u8; 32];
    enc_key.copy_from_slice(&key_material[..32]);
    mac_key.copy_from_slice(&key_material[32..64]);
    let ciphertext = aes256_ctr_xor(&enc_key, &nonce, plaintext);
    let aad = encrypted_backup_aad(
        MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION,
        MEMORY_BACKUP_SCHEMA_VERSION,
        &exported_at,
        &app_version,
        ENCRYPTED_BACKUP_KDF_NAME,
        ENCRYPTED_BACKUP_KDF_ITERATIONS,
        &salt,
        ENCRYPTED_BACKUP_CIPHER_NAME,
        &nonce,
    );
    let mac = hmac_sha256(&mac_key, &[aad.as_slice(), ciphertext.as_slice()].concat())?;
    Ok(MemoryEncryptedBackupBundle {
        schema_version: MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION.to_string(),
        exported_at,
        app_version,
        plaintext_schema_version: MEMORY_BACKUP_SCHEMA_VERSION.to_string(),
        kdf: MemoryBackupKdfManifest {
            name: ENCRYPTED_BACKUP_KDF_NAME.to_string(),
            iterations: ENCRYPTED_BACKUP_KDF_ITERATIONS,
            salt_base64: base64::engine::general_purpose::STANDARD.encode(salt),
        },
        cipher: MemoryBackupCipherManifest {
            name: ENCRYPTED_BACKUP_CIPHER_NAME.to_string(),
            nonce_base64: base64::engine::general_purpose::STANDARD.encode(nonce),
            ciphertext_base64: base64::engine::general_purpose::STANDARD.encode(ciphertext),
            mac_base64: base64::engine::general_purpose::STANDARD.encode(mac),
        },
    })
}

fn decrypt_encrypted_backup_value(parsed: serde_json::Value, passphrase: &str) -> Result<Vec<u8>> {
    validate_backup_unlock_passphrase(passphrase)?;
    let envelope: MemoryEncryptedBackupBundle = serde_json::from_value(parsed)?;
    if envelope.schema_version != MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION {
        anyhow::bail!("unsupported encrypted backup schema");
    }
    if envelope.plaintext_schema_version != MEMORY_BACKUP_SCHEMA_VERSION {
        anyhow::bail!("unsupported encrypted backup plaintext schema");
    }
    if envelope.kdf.name != ENCRYPTED_BACKUP_KDF_NAME {
        anyhow::bail!("unsupported encrypted backup KDF");
    }
    if envelope.kdf.iterations == 0 {
        anyhow::bail!("invalid encrypted backup KDF iterations");
    }
    if envelope.cipher.name != ENCRYPTED_BACKUP_CIPHER_NAME {
        anyhow::bail!("unsupported encrypted backup cipher");
    }
    let salt = base64::engine::general_purpose::STANDARD
        .decode(&envelope.kdf.salt_base64)
        .context("decoding encrypted backup salt")?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&envelope.cipher.nonce_base64)
        .context("decoding encrypted backup nonce")?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&envelope.cipher.ciphertext_base64)
        .context("decoding encrypted backup ciphertext")?;
    let expected_mac = base64::engine::general_purpose::STANDARD
        .decode(&envelope.cipher.mac_base64)
        .context("decoding encrypted backup MAC")?;
    if salt.len() != ENCRYPTED_BACKUP_SALT_BYTES || nonce.len() != ENCRYPTED_BACKUP_NONCE_BYTES {
        anyhow::bail!("encrypted backup salt or nonce has invalid length");
    }
    let key_material = derive_backup_keys(passphrase, &salt, envelope.kdf.iterations)?;
    let mut enc_key = [0u8; 32];
    let mut mac_key = [0u8; 32];
    enc_key.copy_from_slice(&key_material[..32]);
    mac_key.copy_from_slice(&key_material[32..64]);
    let aad = encrypted_backup_aad(
        &envelope.schema_version,
        &envelope.plaintext_schema_version,
        &envelope.exported_at,
        &envelope.app_version,
        &envelope.kdf.name,
        envelope.kdf.iterations,
        &salt,
        &envelope.cipher.name,
        &nonce,
    );
    let actual_mac = hmac_sha256(&mac_key, &[aad.as_slice(), ciphertext.as_slice()].concat())?;
    if !constant_time_eq(&actual_mac, &expected_mac) {
        anyhow::bail!("bad passphrase or corrupted encrypted backup");
    }
    let mut nonce_arr = [0u8; ENCRYPTED_BACKUP_NONCE_BYTES];
    nonce_arr.copy_from_slice(&nonce);
    Ok(aes256_ctr_xor(&enc_key, &nonce_arr, &ciphertext))
}

fn derive_backup_keys(passphrase: &str, salt: &[u8], iterations: u32) -> Result<[u8; 64]> {
    if iterations == 0 {
        anyhow::bail!("PBKDF2 iterations must be greater than zero");
    }
    let mut out = [0u8; 64];
    pbkdf2_hmac_sha256(passphrase.as_bytes(), salt, iterations, &mut out)?;
    Ok(out)
}

fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) -> Result<()> {
    let blocks = out.len().div_ceil(32);
    for block_idx in 1..=blocks {
        let mut salt_block = Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&(block_idx as u32).to_be_bytes());
        let mut u = hmac_sha256(password, &salt_block)?;
        let mut t = u;
        for _ in 1..iterations {
            u = hmac_sha256(password, &u)?;
            for (dst, src) in t.iter_mut().zip(u.iter()) {
                *dst ^= *src;
            }
        }
        let start = (block_idx - 1) * 32;
        let end = (start + 32).min(out.len());
        out[start..end].copy_from_slice(&t[..end - start]);
    }
    Ok(())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<[u8; 32]> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key).map_err(|_| anyhow!("invalid HMAC key length"))?;
    mac.update(data);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn encrypted_backup_aad(
    schema_version: &str,
    plaintext_schema_version: &str,
    exported_at: &str,
    app_version: &str,
    kdf_name: &str,
    kdf_iterations: u32,
    salt: &[u8],
    cipher_name: &str,
    nonce: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    for part in [
        schema_version.as_bytes(),
        plaintext_schema_version.as_bytes(),
        exported_at.as_bytes(),
        app_version.as_bytes(),
        kdf_name.as_bytes(),
        cipher_name.as_bytes(),
    ] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part);
    }
    out.extend_from_slice(&kdf_iterations.to_be_bytes());
    out.extend_from_slice(&(salt.len() as u32).to_be_bytes());
    out.extend_from_slice(salt);
    out.extend_from_slice(&(nonce.len() as u32).to_be_bytes());
    out.extend_from_slice(nonce);
    out
}

fn aes256_ctr_xor(key: &[u8; 32], nonce: &[u8; 16], input: &[u8]) -> Vec<u8> {
    let cipher = aes::Aes256::new(GenericArray::from_slice(key));
    let mut counter = u128::from_be_bytes(*nonce);
    let mut out = Vec::with_capacity(input.len());
    for chunk in input.chunks(16) {
        let mut block = GenericArray::clone_from_slice(&counter.to_be_bytes());
        cipher.encrypt_block(&mut block);
        out.extend(chunk.iter().zip(block.iter()).map(|(a, b)| a ^ b));
        counter = counter.wrapping_add(1);
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn collect_attachment_payloads(
    memories: &[MemoryEntry],
    warnings: &mut Vec<String>,
) -> (
    Vec<MemoryBackupAttachmentPayload>,
    Vec<MemoryBackupAttachmentChunkPayload>,
    Vec<MemoryBackupAttachmentExternalPayload>,
) {
    let mut inline_payloads = Vec::new();
    let mut chunk_payloads = Vec::new();
    let mut external_payloads = Vec::new();
    for memory in memories {
        let Some(path) = memory.attachment_path.as_deref() else {
            continue;
        };
        let path_buf = PathBuf::from(path);
        let meta = match std::fs::metadata(&path_buf) {
            Ok(meta) => meta,
            Err(e) => {
                warnings.push(format!(
                    "attachment for memory {} could not be read: {}",
                    memory.id, e
                ));
                continue;
            }
        };
        if !meta.is_file() {
            warnings.push(format!(
                "attachment for memory {} is not a regular file",
                memory.id
            ));
            continue;
        }
        if meta.len() > MAX_CHUNKED_ATTACHMENT_BYTES {
            match sha256_file_hex(&path_buf) {
                Ok(sha256) => {
                    external_payloads.push(MemoryBackupAttachmentExternalPayload {
                        memory_id: memory.id,
                        original_path: path.to_string(),
                        mime: memory.attachment_mime.clone(),
                        size_bytes: meta.len(),
                        sha256,
                        sidecar_file_name: attachment_sidecar_file_name(memory.id, path),
                        reason: "exceeds_inline_bundle_cap".to_string(),
                    });
                    warnings.push(format!(
                        "attachment for memory {} requires external sidecar payload ({} bytes; JSON payload cap is {} bytes)",
                        memory.id,
                        meta.len(),
                        MAX_CHUNKED_ATTACHMENT_BYTES
                    ));
                }
                Err(e) => warnings.push(format!(
                    "attachment for memory {} is too large to pack and could not be hashed for sidecar metadata: {}",
                    memory.id, e
                )),
            }
            continue;
        }
        let bytes = match std::fs::read(&path_buf) {
            Ok(bytes) => bytes,
            Err(e) => {
                warnings.push(format!(
                    "attachment for memory {} could not be packed: {}",
                    memory.id, e
                ));
                continue;
            }
        };
        let size_bytes = bytes.len() as u64;
        if size_bytes > MAX_CHUNKED_ATTACHMENT_BYTES {
            warnings.push(format!(
                "attachment for memory {} grew beyond the chunked payload cap while packing ({} bytes; cap is {} bytes)",
                memory.id, size_bytes, MAX_CHUNKED_ATTACHMENT_BYTES
            ));
            continue;
        }
        if size_bytes <= MAX_ATTACHMENT_PAYLOAD_BYTES {
            inline_payloads.push(MemoryBackupAttachmentPayload {
                memory_id: memory.id,
                original_path: path.to_string(),
                mime: memory.attachment_mime.clone(),
                size_bytes,
                sha256: sha256_hex(&bytes),
                base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
            continue;
        }

        let sha256 = sha256_hex(&bytes);
        let chunk_count = bytes.chunks(ATTACHMENT_CHUNK_BYTES).count();
        for (chunk_index, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
            chunk_payloads.push(MemoryBackupAttachmentChunkPayload {
                memory_id: memory.id,
                original_path: path.to_string(),
                mime: memory.attachment_mime.clone(),
                size_bytes,
                sha256: sha256.clone(),
                chunk_index,
                chunk_count,
                chunk_size_bytes: chunk.len() as u64,
                chunk_sha256: sha256_hex(chunk),
                base64: base64::engine::general_purpose::STANDARD.encode(chunk),
            });
        }
    }
    (inline_payloads, chunk_payloads, external_payloads)
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn attachment_sidecar_file_name(memory_id: i64, original_path: &str) -> String {
    let ext = Path::new(original_path)
        .extension()
        .and_then(|s| s.to_str())
        .map(sanitize_extension)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "bin".to_string());
    format!("memory-{memory_id}-attachment.{ext}")
}

struct ArchiveSidecarSource {
    source_path: PathBuf,
    archive_name: String,
}

struct ArchiveAttachmentPayload {
    metadata: MemoryBackupAttachmentExternalPayload,
    bytes: Vec<u8>,
}

struct ParsedBackupArchive {
    bundle: MemoryBackupBundle,
    external_payloads_by_memory_id: HashMap<i64, ArchiveAttachmentPayload>,
    sidecar_issues: Vec<MemoryBackupPreviewIssue>,
}

fn collect_archive_sidecar_sources(bundle: &MemoryBackupBundle) -> Vec<ArchiveSidecarSource> {
    bundle
        .attachment_external_payloads
        .iter()
        .map(|payload| ArchiveSidecarSource {
            source_path: PathBuf::from(&payload.original_path),
            archive_name: archive_sidecar_archive_name(payload),
        })
        .collect()
}

fn parse_backup_archive(archive_bytes: &[u8]) -> Result<ParsedBackupArchive> {
    let cursor = std::io::Cursor::new(archive_bytes);
    let mut archive = ZipArchive::new(cursor).context("opening ZIP archive")?;
    let mut manifest_json = String::new();
    {
        let mut manifest = archive
            .by_name("memory-backup.json")
            .context("memory-backup.json not found in archive")?;
        manifest
            .read_to_string(&mut manifest_json)
            .context("reading memory-backup.json")?;
    }
    let bundle = parse_supported_bundle_with_passphrase(&manifest_json, None)
        .context("parsing memory-backup.json")?;
    let mut sidecar_issues = Vec::new();
    let mut external_payloads_by_memory_id = HashMap::new();
    for payload in &bundle.attachment_external_payloads {
        let archive_name = archive_sidecar_archive_name(payload);
        let mut sidecar = match archive.by_name(&archive_name) {
            Ok(sidecar) => sidecar,
            Err(_) => {
                sidecar_issues.push(MemoryBackupPreviewIssue {
                    severity: MemoryHealthSeverity::Warning,
                    code: "attachment_sidecar_missing".to_string(),
                    message: format!("Attachment sidecar is missing: {archive_name}"),
                });
                continue;
            }
        };
        if sidecar.size() != payload.size_bytes {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_size_mismatch".to_string(),
                message: format!(
                    "Attachment sidecar {} has size {}, expected {}",
                    archive_name,
                    sidecar.size(),
                    payload.size_bytes
                ),
            });
            continue;
        }
        if payload.size_bytes > MAX_ARCHIVE_ATTACHMENT_SIDECAR_BYTES {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_too_large".to_string(),
                message: format!(
                    "Attachment sidecar {} exceeds restore cap ({} bytes)",
                    archive_name, MAX_ARCHIVE_ATTACHMENT_SIDECAR_BYTES
                ),
            });
            continue;
        }
        let mut bytes = Vec::with_capacity(payload.size_bytes as usize);
        if let Err(e) = sidecar.read_to_end(&mut bytes) {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_read_failed".to_string(),
                message: format!("Attachment sidecar {archive_name} could not be read: {e}"),
            });
            continue;
        }
        if bytes.len() as u64 != payload.size_bytes {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_size_mismatch".to_string(),
                message: format!(
                    "Attachment sidecar {} decoded to size {}, expected {}",
                    archive_name,
                    bytes.len(),
                    payload.size_bytes
                ),
            });
            continue;
        }
        let actual_sha = sha256_hex(&bytes);
        if actual_sha != payload.sha256 {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_checksum_mismatch".to_string(),
                message: format!("Attachment sidecar {archive_name} checksum does not match"),
            });
            continue;
        }
        if external_payloads_by_memory_id
            .insert(
                payload.memory_id,
                ArchiveAttachmentPayload {
                    metadata: payload.clone(),
                    bytes,
                },
            )
            .is_some()
        {
            sidecar_issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "attachment_sidecar_duplicate_memory_id".to_string(),
                message: format!(
                    "Multiple sidecars target memory {}; keeping the last verified payload",
                    payload.memory_id
                ),
            });
        }
    }
    Ok(ParsedBackupArchive {
        bundle,
        external_payloads_by_memory_id,
        sidecar_issues,
    })
}

fn archive_sidecar_archive_name(payload: &MemoryBackupAttachmentExternalPayload) -> String {
    format!(
        "attachments/{}",
        sanitize_archive_file_name(&payload.sidecar_file_name)
    )
}

fn sanitize_archive_file_name(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches('.').trim_matches('_');
    if cleaned.is_empty() {
        "attachment.bin".to_string()
    } else {
        cleaned.chars().take(128).collect()
    }
}

fn unique_chunk_memory_count(chunks: &[MemoryBackupAttachmentChunkPayload]) -> usize {
    chunks
        .iter()
        .map(|chunk| chunk.memory_id)
        .collect::<HashSet<_>>()
        .len()
}

fn chunked_payload_total_bytes(chunks: &[MemoryBackupAttachmentChunkPayload]) -> u64 {
    let mut seen_memory_ids = HashSet::new();
    chunks
        .iter()
        .filter_map(|chunk| {
            seen_memory_ids
                .insert(chunk.memory_id)
                .then_some(chunk.size_bytes)
        })
        .sum()
}

fn group_attachment_chunks_by_memory_id(
    chunks: &[MemoryBackupAttachmentChunkPayload],
) -> HashMap<i64, Vec<&MemoryBackupAttachmentChunkPayload>> {
    let mut out: HashMap<i64, Vec<&MemoryBackupAttachmentChunkPayload>> = HashMap::new();
    for chunk in chunks {
        out.entry(chunk.memory_id).or_default().push(chunk);
    }
    out
}

fn build_local_memory_id_map(
    bundle_memories: &[MemoryEntry],
    current_memories: &[MemoryEntry],
) -> HashMap<i64, i64> {
    let mut local_ids_by_fingerprint: HashMap<String, Vec<i64>> = HashMap::new();
    for memory in current_memories {
        local_ids_by_fingerprint
            .entry(memory_fingerprint(memory))
            .or_default()
            .push(memory.id);
    }
    let mut out = HashMap::new();
    for memory in bundle_memories {
        let fingerprint = memory_fingerprint(memory);
        let Some(local_ids) = local_ids_by_fingerprint.get(&fingerprint) else {
            continue;
        };
        if local_ids.len() == 1 {
            out.insert(memory.id, local_ids[0]);
        }
    }
    out
}

fn build_history_memory_id_map(
    bundle_memories: &[MemoryEntry],
    current_memories: &[MemoryEntry],
    include_projected_missing: bool,
) -> HashMap<i64, i64> {
    let mut bundle_fingerprint_counts: HashMap<String, usize> = HashMap::new();
    for memory in bundle_memories {
        *bundle_fingerprint_counts
            .entry(memory_fingerprint(memory))
            .or_default() += 1;
    }

    let mut local_ids_by_fingerprint: HashMap<String, Vec<i64>> = HashMap::new();
    for memory in current_memories {
        local_ids_by_fingerprint
            .entry(memory_fingerprint(memory))
            .or_default()
            .push(memory.id);
    }

    let mut out = HashMap::new();
    for memory in bundle_memories {
        let fingerprint = memory_fingerprint(memory);
        if bundle_fingerprint_counts
            .get(&fingerprint)
            .copied()
            .unwrap_or(0)
            != 1
        {
            continue;
        }
        match local_ids_by_fingerprint
            .get(&fingerprint)
            .map(Vec::as_slice)
        {
            Some([local_id]) => {
                out.insert(memory.id, *local_id);
            }
            Some(_) => {}
            None if include_projected_missing => {
                out.insert(memory.id, memory.id);
            }
            None => {}
        }
    }
    out
}

fn legacy_history_restore_counts(
    bundle: &MemoryBackupBundle,
    current_memories: &[MemoryEntry],
    include_projected_missing: bool,
) -> (usize, usize) {
    let memory_id_map = build_history_memory_id_map(
        &bundle.legacy_memories,
        current_memories,
        include_projected_missing,
    );
    let mut restorable = 0usize;
    let mut skipped = 0usize;
    for record in &bundle.legacy_history {
        if memory_id_map.contains_key(&record.memory_id) {
            restorable += 1;
        } else {
            skipped += 1;
        }
    }
    (restorable, skipped)
}

fn remap_legacy_history_records(
    bundle: &MemoryBackupBundle,
    current_memories: &[MemoryEntry],
) -> (Vec<MemoryHistoryRecord>, usize) {
    let memory_id_map =
        build_history_memory_id_map(&bundle.legacy_memories, current_memories, false);
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for record in &bundle.legacy_history {
        let Some(local_memory_id) = memory_id_map.get(&record.memory_id) else {
            skipped += 1;
            continue;
        };
        let mut mapped = record.clone();
        mapped.memory_id = *local_memory_id;
        records.push(mapped);
    }
    (records, skipped)
}

fn build_episode_history_id_map(
    bundle_episodes: &[MemoryEpisodeRecord],
    current_episodes: &[MemoryEpisodeRecord],
    include_projected_missing: bool,
) -> HashMap<String, String> {
    let mut current_ids = HashSet::new();
    let mut local_ids_by_fingerprint: HashMap<String, Vec<String>> = HashMap::new();
    for episode in current_episodes {
        current_ids.insert(episode.id.clone());
        local_ids_by_fingerprint
            .entry(episode_fingerprint(episode))
            .or_default()
            .push(episode.id.clone());
    }
    let mut bundle_fingerprint_counts: HashMap<String, usize> = HashMap::new();
    for episode in bundle_episodes {
        *bundle_fingerprint_counts
            .entry(episode_fingerprint(episode))
            .or_default() += 1;
    }

    let mut out = HashMap::new();
    for episode in bundle_episodes {
        if current_ids.contains(&episode.id) {
            out.insert(episode.id.clone(), episode.id.clone());
            continue;
        }
        let fingerprint = episode_fingerprint(episode);
        if bundle_fingerprint_counts
            .get(&fingerprint)
            .copied()
            .unwrap_or(0)
            != 1
        {
            continue;
        }
        match local_ids_by_fingerprint
            .get(&fingerprint)
            .map(Vec::as_slice)
        {
            Some([local_id]) => {
                out.insert(episode.id.clone(), local_id.clone());
            }
            Some(_) => {}
            None if include_projected_missing => {
                out.insert(episode.id.clone(), episode.id.clone());
            }
            None => {}
        }
    }
    out
}

fn build_procedure_history_id_map(
    bundle_procedures: &[MemoryProcedureRecord],
    current_procedures: &[MemoryProcedureRecord],
    include_projected_missing: bool,
) -> HashMap<String, String> {
    let mut current_ids = HashSet::new();
    let mut local_ids_by_fingerprint: HashMap<String, Vec<String>> = HashMap::new();
    for procedure in current_procedures {
        current_ids.insert(procedure.id.clone());
        local_ids_by_fingerprint
            .entry(procedure_fingerprint(procedure))
            .or_default()
            .push(procedure.id.clone());
    }
    let mut bundle_fingerprint_counts: HashMap<String, usize> = HashMap::new();
    for procedure in bundle_procedures {
        *bundle_fingerprint_counts
            .entry(procedure_fingerprint(procedure))
            .or_default() += 1;
    }

    let mut out = HashMap::new();
    for procedure in bundle_procedures {
        if current_ids.contains(&procedure.id) {
            out.insert(procedure.id.clone(), procedure.id.clone());
            continue;
        }
        let fingerprint = procedure_fingerprint(procedure);
        if bundle_fingerprint_counts
            .get(&fingerprint)
            .copied()
            .unwrap_or(0)
            != 1
        {
            continue;
        }
        match local_ids_by_fingerprint
            .get(&fingerprint)
            .map(Vec::as_slice)
        {
            Some([local_id]) => {
                out.insert(procedure.id.clone(), local_id.clone());
            }
            Some(_) => {}
            None if include_projected_missing => {
                out.insert(procedure.id.clone(), procedure.id.clone());
            }
            None => {}
        }
    }
    out
}

fn experience_history_restore_counts(
    bundle: &MemoryBackupBundle,
    issues: &mut Vec<MemoryBackupPreviewIssue>,
    include_projected_missing: bool,
) -> (usize, usize) {
    let current_episodes = match collect_all_episodes() {
        Ok(records) => records,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_episodes_unavailable".to_string(),
                message: format!("Current episodes could not be compared for history: {e}"),
            });
            Vec::new()
        }
    };
    let current_procedures = match collect_all_procedures() {
        Ok(records) => records,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_procedures_unavailable".to_string(),
                message: format!("Current procedures could not be compared for history: {e}"),
            });
            Vec::new()
        }
    };
    let episode_id_map = build_episode_history_id_map(
        &bundle.episodes,
        &current_episodes,
        include_projected_missing,
    );
    let procedure_id_map = build_procedure_history_id_map(
        &bundle.procedures,
        &current_procedures,
        include_projected_missing,
    );
    let mut restorable = 0usize;
    let mut skipped = 0usize;
    for record in &bundle.experience_history {
        let map = if record.target_kind.trim() == "procedure" {
            &procedure_id_map
        } else {
            &episode_id_map
        };
        if map.contains_key(&record.target_id) {
            restorable += 1;
        } else {
            skipped += 1;
        }
    }
    (restorable, skipped)
}

fn remap_experience_history_records(
    bundle: &MemoryBackupBundle,
    current_episodes: &[MemoryEpisodeRecord],
    current_procedures: &[MemoryProcedureRecord],
    restore_episodes: bool,
    restore_procedures: bool,
) -> (Vec<MemoryExperienceHistoryRecord>, usize) {
    let episode_id_map = build_episode_history_id_map(&bundle.episodes, current_episodes, false);
    let procedure_id_map =
        build_procedure_history_id_map(&bundle.procedures, current_procedures, false);
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for record in &bundle.experience_history {
        let is_procedure = record.target_kind.trim() == "procedure";
        if is_procedure && !restore_procedures {
            skipped += 1;
            continue;
        }
        if !is_procedure && !restore_episodes {
            skipped += 1;
            continue;
        }
        let target_map = if is_procedure {
            &procedure_id_map
        } else {
            &episode_id_map
        };
        let Some(local_target_id) = target_map.get(&record.target_id) else {
            skipped += 1;
            continue;
        };
        let mut mapped = record.clone();
        mapped.target_kind = if is_procedure {
            "procedure".to_string()
        } else {
            "episode".to_string()
        };
        mapped.target_id = local_target_id.clone();
        records.push(mapped);
    }
    (records, skipped)
}

fn restore_attachment_payload(payload: &MemoryBackupAttachmentPayload) -> Result<PathBuf> {
    let dir = crate::paths::memory_attachments_dir()?;
    restore_attachment_payload_to_dir(payload, &dir)
}

fn restore_attachment_chunk_payloads(
    chunks: &[&MemoryBackupAttachmentChunkPayload],
) -> Result<(PathBuf, Option<String>)> {
    let dir = crate::paths::memory_attachments_dir()?;
    let path = restore_attachment_chunk_payloads_to_dir(chunks, &dir)?;
    let mime = chunks.first().and_then(|chunk| chunk.mime.clone());
    Ok((path, mime))
}

fn restore_attachment_external_payload(
    payload: &ArchiveAttachmentPayload,
) -> Result<(PathBuf, Option<String>)> {
    let dir = crate::paths::memory_attachments_dir()?;
    let path = restore_attachment_bytes_to_dir(
        &payload.metadata.original_path,
        payload.metadata.mime.as_deref(),
        &payload.bytes,
        payload.metadata.size_bytes,
        &payload.metadata.sha256,
        MAX_ARCHIVE_ATTACHMENT_SIDECAR_BYTES,
        &dir,
    )?;
    Ok((path, payload.metadata.mime.clone()))
}

fn restore_attachment_payload_to_dir(
    payload: &MemoryBackupAttachmentPayload,
    dir: &Path,
) -> Result<PathBuf> {
    if payload.size_bytes > MAX_ATTACHMENT_PAYLOAD_BYTES {
        return Err(anyhow!("attachment payload exceeds size cap"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&payload.base64)
        .context("decoding attachment payload")?;
    if bytes.len() as u64 != payload.size_bytes {
        return Err(anyhow!(
            "attachment payload size mismatch: expected {}, got {}",
            payload.size_bytes,
            bytes.len()
        ));
    }
    let actual_sha = sha256_hex(&bytes);
    if actual_sha != payload.sha256 {
        return Err(anyhow!("attachment payload checksum mismatch"));
    }
    restore_attachment_bytes_to_dir(
        &payload.original_path,
        payload.mime.as_deref(),
        &bytes,
        payload.size_bytes,
        &payload.sha256,
        MAX_ATTACHMENT_PAYLOAD_BYTES,
        dir,
    )
}

fn restore_attachment_chunk_payloads_to_dir(
    chunks: &[&MemoryBackupAttachmentChunkPayload],
    dir: &Path,
) -> Result<PathBuf> {
    let Some(first) = chunks.first().copied() else {
        return Err(anyhow!("attachment chunk payload is empty"));
    };
    if first.size_bytes == 0 || first.size_bytes > MAX_CHUNKED_ATTACHMENT_BYTES {
        return Err(anyhow!("attachment chunked payload exceeds size cap"));
    }
    if first.chunk_count == 0 {
        return Err(anyhow!("attachment chunked payload has no chunks"));
    }
    let expected_chunk_count =
        (first.size_bytes as usize + ATTACHMENT_CHUNK_BYTES - 1) / ATTACHMENT_CHUNK_BYTES;
    if first.chunk_count != expected_chunk_count {
        return Err(anyhow!(
            "attachment chunk count mismatch: expected {}, got {}",
            expected_chunk_count,
            first.chunk_count
        ));
    }
    if chunks.len() != first.chunk_count {
        return Err(anyhow!(
            "attachment chunk payload is incomplete: expected {}, got {}",
            first.chunk_count,
            chunks.len()
        ));
    }

    let mut ordered = chunks.to_vec();
    ordered.sort_by_key(|chunk| chunk.chunk_index);
    let mut bytes = Vec::with_capacity(first.size_bytes as usize);
    for (expected_index, chunk) in ordered.iter().enumerate() {
        if chunk.memory_id != first.memory_id
            || chunk.original_path != first.original_path
            || chunk.mime.as_deref() != first.mime.as_deref()
            || chunk.size_bytes != first.size_bytes
            || chunk.sha256 != first.sha256
            || chunk.chunk_count != first.chunk_count
        {
            return Err(anyhow!("attachment chunk payload metadata mismatch"));
        }
        if chunk.chunk_index != expected_index {
            return Err(anyhow!(
                "attachment chunk index mismatch: expected {}, got {}",
                expected_index,
                chunk.chunk_index
            ));
        }
        if chunk.chunk_size_bytes > ATTACHMENT_CHUNK_BYTES as u64 {
            return Err(anyhow!("attachment chunk exceeds per-chunk cap"));
        }
        let chunk_bytes = base64::engine::general_purpose::STANDARD
            .decode(&chunk.base64)
            .context("decoding attachment chunk payload")?;
        if chunk_bytes.len() as u64 != chunk.chunk_size_bytes {
            return Err(anyhow!(
                "attachment chunk size mismatch: expected {}, got {}",
                chunk.chunk_size_bytes,
                chunk_bytes.len()
            ));
        }
        if sha256_hex(&chunk_bytes) != chunk.chunk_sha256 {
            return Err(anyhow!("attachment chunk checksum mismatch"));
        }
        bytes.extend_from_slice(&chunk_bytes);
    }

    restore_attachment_bytes_to_dir(
        &first.original_path,
        first.mime.as_deref(),
        &bytes,
        first.size_bytes,
        &first.sha256,
        MAX_CHUNKED_ATTACHMENT_BYTES,
        dir,
    )
}

fn restore_attachment_bytes_to_dir(
    original_path: &str,
    mime: Option<&str>,
    bytes: &[u8],
    size_bytes: u64,
    sha256: &str,
    max_size_bytes: u64,
    dir: &Path,
) -> Result<PathBuf> {
    if size_bytes > max_size_bytes {
        return Err(anyhow!("attachment payload exceeds size cap"));
    }
    if bytes.len() as u64 != size_bytes {
        return Err(anyhow!(
            "attachment payload size mismatch: expected {}, got {}",
            size_bytes,
            bytes.len()
        ));
    }
    let actual_sha = sha256_hex(bytes);
    if actual_sha != sha256 {
        return Err(anyhow!("attachment payload checksum mismatch"));
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let ext = attachment_extension(original_path, mime);
    let filename = if ext.is_empty() {
        format!("mematt_{}", uuid::Uuid::new_v4().simple())
    } else {
        format!("mematt_{}.{}", uuid::Uuid::new_v4().simple(), ext)
    };
    let path = dir.join(filename);
    crate::platform::write_atomic(&path, bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn attachment_extension(original_path: &str, mime: Option<&str>) -> String {
    if let Some(ext) = Path::new(original_path)
        .extension()
        .and_then(|s| s.to_str())
        .map(sanitize_extension)
        .filter(|s| !s.is_empty())
    {
        return ext;
    }
    match mime.unwrap_or("") {
        "image/jpeg" => "jpg".to_string(),
        "image/png" => "png".to_string(),
        "image/gif" => "gif".to_string(),
        "image/webp" => "webp".to_string(),
        "audio/mpeg" => "mp3".to_string(),
        "audio/wav" | "audio/x-wav" => "wav".to_string(),
        "audio/ogg" => "ogg".to_string(),
        "application/pdf" => "pdf".to_string(),
        _ => "bin".to_string(),
    }
}

fn sanitize_extension(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn collect_all_claim_details() -> Result<Vec<ClaimDetail>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = claims::list_claims(ClaimListFilter {
            limit: Some(CLAIM_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        })?;
        let page_len = page.len();
        for claim in page {
            if let Some(detail) = claims::get_claim(&claim.id)? {
                out.push(detail);
            }
        }
        if page_len < CLAIM_PAGE_SIZE {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn collect_all_claim_records() -> Result<Vec<ClaimRecord>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = claims::list_claims(ClaimListFilter {
            limit: Some(CLAIM_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        })?;
        let page_len = page.len();
        out.extend(page);
        if page_len < CLAIM_PAGE_SIZE {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(out)
}

fn build_claim_restore_plan(
    bundle_claims: &[ClaimDetail],
    issues: &mut Vec<MemoryBackupPreviewIssue>,
) -> MemoryBackupClaimRestorePlan {
    let mut plan = MemoryBackupClaimRestorePlan {
        total: bundle_claims.len(),
        ..Default::default()
    };
    if bundle_claims.is_empty() {
        return plan;
    }

    let current_claims = match collect_all_claim_records() {
        Ok(claims) => claims,
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_claims_unavailable".to_string(),
                message: format!("Current claim graph could not be compared: {e}"),
            });
            Vec::new()
        }
    };
    let current_ids: HashSet<String> = current_claims
        .iter()
        .map(|claim| claim.id.clone())
        .collect();
    let current_fingerprints: HashSet<String> =
        current_claims.iter().map(claim_fingerprint).collect();
    let mut current_conflict_claim_by_key: HashMap<String, &ClaimRecord> = HashMap::new();
    for claim in current_claims
        .iter()
        .filter(|claim| claim_status_can_conflict(&claim.status))
    {
        current_conflict_claim_by_key
            .entry(claim_conflict_key(claim))
            .or_insert(claim);
    }

    for detail in bundle_claims {
        let claim = &detail.claim;
        increment_count(&mut plan.by_type, normalized_bucket(&claim.claim_type));
        increment_count(&mut plan.by_status, normalized_bucket(&claim.status));
        plan.manual_evidence_rows += detail
            .evidence
            .iter()
            .filter(|evidence| {
                evidence.source_type == "manual" || evidence.evidence_class == "manual_correction"
            })
            .count();

        let existing_by_id = current_ids.contains(&claim.id);
        let exact_match = current_fingerprints.contains(&claim_fingerprint(claim));
        if existing_by_id {
            plan.existing_by_id += 1;
        }
        if exact_match {
            plan.exact_matches += 1;
        }
        if existing_by_id || exact_match {
            continue;
        }

        plan.import_candidates += 1;
        let conflict_key = claim_conflict_key(claim);
        if claim_status_can_conflict(&claim.status)
            && current_conflict_claim_by_key.contains_key(&conflict_key)
        {
            plan.conflicting_candidates += 1;
            if plan.conflict_examples.len() < 3 {
                if let Some(existing) = current_conflict_claim_by_key.get(&conflict_key) {
                    plan.conflict_examples
                        .push(build_claim_conflict_example(claim, existing));
                }
            }
        }
        match claim.status.as_str() {
            "needs_review" => plan.needs_review_candidates += 1,
            "archived" => plan.archived_candidates += 1,
            "superseded" => plan.superseded_candidates += 1,
            "expired" => plan.expired_candidates += 1,
            _ => {}
        }
    }

    plan
}

fn build_profile_restore_plan(
    bundle_profiles: &[ProfileSnapshotRecord],
    issues: &mut Vec<MemoryBackupPreviewIssue>,
) -> MemoryBackupProfileRestorePlan {
    let mut plan = MemoryBackupProfileRestorePlan {
        total: bundle_profiles.len(),
        ..Default::default()
    };
    if bundle_profiles.is_empty() {
        return plan;
    }

    let current_profiles = match crate::memory::dreaming::list_profile_snapshots() {
        Ok(snapshots) => snapshots,
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_profiles_unavailable".to_string(),
                message: format!("Current profile snapshots could not be compared: {e}"),
            });
            Vec::new()
        }
    };
    let current_scopes: HashSet<String> = current_profiles.iter().map(profile_scope_key).collect();
    let current_fingerprints: HashSet<String> =
        current_profiles.iter().map(profile_fingerprint).collect();
    let mut matching_scopes = HashSet::new();

    for snapshot in bundle_profiles {
        increment_count(
            &mut plan.by_scope_type,
            normalized_bucket(&snapshot.scope_type),
        );
        let scope_key = profile_scope_key(snapshot);
        let scope_matches = current_scopes.contains(&scope_key);
        if scope_matches {
            matching_scopes.insert(scope_key);
        }

        if current_fingerprints.contains(&profile_fingerprint(snapshot)) {
            plan.exact_matches += 1;
        } else {
            plan.import_candidates += 1;
            if scope_matches {
                plan.conflicting_scope_candidates += 1;
            }
        }
    }

    plan.matching_scopes = matching_scopes.len();
    plan
}

fn episode_restore_counts(
    bundle_episodes: &[MemoryEpisodeRecord],
    issues: &mut Vec<MemoryBackupPreviewIssue>,
) -> (usize, usize, usize) {
    if bundle_episodes.is_empty() {
        return (0, 0, 0);
    }
    let current = match collect_all_episodes() {
        Ok(items) => items,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_episodes_unavailable".to_string(),
                message: format!("Current episodes could not be compared: {e}"),
            });
            Vec::new()
        }
    };
    let current_ids: HashSet<String> = current.iter().map(|episode| episode.id.clone()).collect();
    let current_fingerprints: HashSet<String> = current.iter().map(episode_fingerprint).collect();
    let mut id_matches = 0usize;
    let mut exact_matches = 0usize;
    let mut import_candidates = 0usize;
    for episode in bundle_episodes {
        let id_match = current_ids.contains(&episode.id);
        let exact_match = current_fingerprints.contains(&episode_fingerprint(episode));
        if id_match {
            id_matches += 1;
        }
        if exact_match {
            exact_matches += 1;
        }
        if !id_match && !exact_match {
            import_candidates += 1;
        }
    }
    (id_matches, exact_matches, import_candidates)
}

fn procedure_restore_counts(
    bundle_procedures: &[MemoryProcedureRecord],
    issues: &mut Vec<MemoryBackupPreviewIssue>,
) -> (usize, usize, usize) {
    if bundle_procedures.is_empty() {
        return (0, 0, 0);
    }
    let current = match collect_all_procedures() {
        Ok(items) => items,
        Err(e) if is_episode_store_uninitialised(&e) => Vec::new(),
        Err(e) => {
            issues.push(MemoryBackupPreviewIssue {
                severity: MemoryHealthSeverity::Warning,
                code: "current_procedures_unavailable".to_string(),
                message: format!("Current procedures could not be compared: {e}"),
            });
            Vec::new()
        }
    };
    let current_ids: HashSet<String> = current
        .iter()
        .map(|procedure| procedure.id.clone())
        .collect();
    let current_fingerprints: HashSet<String> = current.iter().map(procedure_fingerprint).collect();
    let mut id_matches = 0usize;
    let mut exact_matches = 0usize;
    let mut import_candidates = 0usize;
    for procedure in bundle_procedures {
        let id_match = current_ids.contains(&procedure.id);
        let exact_match = current_fingerprints.contains(&procedure_fingerprint(procedure));
        if id_match {
            id_matches += 1;
        }
        if exact_match {
            exact_matches += 1;
        }
        if !id_match && !exact_match {
            import_candidates += 1;
        }
    }
    (id_matches, exact_matches, import_candidates)
}

fn is_episode_store_uninitialised(error: &anyhow::Error) -> bool {
    error.to_string().contains("episode store not initialised")
}

fn increment_count(map: &mut BTreeMap<String, usize>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

fn normalized_bucket(raw: &str) -> String {
    let normalized = normalize_fingerprint_part(raw);
    if normalized.is_empty() {
        "<unknown>".to_string()
    } else {
        normalized
    }
}

fn claim_fingerprint(claim: &ClaimRecord) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalize_fingerprint_part(&claim.scope_type),
        normalize_fingerprint_part(claim.scope_id.as_deref().unwrap_or("")),
        normalize_fingerprint_part(&claim.claim_type),
        normalize_fingerprint_part(&claim.subject),
        normalize_fingerprint_part(&claim.predicate),
        normalize_fingerprint_part(&claim.object),
        normalize_fingerprint_part(&claim.content)
    )
}

fn claim_conflict_key(claim: &ClaimRecord) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalize_fingerprint_part(&claim.scope_type),
        normalize_fingerprint_part(claim.scope_id.as_deref().unwrap_or("")),
        normalize_fingerprint_part(&claim.claim_type),
        normalize_fingerprint_part(&claim.subject),
        normalize_fingerprint_part(&claim.predicate)
    )
}

fn claim_status_can_conflict(status: &str) -> bool {
    matches!(status, "active" | "needs_review")
}

fn build_claim_conflict_example(
    incoming: &ClaimRecord,
    existing: &ClaimRecord,
) -> MemoryBackupClaimConflictExample {
    MemoryBackupClaimConflictExample {
        incoming_claim_id: incoming.id.clone(),
        existing_claim_id: existing.id.clone(),
        scope: claim_scope_label(incoming),
        claim_type: incoming.claim_type.clone(),
        subject: incoming.subject.clone(),
        predicate: incoming.predicate.clone(),
        incoming_object: incoming.object.clone(),
        existing_object: existing.object.clone(),
        incoming_content: incoming.content.clone(),
        existing_content: existing.content.clone(),
    }
}

fn claim_scope_label(claim: &ClaimRecord) -> String {
    match claim.scope_id.as_deref() {
        Some(id) if !id.is_empty() => format!("{}:{id}", claim.scope_type),
        _ => claim.scope_type.clone(),
    }
}

fn profile_scope_key(snapshot: &ProfileSnapshotRecord) -> String {
    format!(
        "{}\u{1f}{}",
        normalize_fingerprint_part(&snapshot.scope_type),
        normalize_fingerprint_part(snapshot.scope_id.as_deref().unwrap_or(""))
    )
}

fn profile_fingerprint(snapshot: &ProfileSnapshotRecord) -> String {
    format!(
        "{}\u{1f}{}",
        profile_scope_key(snapshot),
        normalize_fingerprint_part(&snapshot.body_md)
    )
}

fn profile_scope_label(snapshot: &ProfileSnapshotRecord) -> String {
    match snapshot.scope_id.as_deref() {
        Some(id) if !id.is_empty() => format!("{}:{id}", snapshot.scope_type),
        _ => snapshot.scope_type.clone(),
    }
}

fn restore_profile_snapshot(snapshot: &ProfileSnapshotRecord) -> Result<()> {
    validate_profile_snapshot(snapshot)?;
    crate::memory::dreaming::insert_profile_snapshot_for_restore(
        &snapshot.scope_type,
        snapshot.scope_id.as_deref().unwrap_or(""),
        &snapshot.body_md,
        &snapshot.source_run_id,
    )?;
    Ok(())
}

fn validate_profile_snapshot(snapshot: &ProfileSnapshotRecord) -> Result<()> {
    match snapshot.scope_type.as_str() {
        "global" => {}
        "agent" | "project" => {
            if snapshot.scope_id.as_deref().unwrap_or("").trim().is_empty() {
                anyhow::bail!("{} profile requires scope_id", snapshot.scope_type);
            }
        }
        other => anyhow::bail!("invalid profile scope_type: {other}"),
    }
    if snapshot.body_md.trim().is_empty() {
        anyhow::bail!("profile body is empty");
    }
    if snapshot.source_run_id.trim().is_empty() {
        anyhow::bail!("profile source_run_id is empty");
    }
    Ok(())
}

fn normalize_fingerprint_part(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn memory_fingerprint(memory: &MemoryEntry) -> String {
    let scope = match &memory.scope {
        crate::memory::MemoryScope::Global => "global".to_string(),
        crate::memory::MemoryScope::Agent { id } => format!("agent:{id}"),
        crate::memory::MemoryScope::Project { id } => format!("project:{id}"),
    };
    let mut tags = memory.tags.clone();
    tags.sort();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        memory.memory_type.as_str(),
        scope,
        tags.join("\u{1e}"),
        memory.content.trim()
    )
}

fn episode_fingerprint(episode: &MemoryEpisodeRecord) -> String {
    let scope = memory_scope_fingerprint(&episode.scope);
    let actions = episode
        .actions
        .iter()
        .map(|action| normalize_fingerprint_part(action))
        .collect::<Vec<_>>()
        .join("\u{1e}");
    let tags = sorted_normalized_parts(&episode.tags).join("\u{1e}");
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        scope,
        normalize_fingerprint_part(&episode.title),
        normalize_fingerprint_part(&episode.situation),
        actions,
        normalize_fingerprint_part(&episode.outcome),
        normalize_fingerprint_part(&episode.lesson),
        tags
    )
}

fn procedure_fingerprint(procedure: &MemoryProcedureRecord) -> String {
    let scope = memory_scope_fingerprint(&procedure.scope);
    let source_episode_ids = sorted_normalized_parts(&procedure.source_episode_ids).join("\u{1e}");
    let tags = sorted_normalized_parts(&procedure.tags).join("\u{1e}");
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        scope,
        normalize_fingerprint_part(&procedure.title),
        normalize_fingerprint_part(&procedure.trigger),
        normalize_fingerprint_part(&procedure.steps_markdown),
        normalize_fingerprint_part(&procedure.constraints_markdown),
        source_episode_ids,
        tags
    )
}

fn memory_scope_fingerprint(scope: &crate::memory::MemoryScope) -> String {
    match scope {
        crate::memory::MemoryScope::Global => "global".to_string(),
        crate::memory::MemoryScope::Agent { id } => {
            format!("agent:{}", normalize_fingerprint_part(id))
        }
        crate::memory::MemoryScope::Project { id } => {
            format!("project:{}", normalize_fingerprint_part(id))
        }
    }
}

fn sorted_normalized_parts(values: &[String]) -> Vec<String> {
    let mut out = values
        .iter()
        .map(|value| normalize_fingerprint_part(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn build_preview_next_steps(preview: &MemoryBackupImportPreview) -> Vec<String> {
    if !preview.valid {
        return vec!["Choose a valid Hope Agent memory backup JSON or ZIP file.".to_string()];
    }
    let mut steps = Vec::new();
    if preview.legacy_import_candidates > 0 {
        steps.push(format!(
            "Preview can import {} legacy memory candidate(s) after user confirmation.",
            preview.legacy_import_candidates
        ));
    }
    if preview.legacy_history_restorable > 0 {
        steps.push(format!(
            "{} legacy memory history event(s) can be restored after their memory rows are mapped.",
            preview.legacy_history_restorable
        ));
    }
    if preview.claim_restore_plan.import_candidates > 0 {
        steps.push(format!(
            "{} structured claim candidate(s) can be restored after explicit confirmation.",
            preview.claim_restore_plan.import_candidates
        ));
    }
    if preview.profile_restore_plan.import_candidates > 0 {
        steps.push(format!(
            "{} profile snapshot candidate(s) can be restored after explicit confirmation.",
            preview.profile_restore_plan.import_candidates
        ));
    }
    if preview.episode_import_candidates > 0 {
        steps.push(format!(
            "{} episode memory candidate(s) can be restored after explicit confirmation.",
            preview.episode_import_candidates
        ));
    }
    if preview.procedure_import_candidates > 0 {
        steps.push(format!(
            "{} procedure memory candidate(s) can be restored after explicit confirmation.",
            preview.procedure_import_candidates
        ));
    }
    if preview.experience_history_restorable > 0 {
        steps.push(format!(
            "{} experience/workflow history event(s) can be restored after their target records are mapped.",
            preview.experience_history_restorable
        ));
    }
    let restorable_attachment_count = preview.attachment_payload_count
        + preview.attachment_chunked_ref_count
        + preview.attachment_external_available_count;
    if restorable_attachment_count > 0 {
        steps.push(format!(
            "{} attachment payload(s) can be restored with their memory rows.",
            restorable_attachment_count
        ));
    }
    if preview.attachment_missing_count > 0 {
        steps.push(
            "Some attachment references have no packed payload; keep original files available."
                .to_string(),
        );
    }
    if preview.attachment_external_ref_count > 0 {
        let missing_sidecars = preview
            .attachment_external_ref_count
            .saturating_sub(preview.attachment_external_available_count);
        if missing_sidecars > 0 {
            steps.push(format!(
                "{} large attachment(s) have sidecar metadata but still need external payload files before restore can include them.",
                missing_sidecars
            ));
        }
    }
    if steps.is_empty() {
        steps.push("No importable memory changes were found.".to_string());
    }
    steps
}

fn memory_config_manifest() -> serde_json::Value {
    let cfg = crate::config::cached_config();
    json!({
        "memoryExtract": cfg.memory_extract,
        "memorySelection": cfg.memory_selection,
        "memoryBudget": cfg.memory_budget,
        "memoryEmbedding": cfg.memory_embedding,
        "dreaming": cfg.dreaming,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryScope, MemoryStats, MemoryType, NewMemory, SqliteMemoryBackend};

    fn temp_backend() -> SqliteMemoryBackend {
        let dir = std::env::temp_dir().join(format!("ha-memory-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        SqliteMemoryBackend::open(&dir.join("memory.db")).unwrap()
    }

    #[test]
    fn backup_bundle_exports_legacy_memory_and_manifest() {
        let backend = temp_backend();
        let id = backend
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "The user prefers concise answers.".to_string(),
                tags: vec!["preference".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: true,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let bundle = export_backup_bundle(&backend).unwrap();
        assert_eq!(bundle.schema_version, MEMORY_BACKUP_SCHEMA_VERSION);
        assert_eq!(bundle.manifest.legacy_memory_count, 1);
        assert_eq!(bundle.manifest.legacy_history_count, 1);
        assert_eq!(bundle.legacy_memories[0].id, id);
        assert_eq!(bundle.legacy_history.len(), 1);
        assert_eq!(bundle.legacy_history[0].memory_id, id);
        assert!(bundle.legacy_markdown.contains("prefers concise answers"));
        assert_eq!(bundle.stats.total, 1);
    }

    #[test]
    fn backup_bundle_packs_small_attachment_payloads() {
        let backend = temp_backend();
        let dir =
            std::env::temp_dir().join(format!("ha-memory-attachment-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let attachment = dir.join("voice memo.wav");
        std::fs::write(&attachment, b"audio bytes").unwrap();

        let id = backend
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Audio note".to_string(),
                tags: vec!["audio".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: Some(attachment.to_string_lossy().to_string()),
                attachment_mime: Some("audio/wav".to_string()),
            })
            .unwrap();

        let bundle = export_backup_bundle(&backend).unwrap();
        assert_eq!(bundle.manifest.attachment_ref_count, 1);
        assert_eq!(bundle.manifest.attachment_payload_count, 1);
        assert_eq!(bundle.manifest.attachment_missing_count, 0);
        assert_eq!(bundle.attachment_payloads.len(), 1);
        let payload = &bundle.attachment_payloads[0];
        assert_eq!(payload.memory_id, id);
        assert_eq!(payload.mime.as_deref(), Some("audio/wav"));
        assert_eq!(payload.size_bytes, 11);
        assert_eq!(payload.sha256, sha256_hex(b"audio bytes"));
    }

    #[test]
    fn backup_bundle_chunks_large_attachment_payloads() {
        let backend = temp_backend();
        let dir = std::env::temp_dir().join(format!(
            "ha-memory-large-attachment-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let attachment = dir.join("large-note.bin");
        let bytes: Vec<u8> = (0..(MAX_ATTACHMENT_PAYLOAD_BYTES as usize + 123))
            .map(|idx| (idx % 251) as u8)
            .collect();
        std::fs::write(&attachment, &bytes).unwrap();

        let id = backend
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Large note".to_string(),
                tags: vec!["large".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: Some(attachment.to_string_lossy().to_string()),
                attachment_mime: Some("application/octet-stream".to_string()),
            })
            .unwrap();

        let bundle = export_backup_bundle(&backend).unwrap();
        assert_eq!(bundle.manifest.attachment_ref_count, 1);
        assert_eq!(bundle.manifest.attachment_payload_count, 0);
        assert_eq!(bundle.manifest.attachment_chunked_ref_count, 1);
        assert_eq!(bundle.manifest.attachment_missing_count, 0);
        assert_eq!(bundle.manifest.attachment_payload_bytes, bytes.len() as u64);
        assert!(bundle.attachment_payloads.is_empty());
        assert_eq!(
            bundle.attachment_payload_chunks.len(),
            (bytes.len() + ATTACHMENT_CHUNK_BYTES - 1) / ATTACHMENT_CHUNK_BYTES
        );
        assert!(bundle
            .attachment_payload_chunks
            .iter()
            .all(|chunk| chunk.memory_id == id));
    }

    #[test]
    fn backup_archive_includes_json_manifest_and_sidecars() {
        let backend = temp_backend();
        backend
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Large sidecar note".to_string(),
                tags: vec!["sidecar".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        let dir = std::env::temp_dir().join(format!("ha-memory-sidecar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar_path = dir.join("huge recording.wav");
        std::fs::write(&sidecar_path, b"external bytes").unwrap();

        let mut bundle = export_backup_bundle(&backend).unwrap();
        bundle.manifest.attachment_external_ref_count = 1;
        bundle
            .attachment_external_payloads
            .push(MemoryBackupAttachmentExternalPayload {
                memory_id: 42,
                original_path: sidecar_path.to_string_lossy().to_string(),
                mime: Some("audio/wav".to_string()),
                size_bytes: 14,
                sha256: sha256_hex(b"external bytes"),
                sidecar_file_name: "memory-42-attachment.wav".to_string(),
                reason: "test".to_string(),
            });

        let archive = build_backup_archive_from_bundle(&bundle).unwrap();
        let cursor = std::io::Cursor::new(archive);
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        let mut manifest = String::new();
        zip.by_name("memory-backup.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        assert!(manifest.contains(MEMORY_BACKUP_SCHEMA_VERSION));
        let mut sidecar = Vec::new();
        zip.by_name("attachments/memory-42-attachment.wav")
            .unwrap()
            .read_to_end(&mut sidecar)
            .unwrap();
        assert_eq!(sidecar, b"external bytes");
    }

    #[test]
    fn backup_archive_preview_counts_verified_sidecars() {
        let source = temp_backend();
        let dir = std::env::temp_dir().join(format!(
            "ha-memory-preview-sidecar-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar_path = dir.join("field recording.wav");
        std::fs::write(&sidecar_path, b"sidecar preview bytes").unwrap();
        let id = source
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Field recording".to_string(),
                tags: vec!["audio".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: Some(sidecar_path.to_string_lossy().to_string()),
                attachment_mime: Some("audio/wav".to_string()),
            })
            .unwrap();
        let mut bundle = export_backup_bundle(&source).unwrap();
        bundle.attachment_payloads.clear();
        bundle.attachment_payload_chunks.clear();
        bundle.attachment_external_payloads = vec![MemoryBackupAttachmentExternalPayload {
            memory_id: id,
            original_path: sidecar_path.to_string_lossy().to_string(),
            mime: Some("audio/wav".to_string()),
            size_bytes: 21,
            sha256: sha256_hex(b"sidecar preview bytes"),
            sidecar_file_name: "memory-preview-sidecar.wav".to_string(),
            reason: "test".to_string(),
        }];
        bundle.manifest.attachment_payload_count = 0;
        bundle.manifest.attachment_chunk_count = 0;
        bundle.manifest.attachment_chunked_ref_count = 0;
        bundle.manifest.attachment_external_ref_count = 1;
        bundle.manifest.attachment_payload_bytes = 0;
        bundle.manifest.attachment_missing_count = 0;

        let archive = build_backup_archive_from_bundle(&bundle).unwrap();
        let target = temp_backend();
        let preview = preview_backup_archive(&target, &archive).unwrap();
        assert!(preview.valid);
        assert_eq!(preview.attachment_external_ref_count, 1);
        assert_eq!(preview.attachment_external_available_count, 1);
        assert_eq!(preview.attachment_missing_count, 0);
        assert!(preview
            .issues
            .iter()
            .any(|issue| issue.code == "attachments_external_sidecars_available"));
    }

    #[test]
    fn backup_archive_restore_restores_verified_sidecar_attachment() {
        let source = temp_backend();
        let dir = std::env::temp_dir().join(format!(
            "ha-memory-restore-sidecar-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sidecar_path = dir.join("meeting audio.wav");
        std::fs::write(&sidecar_path, b"verified sidecar bytes").unwrap();
        let id = source
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Meeting audio".to_string(),
                tags: vec!["audio".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: Some(sidecar_path.to_string_lossy().to_string()),
                attachment_mime: Some("audio/wav".to_string()),
            })
            .unwrap();
        let mut bundle = export_backup_bundle(&source).unwrap();
        bundle.attachment_payloads.clear();
        bundle.attachment_payload_chunks.clear();
        bundle.attachment_external_payloads = vec![MemoryBackupAttachmentExternalPayload {
            memory_id: id,
            original_path: sidecar_path.to_string_lossy().to_string(),
            mime: Some("audio/wav".to_string()),
            size_bytes: 22,
            sha256: sha256_hex(b"verified sidecar bytes"),
            sidecar_file_name: "memory-restore-sidecar.wav".to_string(),
            reason: "test".to_string(),
        }];
        bundle.manifest.attachment_payload_count = 0;
        bundle.manifest.attachment_chunk_count = 0;
        bundle.manifest.attachment_chunked_ref_count = 0;
        bundle.manifest.attachment_external_ref_count = 1;
        bundle.manifest.attachment_payload_bytes = 0;
        bundle.manifest.attachment_missing_count = 0;

        let archive = build_backup_archive_from_bundle(&bundle).unwrap();
        let target = temp_backend();
        let result = restore_backup_legacy_memories_from_archive(
            &target,
            &archive,
            MemoryBackupRestoreOptions::default(),
        )
        .unwrap();

        assert_eq!(result.import_result.created, 1);
        assert_eq!(result.restored_attachments, 1);
        assert_eq!(result.skipped_attachment_refs, 0);
        let imported = target.list(None, None, 10, 0).unwrap();
        assert_eq!(imported.len(), 1);
        let restored_path = imported[0].attachment_path.as_ref().unwrap();
        assert_eq!(
            std::fs::read(restored_path).unwrap(),
            b"verified sidecar bytes"
        );
        assert_eq!(imported[0].attachment_mime.as_deref(), Some("audio/wav"));
    }

    #[test]
    fn restore_attachment_payload_to_dir_verifies_and_writes_file() {
        let dir =
            std::env::temp_dir().join(format!("ha-memory-restore-att-{}", uuid::Uuid::new_v4()));
        let payload = MemoryBackupAttachmentPayload {
            memory_id: 7,
            original_path: "/old/path/photo.png".to_string(),
            mime: Some("image/png".to_string()),
            size_bytes: 9,
            sha256: sha256_hex(b"png bytes"),
            base64: base64::engine::general_purpose::STANDARD.encode(b"png bytes"),
        };

        let restored = restore_attachment_payload_to_dir(&payload, &dir).unwrap();
        assert_eq!(restored.extension().and_then(|s| s.to_str()), Some("png"));
        assert_eq!(std::fs::read(restored).unwrap(), b"png bytes");
    }

    #[test]
    fn restore_attachment_chunk_payload_to_dir_verifies_and_writes_file() {
        let dir = std::env::temp_dir().join(format!(
            "ha-memory-restore-chunked-att-{}",
            uuid::Uuid::new_v4()
        ));
        let bytes: Vec<u8> = (0..(ATTACHMENT_CHUNK_BYTES + 9))
            .map(|idx| (idx % 193) as u8)
            .collect();
        let sha256 = sha256_hex(&bytes);
        let chunk_count = bytes.chunks(ATTACHMENT_CHUNK_BYTES).count();
        let chunks: Vec<MemoryBackupAttachmentChunkPayload> = bytes
            .chunks(ATTACHMENT_CHUNK_BYTES)
            .enumerate()
            .map(|(chunk_index, chunk)| MemoryBackupAttachmentChunkPayload {
                memory_id: 9,
                original_path: "/old/path/archive.pdf".to_string(),
                mime: Some("application/pdf".to_string()),
                size_bytes: bytes.len() as u64,
                sha256: sha256.clone(),
                chunk_index,
                chunk_count,
                chunk_size_bytes: chunk.len() as u64,
                chunk_sha256: sha256_hex(chunk),
                base64: base64::engine::general_purpose::STANDARD.encode(chunk),
            })
            .collect();
        let refs: Vec<&MemoryBackupAttachmentChunkPayload> = chunks.iter().collect();

        let restored = restore_attachment_chunk_payloads_to_dir(&refs, &dir).unwrap();
        assert_eq!(restored.extension().and_then(|s| s.to_str()), Some("pdf"));
        assert_eq!(std::fs::read(restored).unwrap(), bytes);
    }

    #[test]
    fn backup_preview_reports_exact_matches_and_candidates() {
        let current = temp_backend();
        current
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Already here".to_string(),
                tags: vec!["a".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let source = temp_backend();
        source
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Already here".to_string(),
                tags: vec!["a".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        source
            .add(NewMemory {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Global,
                content: "New from backup".to_string(),
                tags: vec![],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let bundle = export_backup_bundle(&source).unwrap();
        let content = serde_json::to_string(&bundle).unwrap();
        let preview = preview_backup_bundle(&current, &content).unwrap();
        assert!(preview.valid);
        assert_eq!(preview.legacy_memory_count, 2);
        assert_eq!(preview.legacy_exact_matches, 1);
        assert_eq!(preview.legacy_import_candidates, 1);
    }

    #[test]
    fn backup_preview_next_steps_include_restorable_experience_history() {
        let preview = MemoryBackupImportPreview {
            valid: true,
            schema_version: Some(MEMORY_BACKUP_SCHEMA_VERSION.to_string()),
            exported_at: None,
            app_version: None,
            source_manifest: None,
            current_stats: MemoryStats {
                total: 0,
                by_type: HashMap::new(),
                by_source: HashMap::new(),
                with_embedding: 0,
                oldest: None,
                newest: None,
            },
            legacy_memory_count: 0,
            legacy_exact_matches: 0,
            legacy_import_candidates: 0,
            legacy_duplicate_in_bundle: 0,
            legacy_history_count: 0,
            legacy_history_restorable: 0,
            legacy_history_skipped_unmapped: 0,
            attachment_ref_count: 0,
            attachment_payload_count: 0,
            attachment_chunk_count: 0,
            attachment_chunked_ref_count: 0,
            attachment_external_ref_count: 0,
            attachment_external_available_count: 0,
            attachment_payload_bytes: 0,
            attachment_missing_count: 0,
            claim_count: 0,
            claim_id_matches: 0,
            claim_restore_plan: MemoryBackupClaimRestorePlan::default(),
            evidence_count: 0,
            claim_link_count: 0,
            profile_snapshot_count: 0,
            profile_restore_plan: MemoryBackupProfileRestorePlan::default(),
            episode_count: 0,
            episode_id_matches: 0,
            episode_exact_matches: 0,
            episode_import_candidates: 0,
            procedure_count: 0,
            procedure_id_matches: 0,
            procedure_exact_matches: 0,
            procedure_import_candidates: 0,
            experience_history_count: 2,
            experience_history_restorable: 2,
            experience_history_skipped_unmapped: 0,
            unsupported_sections: vec![],
            issues: vec![],
            next_steps: vec![],
        };

        let steps = build_preview_next_steps(&preview);
        assert_eq!(
            steps,
            vec![
                "2 experience/workflow history event(s) can be restored after their target records are mapped."
                    .to_string()
            ]
        );
    }

    #[test]
    fn backup_preview_rejects_wrong_schema_without_writing() {
        let backend = temp_backend();
        let preview = preview_backup_bundle(
            &backend,
            r#"{"schemaVersion":"hope.memory.bundle.v999","legacyMemories":[]}"#,
        )
        .unwrap();
        assert!(!preview.valid);
        assert_eq!(preview.issues[0].code, "unsupported_schema");
        assert_eq!(
            preview.next_steps,
            vec!["Choose a valid Hope Agent memory backup JSON or ZIP file.".to_string()]
        );
    }

    #[test]
    fn encrypted_backup_export_rejects_weak_passphrase() {
        let backend = temp_backend();
        assert!(export_encrypted_backup_bundle(&backend, "password1234").is_err());
        assert!(export_encrypted_backup_bundle(&backend, "aaaaaaaaaaaa").is_err());
        assert!(export_encrypted_backup_bundle(&backend, "correct horse battery staple").is_ok());
    }

    #[test]
    fn encrypted_backup_preview_requires_passphrase_and_round_trips() {
        let source = temp_backend();
        source
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Encrypted private preference".to_string(),
                tags: vec!["private".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let passphrase = "correct horse battery staple";
        let encrypted = export_encrypted_backup_bundle(&source, passphrase).unwrap();
        assert_eq!(
            encrypted.schema_version,
            MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION
        );
        let content = serde_json::to_string(&encrypted).unwrap();
        assert!(!content.contains("Encrypted private preference"));

        let current = temp_backend();
        let locked = preview_backup_bundle(&current, &content).unwrap();
        assert!(!locked.valid);
        assert_eq!(locked.issues[0].code, "encrypted_passphrase_required");
        assert_eq!(
            locked.next_steps,
            vec![
                "Enter the backup passphrase to preview or restore this encrypted backup."
                    .to_string()
            ]
        );

        let wrong =
            preview_backup_bundle_with_passphrase(&current, &content, Some("wrong passphrase"))
                .unwrap();
        assert!(!wrong.valid);
        assert_eq!(wrong.issues[0].code, "encrypted_decrypt_failed");
        assert_eq!(
            wrong.next_steps,
            vec![
                "Check the backup passphrase or choose an uncorrupted encrypted backup."
                    .to_string()
            ]
        );

        let preview =
            preview_backup_bundle_with_passphrase(&current, &content, Some(passphrase)).unwrap();
        assert!(preview.valid);
        assert_eq!(preview.legacy_memory_count, 1);
        assert_eq!(preview.legacy_import_candidates, 1);

        let result = restore_backup_legacy_memories_with_passphrase(
            &current,
            &content,
            MemoryBackupRestoreOptions::default(),
            Some(passphrase),
        )
        .unwrap();
        assert_eq!(result.import_result.created, 1);
        assert_eq!(result.preview.legacy_import_candidates, 0);
    }

    #[test]
    fn encrypted_backup_preview_reports_invalid_decrypted_plaintext() {
        let current = temp_backend();
        let passphrase = "correct horse battery staple";
        let encrypted = encrypt_backup_plaintext(
            &[0xff, 0xfe, 0xfd],
            passphrase,
            "2026-07-08T00:00:00Z".to_string(),
            "test".to_string(),
        )
        .unwrap();
        let content = serde_json::to_string(&encrypted).unwrap();

        let preview =
            preview_backup_bundle_with_passphrase(&current, &content, Some(passphrase)).unwrap();

        assert!(!preview.valid);
        assert_eq!(
            preview.schema_version,
            Some(MEMORY_ENCRYPTED_BACKUP_SCHEMA_VERSION.to_string())
        );
        assert_eq!(preview.issues[0].code, "encrypted_plaintext_invalid");
        assert_eq!(
            preview.next_steps,
            vec!["Choose an uncorrupted encrypted backup or export the backup again.".to_string()]
        );
    }

    #[test]
    fn restore_backup_imports_missing_legacy_memories_only() {
        let current = temp_backend();
        current
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Already here".to_string(),
                tags: vec!["a".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let source = temp_backend();
        source
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "Already here".to_string(),
                tags: vec!["a".to_string()],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        source
            .add(NewMemory {
                memory_type: MemoryType::Feedback,
                scope: MemoryScope::Global,
                content: "Restore this missing preference".to_string(),
                tags: vec!["restore".to_string()],
                source: "auto".to_string(),
                source_session_id: Some("old-session".to_string()),
                pinned: true,
                attachment_path: Some("/tmp/missing.bin".to_string()),
                attachment_mime: Some("application/octet-stream".to_string()),
            })
            .unwrap();

        let content = serde_json::to_string(&export_backup_bundle(&source).unwrap()).unwrap();
        let result = restore_backup_legacy_memories(
            &current,
            &content,
            MemoryBackupRestoreOptions::default(),
        )
        .unwrap();

        assert_eq!(result.attempted_legacy_memories, 1);
        assert_eq!(result.skipped_exact_matches, 1);
        assert_eq!(result.skipped_attachment_refs, 1);
        assert_eq!(result.import_result.created, 1);
        assert_eq!(result.preview.legacy_import_candidates, 0);

        let memories = collect_all_memories(&current).unwrap();
        assert_eq!(memories.len(), 2);
        let restored = memories
            .iter()
            .find(|m| m.content == "Restore this missing preference")
            .expect("missing memory restored");
        assert_eq!(restored.source, "auto");
        assert!(restored.pinned);
        assert!(restored.attachment_path.is_none());
    }

    #[test]
    fn restore_backup_restores_mappable_legacy_history_idempotently() {
        let source = temp_backend();
        let source_id = source
            .add(NewMemory {
                memory_type: MemoryType::User,
                scope: MemoryScope::Global,
                content: "History restore preference".to_string(),
                tags: vec!["history".to_string()],
                source: "user".to_string(),
                source_session_id: Some("source-session".to_string()),
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();
        source
            .update(
                source_id,
                "History restore preference updated",
                &["history".to_string(), "updated".to_string()],
            )
            .unwrap();
        source.toggle_pin(source_id, true).unwrap();

        let bundle = export_backup_bundle(&source).unwrap();
        assert_eq!(bundle.legacy_history.len(), 3);
        let content = serde_json::to_string(&bundle).unwrap();

        let current = temp_backend();
        current
            .add(NewMemory {
                memory_type: MemoryType::Reference,
                scope: MemoryScope::Global,
                content: "Unrelated row to force a different local id".to_string(),
                tags: vec![],
                source: "user".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            })
            .unwrap();

        let preview = preview_backup_bundle(&current, &content).unwrap();
        assert_eq!(preview.legacy_history_count, 3);
        assert_eq!(preview.legacy_history_restorable, 3);
        assert_eq!(preview.legacy_history_skipped_unmapped, 0);

        let result = restore_backup_legacy_memories(
            &current,
            &content,
            MemoryBackupRestoreOptions::default(),
        )
        .unwrap();
        assert_eq!(result.import_result.created, 1);
        assert_eq!(result.restored_legacy_history, 3);
        assert_eq!(result.skipped_legacy_history_unmapped, 0);

        let restored_memory = collect_all_memories(&current)
            .unwrap()
            .into_iter()
            .find(|memory| memory.content == "History restore preference updated")
            .expect("restored memory row");
        assert_ne!(restored_memory.id, source_id);
        let restored_history = current.history(20, 0).unwrap();
        let mapped_events = restored_history
            .iter()
            .filter(|event| event.memory_id == restored_memory.id)
            .count();
        assert_eq!(
            mapped_events, 4,
            "target import event plus three source audit events should be visible"
        );
        assert!(restored_history
            .iter()
            .any(|event| event.memory_id == restored_memory.id
                && event.action == crate::memory::MemoryHistoryAction::Pin));

        let second = restore_backup_legacy_memories(
            &current,
            &content,
            MemoryBackupRestoreOptions::default(),
        )
        .unwrap();
        assert_eq!(second.import_result.created, 0);
        assert_eq!(second.restored_legacy_history, 0);
    }

    #[test]
    fn restore_backup_rejects_wrong_schema_without_writing() {
        let backend = temp_backend();
        let err = restore_backup_legacy_memories(
            &backend,
            r#"{"schemaVersion":"hope.memory.bundle.v999","legacyMemories":[]}"#,
            MemoryBackupRestoreOptions::default(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("unsupported memory backup schema"),
            "unexpected error: {err}"
        );
        assert_eq!(backend.stats(None).unwrap().total, 0);
    }
}
