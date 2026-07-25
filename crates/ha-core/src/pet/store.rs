use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::Engine as _;
use rand::RngCore;

use super::atlas::{
    portable_pet_id, validate_package, ValidatedPetPackage, MAX_MANIFEST_BYTES, MAX_SPRITE_BYTES,
};
#[cfg(debug_assertions)]
use super::types::BUILTIN_DEBUG_PET_REF;
use super::types::{
    HopePetMetadata, PetDeleteResult, PetExportResult, PetLibrarySnapshot, PetManifest, PetRef,
    PetSourceKind, PetSummary, BUILTIN_DEFAULT_PET_REF,
};

const HOPE_SCHEMA_VERSION: u32 = 1;
const RESTORE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_RESTORE_TOKENS: usize = 128;
const STAGING_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TRASH_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_TRASH_ENTRIES: usize = 256;
const MAX_METADATA_BYTES: usize = 64 * 1024;

static LIBRARY_REVISION: AtomicU64 = AtomicU64::new(1);
static RESTORE_TOKENS: OnceLock<Mutex<HashMap<String, RestoreEntry>>> = OnceLock::new();
static BUILTIN_PACKAGE: OnceLock<ValidatedPetPackage> = OnceLock::new();
#[cfg(debug_assertions)]
static DEBUG_BUILTIN_PACKAGE: OnceLock<ValidatedPetPackage> = OnceLock::new();

// The project-bound asset is generated specifically for Hope Agent and is not
// copied from Codex.  It is validated by the same atlas code as custom pets.
const BUILTIN_SPRITE: &[u8] = include_bytes!("../../../../src/assets/pets/hope-default.png");
#[cfg(debug_assertions)]
const DEBUG_BUILTIN_SPRITE: &[u8] = include_bytes!("../../../../src/assets/pets/hope-debug.png");

#[derive(Debug, Clone)]
struct RestoreEntry {
    trash_path: PathBuf,
    target_id: String,
    package_hash: String,
    expires_at: Instant,
}

fn restore_tokens() -> &'static Mutex<HashMap<String, RestoreEntry>> {
    RESTORE_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn builtin_manifest() -> PetManifest {
    PetManifest {
        id: "hope-default".to_string(),
        display_name: "Hope".to_string(),
        description: Some("A tiny robotic firefly companion.".to_string()),
        sprite_version_number: super::types::PetSpriteVersion::V1,
        spritesheet_path: "spritesheet.png".to_string(),
    }
}

#[cfg(debug_assertions)]
fn debug_builtin_manifest() -> PetManifest {
    PetManifest {
        id: "hope-debug".to_string(),
        display_name: "Debug Pet / 调试宠物".to_string(),
        description: Some(
            "Development-only atlas showing the active animation state and frame.".to_string(),
        ),
        sprite_version_number: super::types::PetSpriteVersion::V1,
        spritesheet_path: "spritesheet.png".to_string(),
    }
}

pub(crate) fn builtin_package_cached() -> Result<&'static ValidatedPetPackage> {
    if let Some(package) = BUILTIN_PACKAGE.get() {
        return Ok(package);
    }
    let package = validate_package(builtin_manifest(), BUILTIN_SPRITE.to_vec())
        .context("builtin_pet_validation_failed")?;
    let _ = BUILTIN_PACKAGE.set(package);
    BUILTIN_PACKAGE
        .get()
        .ok_or_else(|| anyhow::anyhow!("builtin_pet_cache_unavailable"))
}

pub fn builtin_package() -> Result<ValidatedPetPackage> {
    Ok(builtin_package_cached()?.clone())
}

