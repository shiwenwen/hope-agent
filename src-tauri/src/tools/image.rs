use anyhow::Result;
use serde_json::Value;

use super::expand_tilde;
use super::read::{detect_image_mime, resize_image_if_needed};

/// Tool: image — analyze an image file (returns base64-encoded image for LLM vision).
pub(crate) async fn tool_image(args: &Value) -> Result<String> {
    let path_raw = args.get("path")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    let prompt = args.get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let path = expand_tilde(path_raw);
    let file_path = std::path::Path::new(&path);

    if !file_path.exists() {
        return Ok(format!("Error: File not found: {}", path));
    }

    let data = std::fs::read(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", path, e))?;

    // Check first bytes for image magic
    let mime = detect_image_mime(&data)
        .ok_or_else(|| anyhow::anyhow!("Not an image file: {}", path))?;

    let (b64, final_mime) = resize_image_if_needed(&data, mime)?;

    let mut output = String::new();
    if !prompt.is_empty() {
        output.push_str(&format!("Image analysis prompt: {}\n\n", prompt));
    }
    output.push_str(&format!(
        "Read image file [{}] ({} bytes, {})\nbase64:{}",
        final_mime,
        data.len(),
        path,
        b64,
    ));

    Ok(output)
}
