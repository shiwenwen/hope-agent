use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rand::RngCore;

use super::atlas::{
    idle_thumbnail_png, infer_version, parse_manifest, portable_pet_id, validate_package,
    ValidatedPetPackage, MAX_MANIFEST_BYTES, MAX_SPRITE_BYTES,
};
use super::types::{
    PetCandidatePage, PetImportCandidate, PetImportCommitRequest, PetImportCommitResult,
    PetImportPreview, PetImportPreviewRequest, PetImportSource, PetManifest, PetSourceKind,
    PetValidationIssue, PetValidationSeverity,
};

const MAX_CODEX_CANDIDATES: usize = 500;
const CANDIDATE_TTL: Duration = Duration::from_secs(30 * 60);
const PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PREVIEWS: usize = 128;
const MAX_PREVIEW_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 64;
const MAX_ARCHIVE_DEPTH: usize = 8;
const MAX_LINK_LENGTH: usize = 8 * 1024;
const MAX_LINK_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
struct CandidateEntry {
    manifest_path: PathBuf,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
enum RecheckSource {
    Manifest(PathBuf),
    Local {
        path: PathBuf,
        display_name: Option<String>,
    },
    LocalGroup(Vec<PathBuf>),
    Cached,
}

#[derive(Debug, Clone)]
struct PreviewEntry {
    package: ValidatedPetPackage,
    source_kind: PetSourceKind,
    source_id: Option<String>,
    recheck: RecheckSource,
    upload_ids: Vec<String>,
    expires_at: Instant,
}

static CANDIDATES: OnceLock<Mutex<HashMap<String, CandidateEntry>>> = OnceLock::new();
static PREVIEWS: OnceLock<Mutex<HashMap<String, PreviewEntry>>> = OnceLock::new();

fn candidates() -> &'static Mutex<HashMap<String, CandidateEntry>> {
    CANDIDATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn previews() -> &'static Mutex<HashMap<String, PreviewEntry>> {
    PREVIEWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn discard_upload_leases(upload_ids: Vec<String>) {
    let failure_count = upload_ids
        .iter()
        .filter(|upload_id| crate::file_upload::discard_upload(upload_id).is_err())
        .count();
    if failure_count > 0 {
        // Import sources and upload ids may contain user-controlled or
        // capability-bearing data. Keep this diagnostic aggregate-only.
        crate::app_warn!(
            "pet",
            "upload_cleanup_failed",
            "Failed to discard {} pet upload lease(s)",
            failure_count
        );
    }
}

fn prune_expired_previews(
    entries: &mut HashMap<String, PreviewEntry>,
    now: Instant,
) -> Vec<String> {
    let mut expired_upload_ids = Vec::new();
    entries.retain(|_, entry| {
        if entry.expires_at <= now {
            expired_upload_ids.extend(entry.upload_ids.iter().cloned());
            false
        } else {
            true
        }
    });
    expired_upload_ids
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_bounded(path: &Path, max: usize, code: &'static str) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max as u64 {
        anyhow::bail!(code);
    }
    let bytes = fs::read(path)?;
    if bytes.len() > max {
        anyhow::bail!(code);
    }
    Ok(bytes)
}

fn fallback_sprite_path(root: &Path, manifest: &mut PetManifest) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;
    let safe_candidate = |path: PathBuf| {
        let canonical = path.canonicalize().ok()?;
        (canonical.is_file() && canonical.starts_with(&canonical_root)).then_some(canonical)
    };
    if let Some(declared) = safe_candidate(root.join(&manifest.spritesheet_path)) {
        return Some(declared);
    }
    for name in ["spritesheet.webp", "spritesheet.png"] {
        if let Some(path) = safe_candidate(root.join(name)) {
            manifest.spritesheet_path = name.to_string();
            return Some(path);
        }
    }
    None
}

fn manifest_declares_version(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|object| object.contains_key("spriteVersionNumber"))
}

fn infer_missing_manifest_version(
    manifest_bytes: &[u8],
    manifest: &mut PetManifest,
    sprite_bytes: &[u8],
) {
    if manifest_declares_version(manifest_bytes) {
        return;
    }
    let dimensions = image::guess_format(sprite_bytes).ok().and_then(|format| {
        image::ImageReader::with_format(Cursor::new(sprite_bytes), format)
            .into_dimensions()
            .ok()
    });
    if let Some(version) = dimensions.and_then(|(width, height)| infer_version(width, height)) {
        manifest.sprite_version_number = version;
    }
}

