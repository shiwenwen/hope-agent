use std::collections::VecDeque;
use std::io::Cursor;

use anyhow::{Context, Result};
use image::{ImageFormat, Rgba, RgbaImage};

use super::atlas::{
    portable_pet_id, sanitize_text, validate_package, ATLAS_WIDTH, CELL_HEIGHT, CELL_WIDTH,
    V1_HEIGHT,
};
use super::types::{PetCreateRequest, PetImportPreview, PetManifest, PetSpriteVersion};

const MAX_CREATE_PROMPT_SCALARS: usize = 4_000;
const MAX_GENERATED_SOURCE_DIMENSION: u32 = 4_096;
const BACKGROUND_DISTANCE: i32 = 58;

fn color_distance(left: Rgba<u8>, right: Rgba<u8>) -> i32 {
    let dr = i32::from(left[0]) - i32::from(right[0]);
    let dg = i32::from(left[1]) - i32::from(right[1]);
    let db = i32::from(left[2]) - i32::from(right[2]);
    dr * dr + dg * dg + db * db
}

/// Remove only background-colored pixels connected to an image edge. This
/// keeps similarly colored details inside the mascot intact and gives opaque
/// provider outputs a useful transparent fallback without pretending to be a
/// general segmentation model.
fn remove_edge_background(image: &mut RgbaImage) {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return;
    }
    let transparent = image.pixels().filter(|pixel| pixel[3] < 245).count();
    if transparent.saturating_mul(100) > image.len() {
        return;
    }
    let corners = [
        *image.get_pixel(0, 0),
        *image.get_pixel(width - 1, 0),
        *image.get_pixel(0, height - 1),
        *image.get_pixel(width - 1, height - 1),
    ];
    let corner_limit = BACKGROUND_DISTANCE * BACKGROUND_DISTANCE;
    if corners
        .iter()
        .any(|color| color_distance(*color, corners[0]) > corner_limit)
    {
        return;
    }
    let background = Rgba([
        (corners.iter().map(|value| u32::from(value[0])).sum::<u32>() / 4) as u8,
        (corners.iter().map(|value| u32::from(value[1])).sum::<u32>() / 4) as u8,
        (corners.iter().map(|value| u32::from(value[2])).sum::<u32>() / 4) as u8,
        255,
    ]);
    let count = usize::try_from(width).ok().and_then(|width| {
        usize::try_from(height)
            .ok()
            .and_then(|height| width.checked_mul(height))
    });
    let Some(count) = count else { return };
    let mut seen = vec![false; count];
    let mut queue = VecDeque::new();
    for x in 0..width {
        queue.push_back((x, 0));
        queue.push_back((x, height - 1));
    }
    for y in 0..height {
        queue.push_back((0, y));
        queue.push_back((width - 1, y));
    }
    let limit = BACKGROUND_DISTANCE * BACKGROUND_DISTANCE;
    while let Some((x, y)) = queue.pop_front() {
        let index = y as usize * width as usize + x as usize;
        if seen[index] {
            continue;
        }
        seen[index] = true;
        let pixel = *image.get_pixel(x, y);
        if color_distance(pixel, background) > limit {
            continue;
        }
        image.get_pixel_mut(x, y)[3] = 0;
        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < width {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < height {
            queue.push_back((x, y + 1));
        }
    }
}

fn content_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (width, height) = image.dimensions();
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] <= 8 {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    found.then_some((min_x, min_y, max_x + 1, max_y + 1))
}

fn pose(row: u32, frame: u32) -> (i32, i32, f32, bool) {
    let bob = [0, -2, -4, -2, 0, 1, 0, -1][frame as usize];
    match row {
        0 => (0, bob, 1.0, false),
        1 => ([-7, -5, -3, 0, 3, 5, 7, 3][frame as usize], bob, 1.0, false),
        2 => ([7, 5, 3, 0, -3, -5, -7, -3][frame as usize], bob, 1.0, true),
        3 => (
            [0, 1, 2, 1, 0, -1, 0, 1][frame as usize],
            bob - 2,
            1.02,
            false,
        ),
        4 => (
            0,
            [0, -10, -22, -30, -22, -10, 0, 0][frame as usize],
            1.0,
            false,
        ),
        5 => (0, 5 + bob.abs(), 0.96, false),
        6 => ([0, -1, 0, 1, 0, -1, 0, 1][frame as usize], bob, 1.0, false),
        7 => ([-3, 0, 3, 0, -3, 0, 3, 0][frame as usize], bob, 1.0, false),
        _ => (
            0,
            [0, -6, -14, -6, 0, -4, 0, 0][frame as usize],
            1.04,
            frame >= 4,
        ),
    }
}