#[cfg(debug_assertions)]
pub(crate) fn debug_builtin_package_cached() -> Result<&'static ValidatedPetPackage> {
    if let Some(package) = DEBUG_BUILTIN_PACKAGE.get() {
        return Ok(package);
    }
    let package = validate_package(debug_builtin_manifest(), DEBUG_BUILTIN_SPRITE.to_vec())
        .context("debug_builtin_pet_validation_failed")?;
    let _ = DEBUG_BUILTIN_PACKAGE.set(package);
    DEBUG_BUILTIN_PACKAGE
        .get()
        .ok_or_else(|| anyhow::anyhow!("debug_builtin_pet_cache_unavailable"))
}

#[cfg(debug_assertions)]
fn debug_builtin_package() -> Result<ValidatedPetPackage> {
    Ok(debug_builtin_package_cached()?.clone())
}

fn builtin_summary() -> Result<PetSummary> {
    let package = builtin_package_cached()?;
    Ok(PetSummary {
        pet_ref: PetRef(BUILTIN_DEFAULT_PET_REF.to_string()),
        manifest: package.manifest.clone(),
        asset_id: "builtin/hope-default".to_string(),
        source_kind: PetSourceKind::Builtin,
        package_hash: package.package_hash.clone(),
        builtin: true,
    })
}

#[cfg(debug_assertions)]
fn debug_builtin_summary() -> Result<PetSummary> {
    let package = debug_builtin_package_cached()?;
    Ok(PetSummary {
        pet_ref: PetRef(BUILTIN_DEBUG_PET_REF.to_string()),
        manifest: package.manifest.clone(),
        asset_id: "builtin/hope-debug".to_string(),
        source_kind: PetSourceKind::Builtin,
        package_hash: package.package_hash.clone(),
        builtin: true,
    })
}

fn builtin_summaries() -> Result<Vec<PetSummary>> {
    let pets = vec![builtin_summary()?];
    #[cfg(debug_assertions)]
    let pets = {
        let mut pets = pets;
        pets.push(debug_builtin_summary()?);
        pets
    };
    Ok(pets)
}

pub(crate) fn acquire_library_lock() -> Result<std::fs::File> {
    let lock_path = crate::paths::pets_lock_path()?;
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::platform::try_acquire_exclusive_lock(&lock_path)?
        .ok_or_else(|| anyhow::anyhow!("pet_library_busy"))
}

fn elapsed_since_modified(path: &Path) -> Option<Duration> {
    path.metadata().ok()?.modified().ok()?.elapsed().ok()
}

fn read_bounded(path: &Path, limit: usize, code: &'static str) -> Result<Vec<u8>> {
    let length = path.metadata()?.len();
    if length == 0 || length > limit as u64 {
        anyhow::bail!(code);
    }
    let bytes = fs::read(path)?;
    if bytes.is_empty() || bytes.len() > limit {
        anyhow::bail!(code);
    }
    Ok(bytes)
}