fn package_from_manifest_path(path: &Path) -> Result<ValidatedPetPackage> {
    let manifest_path = path.canonicalize().context("pet_manifest_missing")?;
    if !manifest_path.is_file() {
        anyhow::bail!("pet_manifest_not_file");
    }
    let root = manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("pet_manifest_parent_missing"))?
        .canonicalize()?;
    let manifest_bytes =
        read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "pet_manifest_too_large")?;
    let fallback_id = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("pet");
    let mut manifest = parse_manifest(&manifest_bytes, fallback_id)?;
    let sprite_path = fallback_sprite_path(&root, &mut manifest)
        .ok_or_else(|| anyhow::anyhow!("pet_sprite_missing"))?;
    let canonical_sprite = sprite_path.canonicalize()?;
    if !canonical_sprite.starts_with(&root) {
        anyhow::bail!("pet_sprite_path_escape");
    }
    let sprite_bytes = read_bounded(&canonical_sprite, MAX_SPRITE_BYTES, "pet_sprite_too_large")?;
    infer_missing_manifest_version(&manifest_bytes, &mut manifest, &sprite_bytes);
    validate_package(manifest, sprite_bytes)
}

fn generated_manifest(
    file_name: &str,
    bytes: &[u8],
    display_name: Option<&str>,
) -> Result<PetManifest> {
    let format = image::guess_format(bytes).context("pet_sprite_unknown_format")?;
    let dimensions = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .context("pet_sprite_bad_header")?;
    let version = infer_version(dimensions.0, dimensions.1)
        .ok_or_else(|| anyhow::anyhow!("pet_sprite_dimensions_unsupported"))?;
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("pet");
    let name = display_name
        .map(|value| super::atlas::sanitize_text(value, 256))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| super::atlas::sanitize_text(stem, 256));
    Ok(PetManifest {
        id: portable_pet_id(&name),
        display_name: if name.is_empty() {
            "Pet".to_string()
        } else {
            name
        },
        description: None,
        sprite_version_number: version,
        spritesheet_path: file_name.to_string(),
    })
}

fn package_from_raw_image(path: &Path, display_name: Option<&str>) -> Result<ValidatedPetPackage> {
    let canonical = path.canonicalize()?;
    let bytes = read_bounded(&canonical, MAX_SPRITE_BYTES, "pet_sprite_too_large")?;
    let file_name = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("spritesheet.png");
    let manifest = generated_manifest(file_name, &bytes, display_name)?;
    validate_package(manifest, bytes)
}

fn normalize_archive_path(name: &str) -> Result<String> {
    let path = Path::new(name);
    if path.is_absolute() {
        anyhow::bail!("pet_zip_path_escape");
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                if parts.len() >= MAX_ARCHIVE_DEPTH {
                    anyhow::bail!("pet_zip_path_too_deep");
                }
                parts.push(value.to_string_lossy().to_string());
            }
            _ => anyhow::bail!("pet_zip_path_escape"),
        }
    }
    if parts.is_empty() {
        anyhow::bail!("pet_zip_empty_path");
    }
    Ok(parts.join("/"))
}

