use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Log File Operations ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFileInfo {
    pub name: String,
    pub size_bytes: u64,
    pub modified: String,
}

/// List all .log files under ~/.hope-agent/logs/, newest first.
pub fn list_log_files() -> Result<Vec<LogFileInfo>> {
    let logs_dir = crate::paths::logs_dir()?;
    if !logs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&logs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("log") {
            let meta = std::fs::metadata(&path)?;
            let modified = meta
                .modified()
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                })
                .unwrap_or_default();
            files.push(LogFileInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                size_bytes: meta.len(),
                modified,
            });
        }
    }
    // Sort newest first by name (date-based names sort naturally)
    files.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(files)
}

/// Read a log file with optional tail (last N lines). Returns the content as a string.
/// If `tail_lines` is Some(n), returns only the last n lines; otherwise returns full content.
pub fn read_log_file(filename: &str, tail_lines: Option<u32>) -> Result<String> {
    // Sanitize filename to prevent path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(anyhow::anyhow!("Invalid log filename"));
    }
    let logs_dir = crate::paths::logs_dir()?;
    let path = logs_dir.join(filename);
    if !path.exists() {
        return Err(anyhow::anyhow!("Log file not found: {}", filename));
    }

    let content = std::fs::read_to_string(&path)?;

    if let Some(n) = tail_lines {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(n as usize);
        Ok(lines[start..].join("\n"))
    } else {
        Ok(content)
    }
}

/// Clean up old log files beyond max_age_days.
pub fn cleanup_old_log_files(max_age_days: u32) -> Result<u64> {
    let logs_dir = crate::paths::logs_dir()?;
    if !logs_dir.exists() {
        return Ok(0);
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
    let cutoff_date = cutoff.format("%Y-%m-%d").to_string();
    let mut removed = 0u64;
    for entry in std::fs::read_dir(&logs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        // Parse date from filename: hope-agent-YYYY-MM-DD.log or hope-agent-YYYY-MM-DD.N.log
        if let Some(date_part) = name.strip_prefix("hope-agent-") {
            let date = &date_part[..10.min(date_part.len())];
            if date < cutoff_date.as_str() {
                let _ = std::fs::remove_file(entry.path());
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Get the path to today's log file (for display in UI).
pub fn current_log_file_path() -> Result<String> {
    let logs_dir = crate::paths::logs_dir()?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let path = logs_dir.join(format!("hope-agent-{}.log", today));
    Ok(path.to_string_lossy().to_string())
}

// ── Sensitive Data Redaction ─────────────────────────────────────

/// Redact potentially sensitive values from a JSON string for logging.
pub fn redact_sensitive(input: &str) -> String {
    let sensitive_keys = [
        "api_key",
        "apiKey",
        "api-key",
        "access_token",
        "accessToken",
        "refresh_token",
        "refreshToken",
        "authorization",
        "Authorization",
        "x-api-key",
        "bearer",
        "password",
        "secret",
        // ha-server `?token=<api-key>` WebSocket auth fallback.
        "token",
    ];

    let mut result = input.to_string();
    for key in &sensitive_keys {
        // Pattern 1: "key":"value" or "key": "value" (JSON string values)
        let patterns = [format!("\"{}\":\"", key), format!("\"{}\": \"", key)];
        for pattern in &patterns {
            let mut search_from = 0;
            while search_from < result.len() {
                if let Some(pos) = result[search_from..].find(pattern.as_str()) {
                    let start = search_from + pos;
                    let value_start = start + pattern.len();
                    if let Some(end) = result[value_start..].find('"') {
                        let before = &result[..value_start];
                        let after = &result[value_start + end..];
                        result = format!("{}[REDACTED]{}", before, after);
                        search_from = value_start + "[REDACTED]".len();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        // Pattern 2: URL query parameters (?key=value& or &key=value&)
        for sep in &["?", "&"] {
            let url_pattern = format!("{}{}=", sep, key);
            let mut search_from = 0;
            while search_from < result.len() {
                if let Some(pos) = result[search_from..].find(url_pattern.as_str()) {
                    let start = search_from + pos;
                    let value_start = start + url_pattern.len();
                    let end = result[value_start..]
                        .find(['&', ' ', '"', '\n'])
                        .unwrap_or(result.len() - value_start);
                    let before = &result[..value_start];
                    let after = &result[value_start + end..];
                    result = format!("{}[REDACTED]{}", before, after);
                    search_from = value_start + "[REDACTED]".len();
                } else {
                    break;
                }
            }
        }
    }
    result
}