/// Best-effort bounded cleanup while the cross-process library lock is held.
/// Only private dot-prefixed staging entries and the dedicated trash root are
/// ever removed; installed package directories are never cleanup targets.
fn cleanup_private_entries() {
    if let Ok(root) = crate::paths::pets_dir() {
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(".install-")
                    && elapsed_since_modified(&entry.path()).is_some_and(|age| age > STAGING_TTL)
                {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }

    let protected = restore_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .filter(|entry| entry.expires_at > Instant::now())
        .map(|entry| entry.trash_path.clone())
        .collect::<std::collections::HashSet<_>>();
    let Ok(trash_root) = crate::paths::pets_trash_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&trash_root) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok()),
        )
    });
    for (index, entry) in entries.into_iter().enumerate() {
        let path = entry.path();
        if protected.contains(&path) {
            continue;
        }
        let expired = elapsed_since_modified(&path).is_some_and(|age| age > TRASH_TTL);
        if expired || index >= MAX_TRASH_ENTRIES {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn read_custom_summary(dir_id: &str, dir: &Path) -> Result<PetSummary> {
    crate::paths::validate_pet_id(dir_id)?;
    let manifest_bytes = read_bounded(
        &dir.join("pet.json"),
        MAX_MANIFEST_BYTES,
        "pet_manifest_size_invalid",
    )?;
    let manifest: PetManifest = serde_json::from_slice(&manifest_bytes)?;
    super::atlas::validate_sprite_relative_path(&manifest.spritesheet_path)?;
    let metadata: HopePetMetadata = serde_json::from_slice(&read_bounded(
        &dir.join("hope.json"),
        MAX_METADATA_BYTES,
        "pet_metadata_size_invalid",
    )?)?;
    let sprite_path = dir.join(&manifest.spritesheet_path);
    let canonical_dir = dir.canonicalize()?;
    let canonical_sprite = sprite_path.canonicalize()?;
    if !canonical_sprite.starts_with(&canonical_dir) || !canonical_sprite.is_file() {
        anyhow::bail!("pet_asset_outside_package");
    }
    let sprite_len = canonical_sprite.metadata()?.len();
    if sprite_len == 0 || sprite_len > MAX_SPRITE_BYTES as u64 {
        anyhow::bail!("pet_sprite_size_invalid");
    }
    Ok(PetSummary {
        pet_ref: PetRef(format!("custom:{dir_id}")),
        manifest,
        asset_id: format!("custom/{dir_id}"),
        source_kind: metadata.source_kind,
        package_hash: metadata.package_hash,
        builtin: false,
    })
}

fn custom_summaries() -> Result<Vec<PetSummary>> {
    let root = crate::paths::pets_dir()?;
    fs::create_dir_all(&root)?;
    let mut pets = Vec::new();
    let mut skipped = 0usize;
    for entry in fs::read_dir(root)? {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !crate::paths::is_valid_pet_id(&name) {
            continue;
        }
        match read_custom_summary(&name, &entry.path()) {
            Ok(summary) => pets.push(summary),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        crate::app_warn!(
            "pet",
            "library_entry_skipped",
            "Skipped {} invalid pet library entry or entries",
            skipped
        );
    }
    pets.sort_by(|left, right| {
        left.manifest
            .display_name
            .to_lowercase()
            .cmp(&right.manifest.display_name.to_lowercase())
            .then_with(|| left.pet_ref.0.cmp(&right.pet_ref.0))
    });
    Ok(pets)
}

fn fingerprint(pets: &[PetSummary], selected: &PetRef) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(selected.0.as_bytes());
    for pet in pets {
        hasher.update(pet.pet_ref.0.as_bytes());
        hasher.update(pet.package_hash.as_bytes());
    }
    let digest = hasher.finalize();
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().unwrap_or([0; 8]))
}

pub fn list_pets() -> Result<PetLibrarySnapshot> {
    let config = crate::config::cached_config();
    let selected = config.pet.selected_pet_ref.clone();
    let mut pets = builtin_summaries()?;
    pets.extend(custom_summaries()?);
    let selected_available = pets.iter().any(|pet| pet.pet_ref == selected);
    let revision = fingerprint(&pets, &selected).max(LIBRARY_REVISION.load(Ordering::Relaxed));
    Ok(PetLibrarySnapshot {
        revision,
        selected_pet_ref: selected,
        selected_pet_available: selected_available,
        pets,
    })
}

pub(crate) fn find_duplicate(package_hash: &str) -> Result<Option<PetSummary>> {
    Ok(custom_summaries()?
        .into_iter()
        .find(|pet| pet.package_hash == package_hash))
}

fn package_for_export(pet_ref: &PetRef) -> Result<ValidatedPetPackage> {
    if pet_ref.0 == BUILTIN_DEFAULT_PET_REF {
        return builtin_package();
    }
    #[cfg(debug_assertions)]
    if pet_ref.0 == BUILTIN_DEBUG_PET_REF {
        return debug_builtin_package();
    }
    let id = pet_ref
        .custom_id()
        .ok_or_else(|| anyhow::anyhow!("pet_ref_invalid"))?;
    crate::paths::validate_pet_id(id)?;
    let root = crate::paths::pet_dir(id)?;
    let summary = read_custom_summary(id, &root)?;
    let sprite_path = root.join(&summary.manifest.spritesheet_path);
    let sprite = fs::read(sprite_path)?;
    validate_package(summary.manifest, sprite)
}

/// Build a minimal Codex-compatible package. Unknown source-manifest fields
/// are deliberately not re-emitted; the validated image bytes and known core
/// fields are the compatibility contract.
pub fn export_codex_package(pet_ref: &PetRef) -> Result<PetExportResult> {
    let package = package_for_export(pet_ref)?;
    let id = portable_pet_id(&package.manifest.id);
    let sprite_name = format!("spritesheet.{}", package.extension);
    let mut manifest = package.manifest;
    manifest.spritesheet_path = sprite_name.clone();
    let mut output = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut output);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        zip.start_file(format!("{id}/pet.json"), options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        zip.start_file(format!("{id}/{sprite_name}"), options)?;
        zip.write_all(&package.sprite_bytes)?;
        zip.finish()?;
    }
    Ok(PetExportResult {
        file_name: format!("{id}.zip"),
        mime: "application/zip".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(output.into_inner()),
    })
}