fn package_from_zip(bytes: Vec<u8>) -> Result<ValidatedPetPackage> {
    if bytes.is_empty() || bytes.len() > MAX_ARCHIVE_BYTES {
        anyhow::bail!("pet_zip_too_large");
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("pet_zip_invalid")?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        anyhow::bail!("pet_zip_too_many_entries");
    }
    let mut files = HashMap::<String, Vec<u8>>::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.is_dir() {
            continue;
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            anyhow::bail!("pet_zip_symlink_rejected");
        }
        let name = normalize_archive_path(file.name())?;
        let declared = usize::try_from(file.size()).unwrap_or(usize::MAX);
        if declared > MAX_ARCHIVE_BYTES || total.saturating_add(declared) > MAX_ARCHIVE_BYTES {
            anyhow::bail!("pet_zip_expanded_too_large");
        }
        let mut content = Vec::with_capacity(declared.min(MAX_ARCHIVE_BYTES));
        file.take((MAX_ARCHIVE_BYTES + 1) as u64)
            .read_to_end(&mut content)?;
        total = total.saturating_add(content.len());
        if total > MAX_ARCHIVE_BYTES || files.insert(name, content).is_some() {
            anyhow::bail!("pet_zip_invalid_entries");
        }
    }
    let manifests = files
        .keys()
        .filter(|name| {
            name.ends_with("/pet.json")
                || name.as_str() == "pet.json"
                || name.ends_with("/avatar.json")
                || name.as_str() == "avatar.json"
        })
        .cloned()
        .collect::<Vec<_>>();
    if manifests.len() != 1 {
        anyhow::bail!("pet_zip_manifest_ambiguous");
    }
    let manifest_name = &manifests[0];
    let root = Path::new(manifest_name)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("");
    let fallback_id = Path::new(root)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("pet");
    let manifest_bytes = files
        .get(manifest_name)
        .ok_or_else(|| anyhow::anyhow!("pet_zip_manifest_missing"))?
        .clone();
    let mut manifest = parse_manifest(&manifest_bytes, fallback_id)?;
    let declared = if root.is_empty() {
        manifest.spritesheet_path.clone()
    } else {
        format!("{root}/{}", manifest.spritesheet_path)
    };
    let mut sprite_name = normalize_archive_path(&declared)?;
    if !files.contains_key(&sprite_name) {
        let fallbacks = ["spritesheet.webp", "spritesheet.png"];
        sprite_name = fallbacks
            .iter()
            .map(|name| {
                if root.is_empty() {
                    (*name).to_string()
                } else {
                    format!("{root}/{name}")
                }
            })
            .find(|name| files.contains_key(name))
            .ok_or_else(|| anyhow::anyhow!("pet_sprite_missing"))?;
        manifest.spritesheet_path = Path::new(&sprite_name)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("spritesheet.webp")
            .to_string();
    }
    let sprite_bytes = files
        .remove(&sprite_name)
        .ok_or_else(|| anyhow::anyhow!("pet_sprite_missing"))?;
    infer_missing_manifest_version(&manifest_bytes, &mut manifest, &sprite_bytes);
    validate_package(manifest, sprite_bytes)
}

fn package_from_local(path: &Path, display_name: Option<&str>) -> Result<ValidatedPetPackage> {
    let canonical = path.canonicalize().context("pet_import_path_missing")?;
    if canonical.is_dir() {
        for name in ["pet.json", "avatar.json"] {
            let manifest = canonical.join(name);
            if manifest.is_file() {
                return package_from_manifest_path(&manifest);
            }
        }
        anyhow::bail!("pet_manifest_missing");
    }
    match canonical.extension().and_then(|value| value.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("json") => package_from_manifest_path(&canonical),
        Some(ext) if ext.eq_ignore_ascii_case("zip") => package_from_zip(read_bounded(
            &canonical,
            MAX_ARCHIVE_BYTES,
            "pet_zip_too_large",
        )?),
        Some(ext) if ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("webp") => {
            package_from_raw_image(&canonical, display_name)
        }
        _ => anyhow::bail!("pet_import_type_unsupported"),
    }
}

fn package_from_local_group(paths: &[PathBuf]) -> Result<ValidatedPetPackage> {
    if paths.is_empty() || paths.len() > MAX_ARCHIVE_ENTRIES {
        anyhow::bail!("pet_import_file_count_invalid");
    }
    if paths.len() == 1 {
        return package_from_local(&paths[0], None);
    }
    let manifests = paths
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .collect::<Vec<_>>();
    if manifests.len() != 1 {
        anyhow::bail!("pet_loose_manifest_ambiguous");
    }
    let manifest_path = manifests[0].canonicalize()?;
    let parent = manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("pet_manifest_parent_missing"))?;
    for path in paths {
        let canonical = path.canonicalize()?;
        if canonical.parent() != Some(parent) {
            anyhow::bail!("pet_loose_files_must_share_parent");
        }
    }
    package_from_manifest_path(&manifest_path)
}

fn add_uploaded_batch_bytes(current: u64, next: u64) -> Result<u64> {
    let total = current
        .checked_add(next)
        .ok_or_else(|| anyhow::anyhow!("pet_upload_total_too_large"))?;
    if total > MAX_ARCHIVE_BYTES as u64 {
        anyhow::bail!("pet_upload_total_too_large");
    }
    Ok(total)
}

