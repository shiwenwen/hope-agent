use std::io::Cursor;
use std::path::{Component, Path};

use anyhow::{Context, Result};

use super::types::{PetManifest, PetSpriteVersion, PetValidationIssue, PetValidationSeverity};

pub const CELL_WIDTH: u32 = 192;
pub const CELL_HEIGHT: u32 = 208;
pub const ATLAS_WIDTH: u32 = CELL_WIDTH * 8;
pub const V1_HEIGHT: u32 = CELL_HEIGHT * 9;
pub const V2_HEIGHT: u32 = CELL_HEIGHT * 11;
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const MAX_SPRITE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ValidatedPetPackage {
    pub manifest: PetManifest,
    pub sprite_bytes: Vec<u8>,
    pub extension: &'static str,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub asset_hash: String,
    pub package_hash: String,
    pub issues: Vec<PetValidationIssue>,
}

pub fn infer_version(width: u32, height: u32) -> Option<PetSpriteVersion> {
    match (width, height) {
        (ATLAS_WIDTH, V1_HEIGHT) => Some(PetSpriteVersion::V1),
        (ATLAS_WIDTH, V2_HEIGHT) => Some(PetSpriteVersion::V2),
        _ => None,
    }
}

pub fn sanitize_text(value: &str, max_scalars: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_scalars)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn portable_pet_id(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(64));
    let mut last_dash = false;
    for ch in value.chars() {
        if out.len() >= 64 {
            break;
        }
        let normalized = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_' | ' ') {
            Some('-')
        } else {
            None
        };
        if let Some(ch) = normalized {
            if ch == '-' {
                if out.is_empty() || last_dash {
                    continue;
                }
                last_dash = true;
            } else {
                last_dash = false;
            }
            out.push(ch);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        let digest = blake3::hash(value.as_bytes());
        let prefix = digest.to_hex().chars().take(12).collect::<String>();
        format!("pet-{prefix}")
    } else {
        out
    }
}

pub fn validate_sprite_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        anyhow::bail!("invalid spritesheetPath");
    }
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            _ => anyhow::bail!("invalid spritesheetPath"),
        }
    }
    if depth == 0 || depth > 8 {
        anyhow::bail!("invalid spritesheetPath depth");
    }
    Ok(())
}

pub fn parse_manifest(bytes: &[u8], fallback_id: &str) -> Result<PetManifest> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("pet_manifest_too_large");
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("pet_manifest_invalid_json")?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("pet_manifest_not_object"))?;

    let raw_id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback_id);
    let id = sanitize_text(raw_id, 256);
    let id = if id.is_empty() {
        sanitize_text(fallback_id, 256)
    } else {
        id
    };
    let display_name = sanitize_text(
        object
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&id),
        256,
    );
    let description = object
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(|value| sanitize_text(value, 2048))
        .filter(|value| !value.is_empty());
    let version_number = object
        .get("spriteVersionNumber")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    let version_number =
        u8::try_from(version_number).map_err(|_| anyhow::anyhow!("pet_sprite_version_invalid"))?;
    let sprite_version_number = PetSpriteVersion::try_from(version_number)
        .map_err(|_| anyhow::anyhow!("pet_sprite_version_invalid"))?;
    let spritesheet_path = sanitize_text(
        object
            .get("spritesheetPath")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("spritesheet.webp"),
        1024,
    );
    validate_sprite_relative_path(&spritesheet_path)?;

    Ok(PetManifest {
        id,
        display_name: if display_name.is_empty() {
            "Pet".to_string()
        } else {
            display_name
        },
        description,
        sprite_version_number,
        spritesheet_path,
    })
}

