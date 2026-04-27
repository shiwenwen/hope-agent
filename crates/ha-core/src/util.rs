/// Serde default helper: returns `true`.
pub fn default_true() -> bool {
    true
}

/// Number of seconds in an hour. Prefer this over `3600` / `60 * 60` literals.
pub const SECS_PER_HOUR: u64 = 3_600;
/// Number of seconds in a day. Prefer this over `86_400` / `24 * 3600` literals.
pub const SECS_PER_DAY: u64 = 86_400;

/// Return the unix-epoch timestamp (seconds) for `window_days` ago. Used by
/// Dashboard queries and memory counts that want "rows within the last N
/// days" — keeps the `now - days * 86_400` arithmetic in one place.
/// Returns 0 when the system clock is before the epoch.
pub fn epoch_cutoff_secs(window_days: u32) -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now - (window_days as i64) * (SECS_PER_DAY as i64)
}

/// Trim `opt` and return it if non-empty; otherwise return `fallback`. Used when
/// an optional override ("display text", "override title", ...) should win over
/// a mandatory default only when the caller actually supplied meaningful text.
pub fn non_empty_trim_or<'a>(opt: Option<&'a str>, fallback: &'a str) -> &'a str {
    opt.map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
}

/// Produce a comma-separated list of `?` placeholders for a SQL `IN` clause.
/// Example: `sql_in_placeholders(3)` → `"?,?,?"`.
pub fn sql_in_placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Truncate a string to at most `max_bytes` bytes on a valid UTF-8 char boundary.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // floor_char_boundary is nightly-only, so do it manually
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Return the suffix of `s` that is at most `max_bytes` bytes, aligned to a valid
/// UTF-8 char boundary (complement of `truncate_utf8`).
pub fn truncate_utf8_tail(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// Truncate a `String` in place to at most `max_bytes` bytes on a valid UTF-8
/// char boundary. Returns whether truncation happened.
pub fn truncate_string_utf8(s: &mut String, max_bytes: usize) -> bool {
    if s.len() <= max_bytes {
        return false;
    }
    let end = truncate_utf8(s.as_str(), max_bytes).len();
    s.truncate(end);
    true
}

/// Mask a secret for display by keeping a small char-count prefix and suffix.
/// Empty values remain empty so callers can distinguish "not configured" from
/// "configured but hidden".
pub fn mask_secret_middle(value: &str, prefix_chars: usize, suffix_chars: usize) -> String {
    if value.is_empty() {
        return String::new();
    }

    let visible_chars = prefix_chars.saturating_add(suffix_chars);
    if value.chars().count() <= visible_chars {
        return "****".to_string();
    }

    let prefix: String = value.chars().take(prefix_chars).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(suffix_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

/// Recursively merge `src` JSON into `dst`. Object keys are merged deeply;
/// non-object values in `src` overwrite `dst`.
pub fn merge_json(dst: &mut serde_json::Value, src: serde_json::Value) {
    match (dst, src) {
        (serde_json::Value::Object(dst_map), serde_json::Value::Object(src_map)) => {
            for (k, v) in src_map {
                match dst_map.get_mut(&k) {
                    Some(existing) => merge_json(existing, v),
                    None => {
                        dst_map.insert(k, v);
                    }
                }
            }
        }
        (dst_slot, src_val) => {
            *dst_slot = src_val;
        }
    }
}

/// Read a non-negative i64 column as u64 (rusqlite 0.39+ removed u64 FromSql).
pub fn sql_u64(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<u64> {
    row.get::<_, i64>(idx).map(|v| v as u64)
}

/// Read an optional non-negative i64 column as Option<u64>.
pub fn sql_opt_u64(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(idx).map(|v| v.map(|n| n as u64))
}

/// Locate the first balanced JSON span starting at the earliest `[` or
/// `{`. Tolerates leading prose and ```json code fences — anything
/// before the first bracket is skipped. Tracks string state so brackets
/// inside strings don't confuse the depth counter. Returns the slice
/// including both enclosing brackets.
///
/// When `preferred_open` is `Some('[')` or `Some('{')`, only the matching
/// bracket style is considered; otherwise whichever appears first wins.
/// Used by the LLM response parsers to pluck a JSON envelope out of
/// mixed text — see `memory_extract::parse_extraction_response` and
/// `memory::dreaming::scoring::parse_nominations`.
pub fn extract_json_span(text: &str, preferred_open: Option<char>) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| match preferred_open {
        Some('[') => b == b'[',
        Some('{') => b == b'{',
        _ => b == b'[' || b == b'{',
    })?;
    let open = bytes[start];
    let close = if open == b'[' { b']' } else { b'}' };

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Canonicalize and validate a user-supplied working directory.
///
/// Returns:
/// - `Ok(None)` for `None`, an empty string, or a whitespace-only string
///   (caller treats this as "clear the selection")
/// - `Ok(Some(absolute_path))` after `canonicalize` succeeds and the result
///   is confirmed to be an existing directory
/// - `Err(_)` when the path cannot be resolved or does not point to a directory
///
/// Shared by session-level (`SessionDB::update_session_working_dir`) and
/// project-level (`ProjectDB::create` / `ProjectDB::update`) entry points so
/// the error wording and resolution semantics stay aligned.
pub fn canonicalize_working_dir(input: Option<&str>) -> anyhow::Result<Option<String>> {
    let trimmed = match input.map(str::trim) {
        Some(p) if !p.is_empty() => p,
        _ => return Ok(None),
    };
    let path = std::path::Path::new(trimmed);
    let canon = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("Cannot resolve working directory '{}': {}", trimmed, e))?;
    if !canon.is_dir() {
        anyhow::bail!("Working directory '{}' is not a directory", canon.display());
    }
    Ok(Some(canon.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_in_placeholders_zero() {
        assert_eq!(sql_in_placeholders(0), "");
    }

    #[test]
    fn sql_in_placeholders_one() {
        assert_eq!(sql_in_placeholders(1), "?");
    }

    #[test]
    fn sql_in_placeholders_many() {
        assert_eq!(sql_in_placeholders(3), "?,?,?");
        assert_eq!(sql_in_placeholders(5), "?,?,?,?,?");
    }

    #[test]
    fn truncate_string_utf8_keeps_char_boundaries() {
        let mut s = "你好abc".to_string();
        assert!(truncate_string_utf8(&mut s, 4));
        assert_eq!(s, "你");

        let mut emoji = "🔑abc".to_string();
        assert!(truncate_string_utf8(&mut emoji, 1));
        assert_eq!(emoji, "");
    }

    #[test]
    fn mask_secret_middle_uses_chars_not_bytes() {
        assert_eq!(mask_secret_middle("", 2, 2), "");
        assert_eq!(mask_secret_middle("abcd", 2, 2), "****");
        assert_eq!(mask_secret_middle("abcdef", 2, 2), "ab...ef");
        assert_eq!(mask_secret_middle("密钥🔑abcdef", 2, 2), "密钥...ef");
    }
}