fn package_from_uploaded(
    upload_ids: &[String],
    display_name: Option<&str>,
) -> Result<(ValidatedPetPackage, Vec<String>)> {
    if upload_ids.is_empty() || upload_ids.len() > MAX_ARCHIVE_ENTRIES {
        anyhow::bail!("pet_upload_count_invalid");
    }
    let mut uploaded = Vec::with_capacity(upload_ids.len());
    let mut total_declared_bytes = 0_u64;
    for id in upload_ids {
        let lease = crate::file_upload::upload_status(id)?;
        if lease.purpose != crate::file_upload::FileUploadPurpose::PetPackage {
            anyhow::bail!("file upload purpose mismatch");
        }
        if lease.state != crate::file_upload::FileUploadState::Complete
            || lease.received_bytes != lease.size_bytes
        {
            anyhow::bail!("file upload is not complete");
        }
        total_declared_bytes = add_uploaded_batch_bytes(total_declared_bytes, lease.size_bytes)?;
        uploaded.push((id.clone(), lease.file_name));
    }
    if uploaded.len() == 1 {
        let (id, name) = uploaded
            .pop()
            .ok_or_else(|| anyhow::anyhow!("pet_upload_missing"))?;
        let package = if name.to_ascii_lowercase().ends_with(".zip") {
            let (_, bytes) = crate::file_upload::read_completed_upload_with_limit(
                &id,
                crate::file_upload::FileUploadPurpose::PetPackage,
                MAX_ARCHIVE_BYTES as u64,
            )?;
            package_from_zip(bytes)?
        } else if name.to_ascii_lowercase().ends_with(".png")
            || name.to_ascii_lowercase().ends_with(".webp")
        {
            let (_, bytes) = crate::file_upload::read_completed_upload_with_limit(
                &id,
                crate::file_upload::FileUploadPurpose::PetPackage,
                MAX_SPRITE_BYTES as u64,
            )?;
            let manifest = generated_manifest(&name, &bytes, display_name)?;
            validate_package(manifest, bytes)?
        } else {
            anyhow::bail!("single_manifest_upload_requires_sprite");
        };
        return Ok((package, vec![id]));
    }

    let manifest_indexes = uploaded
        .iter()
        .enumerate()
        .filter(|(_, (_, name))| name.to_ascii_lowercase().ends_with(".json"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if manifest_indexes.len() != 1 {
        anyhow::bail!("pet_loose_manifest_ambiguous");
    }
    let manifest_index = manifest_indexes[0];
    let fallback = Path::new(&uploaded[manifest_index].1)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("pet");
    let (_, manifest_bytes) = crate::file_upload::read_completed_upload_with_limit(
        &uploaded[manifest_index].0,
        crate::file_upload::FileUploadPurpose::PetPackage,
        MAX_MANIFEST_BYTES as u64,
    )?;
    let mut manifest = parse_manifest(&manifest_bytes, fallback)?;
    let sprite_index = uploaded
        .iter()
        .enumerate()
        .filter(|(_, (_, name))| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".png") || lower.ends_with(".webp")
        })
        .filter(|(_, (_, name))| {
            Path::new(name).file_name() == Path::new(&manifest.spritesheet_path).file_name()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if sprite_index.len() != 1 {
        anyhow::bail!("pet_loose_sprite_ambiguous");
    }
    let sprite_name = uploaded[sprite_index[0]].1.clone();
    manifest.spritesheet_path = sprite_name;
    let (_, sprite_bytes) = crate::file_upload::read_completed_upload_with_limit(
        &uploaded[sprite_index[0]].0,
        crate::file_upload::FileUploadPurpose::PetPackage,
        MAX_SPRITE_BYTES as u64,
    )?;
    infer_missing_manifest_version(&manifest_bytes, &mut manifest, &sprite_bytes);
    let package = validate_package(manifest, sprite_bytes)?;
    Ok((package, uploaded.into_iter().map(|entry| entry.0).collect()))
}

fn register_preview(
    package: ValidatedPetPackage,
    source_kind: PetSourceKind,
    source_id: Option<String>,
    recheck: RecheckSource,
    upload_ids: Vec<String>,
) -> Result<PetImportPreview> {
    let duplicate_pet_ref =
        super::store::find_duplicate(&package.package_hash)?.map(|pet| pet.pet_ref);
    let token = random_token();
    let preview = PetImportPreview {
        preview_token: token.clone(),
        manifest: package.manifest.clone(),
        width: package.width,
        height: package.height,
        issues: package.issues.clone(),
        asset_hash: package.asset_hash.clone(),
        package_hash: package.package_hash.clone(),
        duplicate_pet_ref,
    };
    let mut entries = previews()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let expired_upload_ids = prune_expired_previews(&mut entries, now);
    let cached_bytes = entries.values().fold(0_usize, |total, entry| {
        total.saturating_add(entry.package.sprite_bytes.len())
    });
    let capacity = ensure_preview_capacity(entries.len(), cached_bytes, package.sprite_bytes.len());
    if capacity.is_ok() {
        entries.insert(
            token,
            PreviewEntry {
                package,
                source_kind,
                source_id,
                recheck,
                upload_ids,
                expires_at: now + PREVIEW_TTL,
            },
        );
    }
    drop(entries);
    discard_upload_leases(expired_upload_ids);
    capacity?;
    Ok(preview)
}

fn ensure_preview_capacity(
    entry_count: usize,
    cached_bytes: usize,
    incoming_bytes: usize,
) -> Result<()> {
    if entry_count >= MAX_PREVIEWS
        || cached_bytes.saturating_add(incoming_bytes) > MAX_PREVIEW_CACHE_BYTES
    {
        anyhow::bail!("pet_preview_cache_limit");
    }
    Ok(())
}

pub(crate) fn register_created_preview(package: ValidatedPetPackage) -> Result<PetImportPreview> {
    register_preview(
        package,
        PetSourceKind::Created,
        None,
        RecheckSource::Cached,
        Vec::new(),
    )
}

pub fn discover_codex_candidates() -> Result<PetCandidatePage> {
    let codex_root = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .ok_or_else(|| anyhow::anyhow!("codex_home_unavailable"))?;
    if !codex_root.exists() {
        return Ok(PetCandidatePage {
            candidates: Vec::new(),
            total: 0,
            truncated: false,
        });
    }
    let canonical_root = codex_root.canonicalize()?;
    let mut manifests = Vec::new();
    for (dir_name, manifest_name) in [("pets", "pet.json"), ("avatars", "avatar.json")] {
        let dir = canonical_root.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let manifest = entry.path().join(manifest_name);
            if manifest.is_file() {
                let Ok(canonical) = manifest.canonicalize() else {
                    continue;
                };
                if canonical.starts_with(&canonical_root) {
                    manifests.push((canonical, dir_name.to_string()));
                }
            }
        }
    }
    manifests.sort_by(|left, right| left.0.cmp(&right.0));
    let total = manifests.len();
    let truncated = total > MAX_CODEX_CANDIDATES;
    manifests.truncate(MAX_CODEX_CANDIDATES);

    let mut result = Vec::with_capacity(manifests.len());
    let now = Instant::now();
    let mut registry = candidates()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A discovery response is a complete replacement.  Clearing here keeps
    // repeated Settings refreshes bounded to MAX_CODEX_CANDIDATES instead of
    // accumulating opaque path capabilities for the full TTL.
    registry.clear();
    for (manifest_path, kind) in manifests {
        let candidate_id = random_token();
        let fallback = manifest_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or("pet");
        let mut issues = Vec::new();
        let (display_name, width, height, inferred_version) =
            match read_bounded(&manifest_path, MAX_MANIFEST_BYTES, "pet_manifest_too_large")
                .and_then(|bytes| parse_manifest(&bytes, fallback))
            {
                Ok(mut manifest) => {
                    let Some(root) = manifest_path.parent() else {
                        continue;
                    };
                    match fallback_sprite_path(root, &mut manifest).and_then(|path| {
                        let metadata = path.metadata().ok()?;
                        if !metadata.is_file()
                            || metadata.len() == 0
                            || metadata.len() > MAX_SPRITE_BYTES as u64
                        {
                            return None;
                        }
                        let file = fs::File::open(path).ok()?;
                        image::ImageReader::new(BufReader::new(file))
                            .with_guessed_format()
                            .ok()?
                            .into_dimensions()
                            .ok()
                    }) {
                        Some((width, height)) => (
                            manifest.display_name,
                            width,
                            height,
                            infer_version(width, height),
                        ),
                        None => {
                            issues.push(PetValidationIssue {
                                code: "sprite_header_invalid".to_string(),
                                severity: PetValidationSeverity::Error,
                                message: "spritesheet cannot be inspected".to_string(),
                            });
                            (manifest.display_name, 0, 0, None)
                        }
                    }
                }
                Err(_) => {
                    issues.push(PetValidationIssue {
                        code: "manifest_invalid".to_string(),
                        severity: PetValidationSeverity::Error,
                        message: "manifest cannot be parsed".to_string(),
                    });
                    (fallback.to_string(), 0, 0, None)
                }
            };
        registry.insert(
            candidate_id.clone(),
            CandidateEntry {
                manifest_path,
                expires_at: now + CANDIDATE_TTL,
            },
        );
        result.push(PetImportCandidate {
            candidate_id,
            display_name,
            source_kind: if kind == "avatars" {
                "codex_legacy"
            } else {
                "codex"
            }
            .to_string(),
            width,
            height,
            inferred_version,
            issues,
        });
    }
    Ok(PetCandidatePage {
        candidates: result,
        total: total.min(u32::MAX as usize) as u32,
        truncated,
    })
}

fn candidate_path(candidate_id: &str) -> Result<PathBuf> {
    let mut registry = candidates()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    registry.retain(|_, entry| entry.expires_at > now);
    registry
        .get(candidate_id)
        .map(|entry| entry.manifest_path.clone())
        .ok_or_else(|| anyhow::anyhow!("pet_candidate_expired"))
}

pub fn preview_thumbnail(candidate_id: &str) -> Result<Vec<u8>> {
    let path = candidate_path(candidate_id)?;
    idle_thumbnail_png(&package_from_manifest_path(&path)?)
}

pub fn preview_token_thumbnail(preview_token: &str) -> Result<Vec<u8>> {
    let (package, expired_upload_ids) = {
        let mut entries = previews()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let expired_upload_ids = prune_expired_previews(&mut entries, now);
        let package = entries
            .get(preview_token)
            .map(|entry| entry.package.clone())
            .ok_or_else(|| anyhow::anyhow!("pet_preview_expired"));
        (package, expired_upload_ids)
    };
    discard_upload_leases(expired_upload_ids);
    super::atlas::idle_animation_strip_png(&package?)
}

pub fn preview_import(request: PetImportPreviewRequest) -> Result<PetImportPreview> {
    let display_name = request.display_name.as_deref();
    match request.source {
        PetImportSource::Candidate { candidate_id } => {
            let path = candidate_path(&candidate_id)?;
            let package = package_from_manifest_path(&path)?;
            register_preview(
                package,
                PetSourceKind::Codex,
                Some(candidate_id),
                RecheckSource::Manifest(path),
                Vec::new(),
            )
        }
        PetImportSource::LocalPath { path } => {
            let path = PathBuf::from(path);
            let package = package_from_local(&path, display_name)?;
            register_preview(
                package,
                PetSourceKind::File,
                None,
                RecheckSource::Local {
                    path,
                    display_name: request.display_name,
                },
                Vec::new(),
            )
        }
        PetImportSource::LocalPaths { paths } => {
            let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
            let package = package_from_local_group(&paths)?;
            register_preview(
                package,
                PetSourceKind::File,
                None,
                RecheckSource::LocalGroup(paths),
                Vec::new(),
            )
        }
        PetImportSource::UploadedPath { upload_id } => {
            let (package, ids) = package_from_uploaded(&[upload_id], display_name)?;
            register_preview(
                package,
                PetSourceKind::File,
                None,
                RecheckSource::Cached,
                ids,
            )
        }
        PetImportSource::UploadedFiles { upload_ids } => {
            let (package, ids) = package_from_uploaded(&upload_ids, display_name)?;
            register_preview(
                package,
                PetSourceKind::File,
                None,
                RecheckSource::Cached,
                ids,
            )
        }
        PetImportSource::Link { .. } => anyhow::bail!("pet_link_preview_requires_async"),
    }
}

#[derive(Debug)]
struct LinkSpec {
    image_url: String,
    display_name: String,
    description: Option<String>,
    version: Option<super::types::PetSpriteVersion>,
}

fn parse_link_spec(raw: &str, display_name: Option<&str>) -> Result<LinkSpec> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_LINK_LENGTH {
        anyhow::bail!("pet_link_invalid");
    }
    let parsed = url::Url::parse(raw).context("pet_link_invalid")?;
    if parsed.scheme() == "https" {
        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            anyhow::bail!("pet_link_invalid");
        }
        let fallback = parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .and_then(|name| Path::new(name).file_stem())
            .and_then(|name| name.to_str())
            .unwrap_or("Pet");
        let name = display_name
            .map(|value| super::atlas::sanitize_text(value, 256))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| super::atlas::sanitize_text(fallback, 256));
        return Ok(LinkSpec {
            image_url: parsed.to_string(),
            display_name: if name.is_empty() {
                "Pet".to_string()
            } else {
                name
            },
            description: None,
            version: None,
        });
    }
    if !matches!(parsed.scheme(), "codex" | "hope-agent")
        || parsed.host_str() != Some("pets")
        || parsed.path() != "/install"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        anyhow::bail!("pet_link_shape_invalid");
    }
    let mut known = HashMap::<String, String>::new();
    for (key, value) in parsed.query_pairs() {
        if !matches!(
            key.as_ref(),
            "name" | "imageUrl" | "description" | "spriteVersionNumber"
        ) {
            continue;
        }
        if known.insert(key.into_owned(), value.into_owned()).is_some() {
            anyhow::bail!("pet_link_parameter_duplicate");
        }
    }
    let name = known
        .remove("name")
        .map(|value| super::atlas::sanitize_text(&value, 256))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("pet_link_name_missing"))?;
    let image_url = known
        .remove("imageUrl")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("pet_link_image_url_missing"))?;
    let image = url::Url::parse(&image_url).context("pet_link_image_url_invalid")?;
    if image.scheme() != "https"
        || !image.username().is_empty()
        || image.password().is_some()
        || image.fragment().is_some()
    {
        anyhow::bail!("pet_link_image_url_invalid");
    }
    let description = known
        .remove("description")
        .map(|value| super::atlas::sanitize_text(&value, 2048))
        .filter(|value| !value.is_empty());
    let version = known
        .remove("spriteVersionNumber")
        .map(|value| {
            value
                .parse::<u8>()
                .map_err(|_| anyhow::anyhow!("pet_link_version_invalid"))
                .and_then(|value| {
                    super::types::PetSpriteVersion::try_from(value)
                        .map_err(|_| anyhow::anyhow!("pet_link_version_invalid"))
                })
        })
        .transpose()?;
    Ok(LinkSpec {
        image_url,
        display_name: name,
        description,
        version,
    })
}