pub fn validate_package(
    mut manifest: PetManifest,
    sprite_bytes: Vec<u8>,
) -> Result<ValidatedPetPackage> {
    if sprite_bytes.is_empty() || sprite_bytes.len() > MAX_SPRITE_BYTES {
        anyhow::bail!("pet_sprite_size_invalid");
    }
    let format = image::guess_format(&sprite_bytes).context("pet_sprite_unknown_format")?;
    let (extension, mime) = match format {
        image::ImageFormat::Png => ("png", "image/png"),
        image::ImageFormat::WebP => ("webp", "image/webp"),
        _ => anyhow::bail!("pet_sprite_unsupported_format"),
    };

    let reader = image::ImageReader::with_format(Cursor::new(&sprite_bytes), format);
    let (width, height) = reader.into_dimensions().context("pet_sprite_bad_header")?;
    let inferred = infer_version(width, height);
    let mut issues = Vec::new();
    match inferred {
        Some(actual) if actual != manifest.sprite_version_number => {
            issues.push(PetValidationIssue {
                code: "sprite_version_dimension_mismatch".to_string(),
                severity: PetValidationSeverity::Error,
                message: format!(
                    "manifest declares v{} but dimensions match v{}",
                    manifest.sprite_version_number.number(),
                    actual.number()
                ),
            });
        }
        Some(_) => {}
        None => issues.push(PetValidationIssue {
            code: "sprite_dimensions_unsupported".to_string(),
            severity: PetValidationSeverity::Error,
            message: format!(
                "expected {ATLAS_WIDTH}x{V1_HEIGHT} or {ATLAS_WIDTH}x{V2_HEIGHT}, got {width}x{height}"
            ),
        }),
    }

    // A full decode after the exact dimension gate catches truncated/corrupt
    // files while bounding decoded memory to the known v1/v2 atlas size.
    if inferred.is_some() {
        let decoded = image::load_from_memory_with_format(&sprite_bytes, format)
            .context("pet_sprite_decode_failed")?;
        if !decoded.color().has_alpha() {
            issues.push(PetValidationIssue {
                code: "sprite_missing_alpha".to_string(),
                severity: PetValidationSeverity::Warning,
                message: "spritesheet has no alpha channel".to_string(),
            });
        }
    }

    manifest.spritesheet_path = format!("spritesheet.{extension}");
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let asset_hash = format!("blake3:{}", blake3::hash(&sprite_bytes).to_hex());
    let mut package_hasher = blake3::Hasher::new();
    package_hasher.update(&manifest_bytes);
    package_hasher.update(&[0]);
    package_hasher.update(asset_hash.as_bytes());
    let package_hash = format!("blake3:{}", package_hasher.finalize().to_hex());

    Ok(ValidatedPetPackage {
        manifest,
        sprite_bytes,
        extension,
        mime,
        width,
        height,
        asset_hash,
        package_hash,
        issues,
    })
}

pub fn idle_thumbnail_png(package: &ValidatedPetPackage) -> Result<Vec<u8>> {
    let format = match package.extension {
        "png" => image::ImageFormat::Png,
        "webp" => image::ImageFormat::WebP,
        _ => anyhow::bail!("pet_sprite_unsupported_format"),
    };
    let image = image::load_from_memory_with_format(&package.sprite_bytes, format)?;
    let idle = image.crop_imm(0, 0, CELL_WIDTH, CELL_HEIGHT);
    let mut bytes = Cursor::new(Vec::new());
    idle.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

/// A bounded eight-cell idle-row preview. It is small enough for the
/// Settings IPC path while still exercising animation cadence and cell
/// alignment before installation; returning the full 9/11-row atlas would be
/// needlessly expensive for large imported WebP files.
pub fn idle_animation_strip_png(package: &ValidatedPetPackage) -> Result<Vec<u8>> {
    let format = match package.extension {
        "png" => image::ImageFormat::Png,
        "webp" => image::ImageFormat::WebP,
        _ => anyhow::bail!("pet_sprite_unsupported_format"),
    };
    let image = image::load_from_memory_with_format(&package.sprite_bytes, format)?;
    let idle_row = image.crop_imm(0, 0, ATLAS_WIDTH, CELL_HEIGHT);
    let mut bytes = Cursor::new(Vec::new());
    idle_row.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_match_codex_dimensions() {
        assert_eq!(infer_version(1536, 1872), Some(PetSpriteVersion::V1));
        assert_eq!(infer_version(1536, 2288), Some(PetSpriteVersion::V2));
        assert_eq!(infer_version(1024, 1024), None);
    }

    #[test]
    fn portable_ids_never_become_paths() {
        assert_eq!(portable_pet_id("Hope Pet"), "hope-pet");
        assert!(crate::paths::is_valid_pet_id(&portable_pet_id("../宠物")));
    }

    #[test]
    fn sprite_paths_reject_escape() {
        for invalid in ["", "../x.png", "/tmp/x.png", "a/../../x.png"] {
            assert!(validate_sprite_relative_path(invalid).is_err());
        }
        assert!(validate_sprite_relative_path("spritesheet.webp").is_ok());
    }
}