fn collision_safe_id(base: &str, package_hash: &str) -> Result<String> {
    fn prefix(value: &str, length: usize) -> String {
        value.chars().take(length).collect()
    }

    let base = portable_pet_id(base);
    if !crate::paths::pet_dir(&base)?.exists() {
        return Ok(base);
    }
    let hash = package_hash.strip_prefix("blake3:").unwrap_or(package_hash);
    let hash = prefix(hash, 8);
    let stem_len = 64usize.saturating_sub(hash.len() + 1);
    let mut candidate = format!("{}-{hash}", prefix(&base, stem_len));
    if !crate::paths::pet_dir(&candidate)?.exists() {
        return Ok(candidate);
    }
    for suffix in 2..=999u16 {
        let tail = format!("-{hash}-{suffix}");
        let keep = 64usize.saturating_sub(tail.len());
        candidate = format!("{}{tail}", prefix(&base, keep));
        if !crate::paths::pet_dir(&candidate)?.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("pet_id_collision_exhausted")
}

pub(crate) fn install_validated(
    package: &ValidatedPetPackage,
    source_kind: PetSourceKind,
    source_id: Option<String>,
) -> Result<(PetSummary, bool)> {
    if package
        .issues
        .iter()
        .any(|issue| issue.severity == super::types::PetValidationSeverity::Error)
    {
        anyhow::bail!("pet_preview_has_errors");
    }
    let _lock = acquire_library_lock()?;
    cleanup_private_entries();
    if let Some(existing) = find_duplicate(&package.package_hash)? {
        return Ok((existing, false));
    }

    let dir_id = collision_safe_id(&package.manifest.id, &package.package_hash)?;
    let root = crate::paths::pets_dir()?;
    let staging_id = format!(".install-{}", uuid::Uuid::new_v4());
    let staging = root.join(staging_id);
    fs::create_dir(&staging)?;
    let result = (|| -> Result<PetSummary> {
        let mut manifest = package.manifest.clone();
        manifest.spritesheet_path = format!("spritesheet.{}", package.extension);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let metadata = HopePetMetadata {
            schema_version: HOPE_SCHEMA_VERSION,
            source_kind: source_kind.clone(),
            source_id,
            package_hash: package.package_hash.clone(),
            asset_hash: package.asset_hash.clone(),
            imported_at: chrono::Utc::now().to_rfc3339(),
        };
        crate::platform::write_atomic(&staging.join("pet.json"), &manifest_bytes)?;
        crate::platform::write_atomic(
            &staging.join("hope.json"),
            &serde_json::to_vec_pretty(&metadata)?,
        )?;
        crate::platform::write_atomic(
            &staging.join(&manifest.spritesheet_path),
            &package.sprite_bytes,
        )?;
        if let Ok(dir) = fs::File::open(&staging) {
            let _ = dir.sync_all();
        }
        let target = crate::paths::pet_dir(&dir_id)?;
        crate::platform::publish_dir_atomic(&staging, &target)?;
        read_custom_summary(&dir_id, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let summary = result?;
    emit_library_changed();
    Ok((summary, true))
}

pub fn delete_pet(pet_ref: &PetRef, expected_package_hash: &str) -> Result<PetDeleteResult> {
    let id = pet_ref
        .custom_id()
        .ok_or_else(|| anyhow::anyhow!("builtin_pet_cannot_be_deleted"))?;
    crate::paths::validate_pet_id(id)?;
    let _lock = acquire_library_lock()?;
    // Selection writes hold this same cross-process lock through config
    // persistence. Recheck only after acquiring it so a concurrent picker
    // cannot select the package between this guard and the rename below.
    if crate::config::cached_config().pet.selected_pet_ref == *pet_ref {
        anyhow::bail!("selected_pet_cannot_be_deleted");
    }
    cleanup_private_entries();
    let source = crate::paths::pet_dir(id)?;
    let summary = read_custom_summary(id, &source)?;
    if summary.package_hash != expected_package_hash {
        anyhow::bail!("stale_pet_package");
    }
    let trash_root = crate::paths::pets_trash_dir()?;
    fs::create_dir_all(&trash_root)?;
    let trash_path = trash_root.join(format!("{}-{id}", uuid::Uuid::new_v4()));
    fs::rename(&source, &trash_path)?;
    let token = random_token();
    let mut tokens = restore_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    tokens.retain(|_, entry| entry.expires_at > now && entry.trash_path.exists());
    if tokens.len() >= MAX_RESTORE_TOKENS {
        if let Some(oldest) = tokens
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
            .map(|(token, _)| token.clone())
        {
            tokens.remove(&oldest);
        }
    }
    tokens.insert(
        token.clone(),
        RestoreEntry {
            trash_path,
            target_id: id.to_string(),
            package_hash: summary.package_hash,
            expires_at: now + RESTORE_TTL,
        },
    );
    drop(tokens);
    emit_library_changed();
    Ok(PetDeleteResult {
        restore_token: token,
        deleted_pet_ref: pet_ref.clone(),
    })
}

pub fn restore_pet(token: &str) -> Result<PetSummary> {
    let _lock = acquire_library_lock()?;
    cleanup_private_entries();
    let entry = restore_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(token)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("pet_restore_token_invalid"))?;
    if entry.expires_at <= Instant::now() || !entry.trash_path.is_dir() {
        restore_tokens()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(token);
        anyhow::bail!("pet_restore_token_expired");
    }
    let target = crate::paths::pet_dir(&entry.target_id)?;
    if target.exists() {
        anyhow::bail!("pet_restore_target_occupied");
    }
    let restored = read_custom_summary(&entry.target_id, &entry.trash_path)?;
    if restored.package_hash != entry.package_hash {
        anyhow::bail!("pet_restore_hash_mismatch");
    }
    fs::rename(&entry.trash_path, &target)?;
    restore_tokens()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(token);
    let summary = read_custom_summary(&entry.target_id, &target)?;
    emit_library_changed();
    Ok(summary)
}

pub(crate) fn emit_library_changed() {
    let revision = LIBRARY_REVISION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "pet:library_changed",
            serde_json::json!({ "revision": revision }),
        );
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn development_library_includes_the_debug_builtin() {
        let pets = builtin_summaries().expect("load builtin summaries");
        assert_eq!(pets.len(), 2);
        let debug = pets
            .iter()
            .find(|pet| pet.pet_ref.0 == BUILTIN_DEBUG_PET_REF)
            .expect("debug builtin summary");
        assert!(debug.builtin);
        assert_eq!(debug.asset_id, "builtin/hope-debug");
        assert_eq!(debug.manifest.sprite_version_number.number(), 1);
    }
}