async fn download_link_image(raw: &str) -> Result<Vec<u8>> {
    let mut next =
        crate::security::ssrf::check_url(raw, crate::security::ssrf::SsrfPolicy::Strict, &[])
            .await?;
    if next.scheme() != "https" {
        anyhow::bail!("pet_link_https_required");
    }
    let client = crate::provider::apply_proxy_for_url(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30)),
        next.as_str(),
    )
    .build()?;
    for _ in 0..=MAX_LINK_REDIRECTS {
        let response = client.get(next.clone()).send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("pet_link_redirect_invalid"))?;
            let redirected = response.url().join(location)?;
            if redirected.scheme() != "https" {
                anyhow::bail!("pet_link_https_required");
            }
            next = crate::security::ssrf::check_url(
                redirected.as_str(),
                crate::security::ssrf::SsrfPolicy::Strict,
                &[],
            )
            .await?;
            continue;
        }
        if !response.status().is_success() {
            anyhow::bail!("pet_link_download_failed");
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SPRITE_BYTES as u64)
        {
            anyhow::bail!("pet_sprite_too_large");
        }
        if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
            let content_type = content_type.to_str().unwrap_or_default();
            if !content_type.starts_with("image/")
                && !content_type.starts_with("application/octet-stream")
            {
                anyhow::bail!("pet_link_content_type_invalid");
            }
        }
        let bytes =
            crate::security::http_stream::read_bytes_capped(response, MAX_SPRITE_BYTES + 1).await?;
        if bytes.is_empty() || bytes.len() > MAX_SPRITE_BYTES {
            anyhow::bail!("pet_sprite_too_large");
        }
        return Ok(bytes);
    }
    anyhow::bail!("pet_link_too_many_redirects")
}

