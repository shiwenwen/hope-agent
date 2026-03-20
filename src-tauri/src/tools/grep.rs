use anyhow::Result;
use serde_json::Value;

use super::{expand_tilde, extract_string_param};

/// Max matches for grep (default).
pub(crate) const GREP_DEFAULT_LIMIT: usize = 100;
/// Max chars per grep output line.
const GREP_MAX_LINE_LENGTH: usize = 500;
/// Max output bytes for grep/find (50KB).
pub(crate) const GREP_FIND_MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub(crate) async fn tool_grep(args: &Value) -> Result<String> {
    let pattern_str = args
        .get("pattern")
        .and_then(|v| extract_string_param(v))
        .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' parameter"))?;

    let raw_path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| extract_string_param(v))
        .unwrap_or(".");
    let search_path = expand_tilde(raw_path);

    let glob_pattern = args.get("glob").and_then(|v| extract_string_param(v));
    let ignore_case = args
        .get("ignore_case")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || args
            .get("ignoreCase")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let literal = args
        .get("literal")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let context_lines = args
        .get("context")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(GREP_DEFAULT_LIMIT as u64) as usize;

    log::info!(
        "Grep: pattern='{}', path='{}', glob={:?}, limit={}",
        pattern_str,
        search_path,
        glob_pattern,
        limit
    );

    // Build regex
    let regex_pattern = if literal {
        regex::escape(pattern_str)
    } else {
        pattern_str.to_string()
    };
    let re = regex::RegexBuilder::new(&regex_pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| anyhow::anyhow!("Invalid regex pattern '{}': {}", pattern_str, e))?;

    // Build glob matcher if provided
    let glob_matcher = if let Some(g) = glob_pattern {
        Some(
            glob::Pattern::new(g)
                .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", g, e))?,
        )
    } else {
        None
    };

    // Walk directory respecting .gitignore
    let search_path_clone = search_path.clone();
    let walker = ignore::WalkBuilder::new(&search_path)
        .hidden(false) // include hidden files
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let mut output = String::new();
    let mut match_count: usize = 0;
    let mut byte_limited = false;
    let mut lines_truncated = false;

    let search_base = std::path::Path::new(&search_path_clone);

    for entry_result in walker {
        if match_count >= limit || byte_limited {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip directories
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if ft.is_dir() {
            continue;
        }

        let entry_path = entry.path();

        // Apply glob filter
        if let Some(ref gm) = glob_matcher {
            let rel = entry_path
                .strip_prefix(search_base)
                .unwrap_or(entry_path);
            let rel_str = rel.to_string_lossy();
            let file_name = entry_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            // Match against filename or relative path
            if !gm.matches(&file_name) && !gm.matches(&rel_str) {
                continue;
            }
        }

        // Read file as text (skip binary)
        let content = match std::fs::read_to_string(entry_path) {
            Ok(c) => c,
            Err(_) => continue, // skip binary/unreadable files
        };

        let rel_path = entry_path
            .strip_prefix(search_base)
            .unwrap_or(entry_path)
            .to_string_lossy();

        let file_lines: Vec<&str> = content.lines().collect();

        for (line_idx, line) in file_lines.iter().enumerate() {
            if match_count >= limit {
                break;
            }
            if !re.is_match(line) {
                continue;
            }

            match_count += 1;

            // Add context lines before
            if context_lines > 0 {
                let ctx_start = line_idx.saturating_sub(context_lines);
                for ci in ctx_start..line_idx {
                    let ctx_line =
                        truncate_line(file_lines[ci], GREP_MAX_LINE_LENGTH, &mut lines_truncated);
                    let formatted = format!("{}-{}- {}\n", rel_path, ci + 1, ctx_line);
                    if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                        byte_limited = true;
                        break;
                    }
                    output.push_str(&formatted);
                }
            }

            if byte_limited {
                break;
            }

            // Match line
            let match_line = truncate_line(line, GREP_MAX_LINE_LENGTH, &mut lines_truncated);
            let formatted = format!("{}:{}: {}\n", rel_path, line_idx + 1, match_line);
            if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                byte_limited = true;
                break;
            }
            output.push_str(&formatted);

            // Add context lines after
            if context_lines > 0 {
                let ctx_end = (line_idx + 1 + context_lines).min(file_lines.len());
                for ci in (line_idx + 1)..ctx_end {
                    let ctx_line =
                        truncate_line(file_lines[ci], GREP_MAX_LINE_LENGTH, &mut lines_truncated);
                    let formatted = format!("{}-{}- {}\n", rel_path, ci + 1, ctx_line);
                    if output.len() + formatted.len() > GREP_FIND_MAX_OUTPUT_BYTES {
                        byte_limited = true;
                        break;
                    }
                    output.push_str(&formatted);
                }
                if !byte_limited {
                    output.push('\n'); // separator between match groups
                }
            }
        }
    }

    if match_count == 0 {
        return Ok("No matches found.".to_string());
    }

    // Append notices
    let mut notices = Vec::new();
    if match_count >= limit {
        notices.push(format!(
            "{} matches limit reached. Use limit={} for more, or refine pattern.",
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
    if lines_truncated {
        notices.push(
            "Some lines truncated to 500 chars. Use read tool to see full lines.".to_string(),
        );
    }
    if !notices.is_empty() {
        output.push_str(&format!("[{}]\n", notices.join(" ")));
    }

    Ok(output.trim_end().to_string())
}

/// Truncate a line to max_len chars, setting flag if truncated.
fn truncate_line(line: &str, max_len: usize, truncated_flag: &mut bool) -> String {
    if line.len() <= max_len {
        line.to_string()
    } else {
        *truncated_flag = true;
        // Truncate at char boundary
        let end = line
            .char_indices()
            .nth(max_len)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        format!("{}... [truncated]", &line[..end])
    }
}
