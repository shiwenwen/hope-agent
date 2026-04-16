/// Serde default helper: returns `true`.
pub fn default_true() -> bool {
    true
}

/// Number of seconds in an hour. Prefer this over `3600` / `60 * 60` literals.
pub const SECS_PER_HOUR: u64 = 3_600;
/// Number of seconds in a day. Prefer this over `86_400` / `24 * 3600` literals.
pub const SECS_PER_DAY: u64 = 86_400;

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