pub async fn preview_import_async(request: PetImportPreviewRequest) -> Result<PetImportPreview> {
    let PetImportPreviewRequest {
        source,
        display_name,
    } = request;
    match source {
        PetImportSource::Link { link } => {
            let spec = parse_link_spec(&link, display_name.as_deref())?;
            let bytes = download_link_image(&spec.image_url).await?;
            let mut manifest =
                generated_manifest("spritesheet.png", &bytes, Some(&spec.display_name))?;
            manifest.description = spec.description;
            if let Some(version) = spec.version {
                manifest.sprite_version_number = version;
            }
            let package = validate_package(manifest, bytes)?;
            crate::blocking::run_blocking(move || {
                register_preview(
                    package,
                    PetSourceKind::Url,
                    None,
                    RecheckSource::Cached,
                    Vec::new(),
                )
            })
            .await
        }
        source => {
            crate::blocking::run_blocking(move || {
                preview_import(PetImportPreviewRequest {
                    source,
                    display_name,
                })
            })
            .await
        }
    }
}

fn recheck(entry: &PreviewEntry) -> Result<()> {
    let current = match &entry.recheck {
        RecheckSource::Manifest(path) => Some(package_from_manifest_path(path)?),
        RecheckSource::Local { path, display_name } => {
            Some(package_from_local(path, display_name.as_deref())?)
        }
        RecheckSource::LocalGroup(paths) => Some(package_from_local_group(paths)?),
        RecheckSource::Cached => None,
    };
    if current
        .as_ref()
        .is_some_and(|package| package.package_hash != entry.package.package_hash)
    {
        anyhow::bail!("stale_preview");
    }
    Ok(())
}

