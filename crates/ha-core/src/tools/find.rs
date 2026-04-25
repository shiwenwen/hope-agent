use anyhow::Result;
use serde_json::Value;

use super::extract_string_param;
use super::grep::GREP_FIND_MAX_OUTPUT_BYTES;

/// Default max results for find.
const FIND_DEFAULT_LIMIT: usize = 1000;

pub(crate) async fn tool_find(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(ctx.default_path());
    let search_path = ctx.resolve_path(raw_path);

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(FIND_DEFAULT_LIMIT as u64) as usize;

    app_info!(
        "tool",
        "find",
        "Find: pattern='{}', path='{}', limit={}",
        pattern_str,
        search_path,
        limit
    );

    // Build glob matcher
    let glob_matcher = glob::Pattern::new(pattern_str)
        .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", pattern_str, e))?;

    // Validate path
    let meta = tokio::fs::metadata(&search_path)
        .await
        .map_err(|_| anyhow::anyhow!("Path not found: {}", search_path))?;
    if !meta.is_dir() {
        return Err(anyhow::anyhow!("Not a directory: {}", search_path));
    }

    // Walk directory respecting .gitignore
    let walker = ignore::WalkBuilder::new(&search_path)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let search_base = std::path::Path::new(&search_path);
    let mut output = String::new();
    let mut count: usize = 0;
    let mut byte_limited = false;

    for entry_result in walker {
        if count >= limit || byte_limited {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip directories themselves (but walk into them)
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if ft.is_dir() {
            continue;
        }

        let entry_path = entry.path();
        let rel_path = entry_path.strip_prefix(search_base).unwrap_or(entry_path);
        let rel_str = rel_path.to_string_lossy();
        let file_name = entry_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        // Match against filename or full relative path
        if !glob_matcher.matches(&file_name) && !glob_matcher.matches(&rel_str) {
            continue;
        }

        count += 1;
        let line = format!("{}\n", rel_str);
        if output.len() + line.len() > GREP_FIND_MAX_OUTPUT_BYTES {
            byte_limited = true;
            break;
        }
        output.push_str(&line);
    }

    if count == 0 {
        return Ok("No files found.".to_string());
    }

    // Append notices
    let mut notices = Vec::new();
    if count >= limit {
        notices.push(format!(
            "{} results limit reached. Use limit={} for more, or refine pattern.",
            limit,
            limit * 2
        ));
    }
    if byte_limited {
        notices.push(format!(
            "{}KB output limit reached.",
            GREP_FIND_MAX_OUTPUT_BYTES / 1024
        ));
    }
    if !notices.is_empty() {
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}
