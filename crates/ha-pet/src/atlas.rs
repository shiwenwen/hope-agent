use std::io::Cursor;
use std::path::{Component, Path};

use anyhow::{Context, Result};

use super::types::{PetManifest, PetSpriteVersion, PetValidationIssue, PetValidationSeverity};

pub const CELL_WIDTH: u32 = 192;
pub const CELL_HEIGHT: u32 = 208;
pub const ATLAS_WIDTH: u32 = CELL_WIDTH * 8;
pub const V1_HEIGHT: u32 = CELL_HEIGHT * 9;
pub const V2_HEIGHT: u32 = CELL_HEIGHT * 11;
pub const LOOK_DIRECTION_COUNT: usize = 16;
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

/// Upgrade a validated Codex v1 atlas to v2 without changing its nine action
/// rows. The appended cells follow the v2 direction contract: index 0 looks
/// up and the remaining cells advance clockwise through rows 9 and 10.
///
/// A v1 package does not contain separate head poses, so the deterministic
/// upgrader derives a conservative look loop from the most populated idle
/// frame. The upper silhouette bends toward each direction while the lower
/// body stays anchored; the full cell is never translated or mirrored.
pub fn upgrade_v1_atlas_to_v2(sprite_bytes: &[u8]) -> Result<Vec<u8>> {
    if sprite_bytes.is_empty() || sprite_bytes.len() > MAX_SPRITE_BYTES {
        anyhow::bail!("pet_sprite_size_invalid");
    }
    let format = image::guess_format(sprite_bytes).context("pet_sprite_unknown_format")?;
    let source = image::load_from_memory_with_format(sprite_bytes, format)?.to_rgba8();
    if source.dimensions() != (ATLAS_WIDTH, V1_HEIGHT) {
        anyhow::bail!("pet_upgrade_requires_v1");
    }

    let mut upgraded = image::RgbaImage::new(ATLAS_WIDTH, V2_HEIGHT);
    image::imageops::replace(&mut upgraded, &source, 0, 0);
    let idle = (0..8)
        .map(|column| {
            let cell =
                image::imageops::crop_imm(&source, column * CELL_WIDTH, 0, CELL_WIDTH, CELL_HEIGHT)
                    .to_image();
            let visible = cell.pixels().filter(|pixel| pixel[3] > 8).count();
            (visible, cell)
        })
        .max_by_key(|(visible, _)| *visible)
        .map(|(_, cell)| cell)
        .ok_or_else(|| anyhow::anyhow!("pet_upgrade_idle_missing"))?;
    if !idle.pixels().any(|pixel| pixel[3] > 8) {
        anyhow::bail!("pet_upgrade_idle_missing");
    }
    let (visible_top, visible_bottom) = visible_vertical_bounds(&idle)
        .ok_or_else(|| anyhow::anyhow!("pet_upgrade_idle_missing"))?;
    const OFFSETS: [(i32, i32); LOOK_DIRECTION_COUNT] = [
        (0, -6),
        (2, -6),
        (4, -4),
        (6, -2),
        (6, 0),
        (6, 2),
        (4, 4),
        (2, 6),
        (0, 6),
        (-2, 6),
        (-4, 4),
        (-6, 2),
        (-6, 0),
        (-6, -2),
        (-4, -4),
        (-2, -6),
    ];

    for (direction, (dx, dy)) in OFFSETS.into_iter().enumerate() {
        let frame = synthesize_look_frame(&idle, visible_top, visible_bottom, dx, dy);
        let target_row = 9 + direction as u32 / 8;
        let target_column = direction as u32 % 8;
        image::imageops::replace(
            &mut upgraded,
            &frame,
            (target_column * CELL_WIDTH).into(),
            (target_row * CELL_HEIGHT).into(),
        );
    }

    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(upgraded).write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn visible_vertical_bounds(cell: &image::RgbaImage) -> Option<(u32, u32)> {
    let mut top = CELL_HEIGHT;
    let mut bottom = 0;
    let mut visible = false;
    for (_, y, pixel) in cell.enumerate_pixels() {
        if pixel[3] <= 8 {
            continue;
        }
        visible = true;
        top = top.min(y);
        bottom = bottom.max(y);
    }
    visible.then_some((top, bottom))
}

fn rounded_scaled_offset(offset: i32, numerator: u32, denominator: u32) -> i32 {
    if numerator == 0 || offset == 0 {
        return 0;
    }
    let scaled = offset * numerator as i32;
    let half = denominator as i32 / 2;
    if scaled > 0 {
        (scaled + half) / denominator as i32
    } else {
        (scaled - half) / denominator as i32
    }
}

fn synthesize_look_frame(
    cell: &image::RgbaImage,
    visible_top: u32,
    visible_bottom: u32,
    dx: i32,
    dy: i32,
) -> image::RgbaImage {
    let visible_height = visible_bottom.saturating_sub(visible_top) + 1;
    // Keep the top 40% fully directional, then taper the displacement to zero
    // at 75% of the visible height. Pixels below that anchor remain identical
    // in every direction, avoiding the whole-pet jitter produced by translating
    // an entire cell.
    let full_shift_end = visible_top + visible_height * 2 / 5;
    let anchor_y = visible_top + visible_height * 3 / 4;
    let taper_height = anchor_y.saturating_sub(full_shift_end).max(1);
    let mut frame = image::RgbaImage::new(CELL_WIDTH, CELL_HEIGHT);

    for y in 0..CELL_HEIGHT {
        let numerator = if y <= full_shift_end {
            taper_height
        } else if y >= anchor_y {
            0
        } else {
            anchor_y - y
        };
        let shift_x = rounded_scaled_offset(dx, numerator, taper_height);
        let shift_y = rounded_scaled_offset(dy, numerator, taper_height);
        for x in 0..CELL_WIDTH {
            let source_x = x as i32 - shift_x;
            let source_y = y as i32 - shift_y;
            if !(0..CELL_WIDTH as i32).contains(&source_x)
                || !(0..CELL_HEIGHT as i32).contains(&source_y)
            {
                continue;
            }
            let mut pixel = *cell.get_pixel(source_x as u32, source_y as u32);
            if pixel[3] == 0 {
                pixel = image::Rgba([0, 0, 0, 0]);
            }
            frame.put_pixel(x, y, pixel);
        }
    }
    frame
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
    fn v1_upgrade_preserves_actions_and_appends_clockwise_look_rows() {
        let source = image::RgbaImage::from_fn(ATLAS_WIDTH, V1_HEIGHT, |x, y| {
            let local_x = x % CELL_WIDTH;
            if x == 0 && y == 0 {
                // Fully transparent RGB residue is still part of the original
                // v1 pixel data and must survive in the preserved action rows.
                image::Rgba([200, 10, 30, 0])
            } else if (CELL_WIDTH..CELL_WIDTH * 2).contains(&x)
                && (76..116).contains(&local_x)
                && (20..70).contains(&y)
            {
                image::Rgba([20, 80, 160, 255])
            } else if (CELL_WIDTH..CELL_WIDTH * 2).contains(&x)
                && (80..112).contains(&local_x)
                && (70..190).contains(&y)
            {
                image::Rgba([180, 60, 30, 255])
            } else if y < CELL_HEIGHT {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([(x % 251) as u8, (y % 241) as u8, 40, 255])
            }
        });
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source.clone())
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();

        let upgraded = upgrade_v1_atlas_to_v2(&encoded.into_inner()).unwrap();
        let upgraded = image::load_from_memory(&upgraded).unwrap().to_rgba8();
        assert_eq!(upgraded.dimensions(), (ATLAS_WIDTH, V2_HEIGHT));
        assert_eq!(
            image::imageops::crop_imm(&upgraded, 0, 0, ATLAS_WIDTH, V1_HEIGHT).to_image(),
            source
        );
        let idle =
            image::imageops::crop_imm(&source, CELL_WIDTH, 0, CELL_WIDTH, CELL_HEIGHT).to_image();
        let mut look_frames = Vec::new();
        for direction in 0..LOOK_DIRECTION_COUNT {
            let row = 9 + direction as u32 / 8;
            let column = direction as u32 % 8;
            let cell = image::imageops::crop_imm(
                &upgraded,
                column * CELL_WIDTH,
                row * CELL_HEIGHT,
                CELL_WIDTH,
                CELL_HEIGHT,
            )
            .to_image();
            assert!(cell.pixels().any(|pixel| pixel[3] > 0));
            assert_eq!(
                image::imageops::crop_imm(&cell, 0, 155, CELL_WIDTH, CELL_HEIGHT - 155).to_image(),
                image::imageops::crop_imm(&idle, 0, 155, CELL_WIDTH, CELL_HEIGHT - 155).to_image(),
                "direction {direction} moved the anchored lower body"
            );
            look_frames.push(cell);
        }

        fn color_centroid(cell: &image::RgbaImage, expected: [u8; 4]) -> (f32, f32) {
            let mut count = 0_u32;
            let mut x_sum = 0_u32;
            let mut y_sum = 0_u32;
            for (x, y, pixel) in cell.enumerate_pixels() {
                if pixel.0 == expected {
                    count += 1;
                    x_sum += x;
                    y_sum += y;
                }
            }
            assert!(count > 0);
            (x_sum as f32 / count as f32, y_sum as f32 / count as f32)
        }

        let blue = [20, 80, 160, 255];
        let up = color_centroid(&look_frames[0], blue);
        let right = color_centroid(&look_frames[4], blue);
        let down = color_centroid(&look_frames[8], blue);
        let left = color_centroid(&look_frames[12], blue);
        assert!(up.1 < down.1, "up/down poses did not move the upper body");
        assert!(
            right.0 > left.0,
            "left/right poses did not move the upper body"
        );
    }

    #[test]
    fn portable_ids_never_become_paths() {
        assert_eq!(portable_pet_id("Hope Pet"), "hope-pet");
        assert!(ha_core::paths::is_valid_pet_id(&portable_pet_id("../宠物")));
    }

    #[test]
    fn sprite_paths_reject_escape() {
        for invalid in ["", "../x.png", "/tmp/x.png", "a/../../x.png"] {
            assert!(validate_sprite_relative_path(invalid).is_err());
        }
        assert!(validate_sprite_relative_path("spritesheet.webp").is_ok());
    }
}