pub async fn commit_import(request: PetImportCommitRequest) -> Result<PetImportCommitResult> {
    let (entry, expired_upload_ids) = {
        let mut entries = previews()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        let expired_upload_ids = prune_expired_previews(&mut entries, now);
        (
            entries.get(&request.preview_token).cloned(),
            expired_upload_ids,
        )
    };
    if !expired_upload_ids.is_empty() {
        crate::blocking::run_blocking(move || {
            discard_upload_leases(expired_upload_ids);
            Ok::<_, anyhow::Error>(())
        })
        .await?;
    }
    let entry = entry.ok_or_else(|| anyhow::anyhow!("pet_preview_expired"))?;
    if entry.expires_at <= Instant::now() {
        anyhow::bail!("pet_preview_expired");
    }
    let install_entry = entry.clone();
    let (pet, imported) = crate::blocking::run_blocking(move || -> Result<_> {
        recheck(&install_entry)?;
        super::store::install_validated(
            &install_entry.package,
            install_entry.source_kind,
            install_entry.source_id,
        )
    })
    .await?;
    if request.enable_after_import {
        super::update_config(Some(true), Some(pet.pet_ref.clone()), "pet-import").await?;
    }
    // Consume the preview only after every requested side effect succeeds. If
    // config persistence fails after the package is installed, the same token
    // can safely retry (installation is content-addressed and idempotent).
    previews()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&request.preview_token);
    crate::blocking::run_blocking(move || {
        discard_upload_leases(entry.upload_ids);
        Ok::<_, anyhow::Error>(())
    })
    .await?;
    Ok(PetImportCommitResult { pet, imported })
}

