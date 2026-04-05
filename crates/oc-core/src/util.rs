/// Serde default helper: returns `true`.
pub fn default_true() -> bool {
    true
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

/// Read a non-negative i64 column as u64 (rusqlite 0.39+ removed u64 FromSql).
pub fn sql_u64(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<u64> {
    row.get::<_, i64>(idx).map(|v| v as u64)
}

/// Read an optional non-negative i64 column as Option<u64>.
pub fn sql_opt_u64(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(idx).map(|v| v.map(|n| n as u64))
}