fn build_atlas(source: &[u8]) -> Result<Vec<u8>> {
    if source.is_empty() || source.len() > super::atlas::MAX_SPRITE_BYTES {
        anyhow::bail!("pet_creator_source_size_invalid");
    }
    let format = image::guess_format(source).context("pet_creator_source_format_invalid")?;
    let dimensions = image::ImageReader::with_format(Cursor::new(source), format)
        .into_dimensions()
        .context("pet_creator_source_header_invalid")?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > MAX_GENERATED_SOURCE_DIMENSION
        || dimensions.1 > MAX_GENERATED_SOURCE_DIMENSION
    {
        anyhow::bail!("pet_creator_source_dimensions_invalid");
    }
    let mut source = image::load_from_memory_with_format(source, format)?.to_rgba8();
    remove_edge_background(&mut source);
    let bounds =
        content_bounds(&source).ok_or_else(|| anyhow::anyhow!("pet_creator_empty_image"))?;
    let cropped = image::imageops::crop_imm(
        &source,
        bounds.0,
        bounds.1,
        bounds.2 - bounds.0,
        bounds.3 - bounds.1,
    )
    .to_image();
    let fit = (146_f32 / cropped.width().max(1) as f32)
        .min(166_f32 / cropped.height().max(1) as f32)
        .min(1.0);
    let base_width = ((cropped.width() as f32 * fit).round() as u32).max(1);
    let base_height = ((cropped.height() as f32 * fit).round() as u32).max(1);
    let base = image::imageops::resize(
        &cropped,
        base_width,
        base_height,
        image::imageops::FilterType::Lanczos3,
    );
    let mut atlas = RgbaImage::new(ATLAS_WIDTH, V1_HEIGHT);
    for row in 0..9 {
        for frame in 0..8 {
            let (dx, dy, scale, flip) = pose(row, frame);
            let width = ((base.width() as f32 * scale).round() as u32).max(1);
            let height = ((base.height() as f32 * scale).round() as u32).max(1);
            let resized = image::imageops::resize(
                &base,
                width,
                height,
                image::imageops::FilterType::Lanczos3,
            );
            let frame_image = if flip {
                image::imageops::flip_horizontal(&resized)
            } else {
                resized
            };
            let x = i64::from(frame * CELL_WIDTH)
                + i64::from(CELL_WIDTH.saturating_sub(frame_image.width()) / 2)
                + i64::from(dx);
            let y = i64::from(row * CELL_HEIGHT)
                + i64::from(
                    CELL_HEIGHT
                        .saturating_sub(frame_image.height())
                        .saturating_sub(10),
                )
                + i64::from(dy);
            image::imageops::overlay(&mut atlas, &frame_image, x, y);
        }
    }
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(atlas).write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}

pub async fn create_preview(request: PetCreateRequest) -> Result<PetImportPreview> {
    let display_name = sanitize_text(&request.display_name, 256);
    let description = request
        .description
        .as_deref()
        .map(|value| sanitize_text(value, 2048))
        .filter(|value| !value.is_empty());
    let prompt = sanitize_text(&request.prompt, MAX_CREATE_PROMPT_SCALARS);
    if display_name.is_empty() || prompt.is_empty() {
        anyhow::bail!("pet_creator_fields_required");
    }
    let generation_prompt = format!(
        "Create one original full-body desktop mascot character on a transparent background. Center it, keep the entire character visible with generous padding, use a clear readable silhouette at tiny size, no text, no frame, no props cut off, no sprite grid. Neutral friendly three-quarter pose. Character direction: {prompt}"
    );
    let media_config = crate::config::cached_config().media_gen.clone();
    let outcome = crate::media_gen::execute_image(
        &media_config,
        crate::media_gen::ImageRequest {
            prompt: &generation_prompt,
            size: None,
            n: 1,
            aspect_ratio: Some("1:1"),
            resolution: None,
            input_images: &[],
            mask: None,
            explicit_model: None,
        },
        crate::media_gen::UsageMeta {
            operation: "pet.create",
            source: "pet.creator",
            session_id: None,
            agent_id: None,
        },
    )
    .await?;
    let generated = outcome
        .result
        .images
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("pet_creator_no_image"))?;
    let atlas = crate::blocking::run_blocking(move || build_atlas(&generated.data)).await?;
    let manifest = PetManifest {
        id: portable_pet_id(&display_name),
        display_name,
        description,
        sprite_version_number: PetSpriteVersion::V1,
        spritesheet_path: "spritesheet.png".to_string(),
    };
    let package = crate::blocking::run_blocking(move || validate_package(manifest, atlas)).await?;
    crate::blocking::run_blocking(move || super::import::register_created_preview(package)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn generated_atlas_has_codex_v1_geometry() {
        let source = RgbaImage::from_fn(64, 64, |x, y| {
            if (12..52).contains(&x) && (8..58).contains(&y) {
                Rgba([40, 120, 240, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        let atlas = build_atlas(&bytes.into_inner()).unwrap();
        let dimensions = image::load_from_memory(&atlas).unwrap().dimensions();
        assert_eq!(dimensions, (ATLAS_WIDTH, V1_HEIGHT));
    }
}