/// Cancel a preview and release every upload lease retained by it. The token
/// is intentionally idempotent so a close racing a successful commit is safe.
pub async fn cancel_import_preview(preview_token: String) -> Result<bool> {
    let entry = previews()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&preview_token);
    let Some(entry) = entry else {
        return Ok(false);
    };
    crate::blocking::run_blocking(move || {
        discard_upload_leases(entry.upload_ids);
        Ok::<_, anyhow::Error>(())
    })
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_paths_fail_closed() {
        for invalid in ["../pet.json", "/pet.json", "a/../../pet.json"] {
            assert!(normalize_archive_path(invalid).is_err());
        }
        assert_eq!(
            normalize_archive_path("hope/pet.json").unwrap(),
            "hope/pet.json"
        );
    }

    #[test]
    fn preview_tokens_have_256_bits_of_hex() {
        let token = random_token();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn uploaded_loose_files_share_one_archive_sized_budget() {
        let half = MAX_ARCHIVE_BYTES as u64 / 2;
        assert_eq!(add_uploaded_batch_bytes(half, half).unwrap(), half * 2);
        assert!(add_uploaded_batch_bytes(half, half + 1).is_err());
        assert!(add_uploaded_batch_bytes(u64::MAX, 1).is_err());
    }

    #[test]
    fn preview_capacity_never_requires_evicting_a_live_token() {
        assert!(ensure_preview_capacity(MAX_PREVIEWS - 1, 0, 1).is_ok());
        assert!(ensure_preview_capacity(MAX_PREVIEWS, 0, 1).is_err());
        assert!(ensure_preview_capacity(1, MAX_PREVIEW_CACHE_BYTES, 1).is_err());
    }

    #[test]
    fn codex_and_hope_install_links_share_the_strict_parser() {
        for scheme in ["codex", "hope-agent"] {
            let spec = parse_link_spec(
                &format!(
                    "{scheme}://pets/install?name=Firefly&imageUrl=https%3A%2F%2Fexample.com%2Fpet.png&spriteVersionNumber=2"
                ),
                None,
            )
            .unwrap();
            assert_eq!(spec.display_name, "Firefly");
            assert_eq!(spec.image_url, "https://example.com/pet.png");
            assert_eq!(
                spec.version,
                Some(super::super::types::PetSpriteVersion::V2)
            );
        }
    }

    #[test]
    fn install_links_reject_ambiguous_or_unsafe_parameters() {
        for invalid in [
            "codex://pets/install?name=A&name=B&imageUrl=https%3A%2F%2Fexample.com%2Fpet.png",
            "codex://pets/install?name=A&imageUrl=http%3A%2F%2Fexample.com%2Fpet.png",
            "codex://pets/install?name=A&imageUrl=https%3A%2F%2Fuser%40example.com%2Fpet.png",
            "codex://pets/install?name=A&imageUrl=https%3A%2F%2Fexample.com%2Fpet.png&spriteVersionNumber=3",
        ] {
            assert!(parse_link_spec(invalid, None).is_err(), "accepted {invalid}");
        }
    }
}
