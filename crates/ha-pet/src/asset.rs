use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::atlas::{MAX_MANIFEST_BYTES, MAX_SPRITE_BYTES};
use super::types::{HopePetMetadata, InstalledSprite, PetManifest};

const MAX_METADATA_BYTES: usize = 64 * 1024;

fn read_bounded(path: &std::path::Path, limit: usize, code: &'static str) -> Result<Vec<u8>> {
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

fn custom_sprite(id: &str) -> Result<(PathBuf, &'static str, String)> {
    ha_core::paths::validate_pet_id(id)?;
    let dir = ha_core::paths::pet_dir(id)?;
    let canonical_dir = dir.canonicalize().context("pet_package_missing")?;
    let manifest: PetManifest = serde_json::from_slice(&read_bounded(
        &dir.join("pet.json"),
        MAX_MANIFEST_BYTES,
        "pet_manifest_size_invalid",
    )?)?;
    super::atlas::validate_sprite_relative_path(&manifest.spritesheet_path)?;
    let sprite = dir.join(&manifest.spritesheet_path);
    let canonical_sprite = sprite.canonicalize().context("pet_sprite_missing")?;
    if !canonical_sprite.starts_with(&canonical_dir) || !canonical_sprite.is_file() {
        anyhow::bail!("pet_asset_outside_package");
    }
    let len = canonical_sprite.metadata()?.len();
    if len == 0 || len > MAX_SPRITE_BYTES as u64 {
        anyhow::bail!("pet_sprite_size_invalid");
    }
    let mime = match canonical_sprite.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("png") => "image/png",
        Some(ext) if ext.eq_ignore_ascii_case("webp") => "image/webp",
        _ => anyhow::bail!("pet_sprite_unsupported_format"),
    };
    let metadata: HopePetMetadata = serde_json::from_slice(&read_bounded(
        &dir.join("hope.json"),
        MAX_METADATA_BYTES,
        "pet_metadata_size_invalid",
    )?)?;
    Ok((canonical_sprite, mime, metadata.asset_hash))
}

pub fn resolve_installed_sprite(asset_id: &str) -> Result<InstalledSprite> {
    if asset_id == "builtin/hope-default" {
        let package = super::store::builtin_package_cached()?;
        return Ok(InstalledSprite {
            path: None,
            mime: package.mime,
            etag: package.asset_hash.clone(),
        });
    }
    #[cfg(debug_assertions)]
    if asset_id == "builtin/hope-debug" {
        let package = super::store::debug_builtin_package_cached()?;
        return Ok(InstalledSprite {
            path: None,
            mime: package.mime,
            etag: package.asset_hash.clone(),
        });
    }
    let id = asset_id
        .strip_prefix("custom/")
        .ok_or_else(|| anyhow::anyhow!("pet_asset_id_invalid"))?;
    let (path, mime, etag) = custom_sprite(id)?;
    Ok(InstalledSprite {
        path: Some(path),
        mime,
        etag,
    })
}

pub fn read_installed_sprite(asset_id: &str) -> Result<(Vec<u8>, InstalledSprite)> {
    let descriptor = resolve_installed_sprite(asset_id)?;
    let bytes = match descriptor.path.as_ref() {
        Some(path) => fs::read(path)?,
        None if asset_id == "builtin/hope-default" => {
            super::store::builtin_package_cached()?.sprite_bytes.clone()
        }
        #[cfg(debug_assertions)]
        None if asset_id == "builtin/hope-debug" => super::store::debug_builtin_package_cached()?
            .sprite_bytes
            .clone(),
        None => anyhow::bail!("pet_asset_missing"),
    };
    if bytes.is_empty() || bytes.len() > MAX_SPRITE_BYTES {
        anyhow::bail!("pet_sprite_size_invalid");
    }
    Ok((bytes, descriptor))
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn development_debug_sprite_is_embedded_and_valid() {
        let (bytes, descriptor) =
            read_installed_sprite("builtin/hope-debug").expect("read debug sprite");
        assert_eq!(descriptor.mime, "image/png");
        assert!(!descriptor.etag.is_empty());
        assert!(!bytes.is_empty());
        let image = image::load_from_memory(&bytes).expect("decode debug sprite");
        assert_eq!(image.width(), super::super::atlas::ATLAS_WIDTH);
        assert_eq!(image.height(), super::super::atlas::V1_HEIGHT);
    }
}
