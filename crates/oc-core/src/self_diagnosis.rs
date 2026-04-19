use crate::crash_journal::{CrashJournal, DiagnosisResult};
use crate::paths;
use crate::provider::{ApiType, ProviderConfig};
use std::time::Duration;

const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_LOG_LINES: usize = 200;

// ── Public API ─────────────────────────────────────────────────────

/// Run self-diagnosis using available LLM providers.
/// Reads crash logs, builds a diagnostic prompt, and calls the cheapest available LLM.
/// Falls back to basic log analysis if all LLM calls fail.
pub fn diagnose(journal: &CrashJournal) -> Result<DiagnosisResult, String> {
    let log_excerpt = read_recent_logs();
    let crash_summary = build_crash_summary(journal);
    let prompt = build_diagnosis_prompt(&crash_summary, &log_excerpt);

    // Try LLM diagnosis with provider failover
    let providers = load_candidate_providers();
    if !providers.is_empty() {
        for provider in &providers {
            match call_llm(provider, &prompt) {
                Ok(result) => return Ok(result),
                Err(e) => {
                    eprintln!(
                        "[Diagnosis] Provider '{}' failed: {}, trying next...",
                        provider.name, e
                    );
                }
            }
        }
        eprintln!("[Diagnosis] All LLM providers failed, falling back to basic analysis.");
    } else {
        eprintln!("[Diagnosis] No LLM providers available, using basic analysis.");
    }

    // Fallback: basic analysis without LLM
    Ok(basic_analysis(journal))
}

/// Apply safe auto-fixes based on diagnosis result.
/// Returns a list of applied fixes.
pub fn auto_fix(result: &DiagnosisResult) -> Vec<String> {
    let mut fixes = Vec::new();

    // Check if config.json is corrupted
    if let Ok(config_path) = paths::config_path() {
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if serde_json::from_str::<serde_json::Value>(&content).is_err() {
                    // Config is corrupted - try to restore from backup
                    if try_restore_config_from_backup() {
                        fixes.push("Restored config.json from backup (was corrupted)".to_string());
                    } else {
                        // No backup available - reset to defaults
                        let default_config = serde_json::json!({
                            "providers": [],
                            "activeModel": null,
                            "fallbackModels": []
                        });
                        if let Ok(json_str) = serde_json::to_string_pretty(&default_config) {
                            if std::fs::write(&config_path, json_str).is_ok() {
                                fixes.push("Reset config.json to defaults (was corrupted, no backup available)".to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Check if logs.db is corrupted (basic check: can we open it?)
    if result.cause.contains("database")
        || result.cause.contains("sqlite")
        || result.cause.contains("SQLite")
    {
        if let Ok(logs_path) = paths::logs_db_path() {
            if logs_path.exists() {
                match rusqlite::Connection::open(&logs_path) {
                    Ok(conn) => {
                        // Try integrity check
                        let integrity_ok = conn
                            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                            .map(|result| result == "ok")
                            .unwrap_or(false);
                        if !integrity_ok {
                            drop(conn);
                            if std::fs::remove_file(&logs_path).is_ok() {
                                fixes.push(
                                    "Removed corrupted logs.db (will be recreated on restart)"
                                        .to_string(),
                                );
                            }
                        }
                    }
                    Err(_) => {
                        if std::fs::remove_file(&logs_path).is_ok() {
                            fixes.push(
                                "Removed corrupted logs.db (will be recreated on restart)"
                                    .to_string(),
                            );
                        }
                    }
                }
            }
        }
    }

    // Check compact config for problematic values
    if result.cause.contains("context")
        || result.cause.contains("compact")
        || result.cause.contains("overflow")
    {
        if let Ok(config_path) = paths::config_path() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&content) {
                    if config.get("compact").is_some() {
                        config["compact"] = serde_json::json!({});
                        if let Ok(json_str) = serde_json::to_string_pretty(&config) {
                            if std::fs::write(&config_path, json_str).is_ok() {
                                fixes.push("Reset compact config to defaults".to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    fixes
}

// ── Provider Loading ───────────────────────────────────────────────

/// Load candidate providers for diagnosis, sorted by cost (cheapest first).
/// Skips Codex (OAuth) providers since Guardian can't handle token refresh.
fn load_candidate_providers() -> Vec<ProviderConfig> {
    // Read config.json directly (don't use config::load_config which may depend on complex types)
    let config_path = match paths::config_path() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Parse just the providers array
    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let providers_val = match config.get("providers") {
        Some(v) => v,
        None => return Vec::new(),
    };

    let providers: Vec<ProviderConfig> = match serde_json::from_value(providers_val.clone()) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut candidates: Vec<ProviderConfig> = providers
        .into_iter()
        .filter(|p| {
            p.enabled
                && !p.api_key.is_empty()
                && p.api_type != ApiType::Codex
                && !p.models.is_empty()
        })
        .collect();

    // Sort by cheapest model cost (prefer low-cost for diagnosis)
    candidates.sort_by(|a, b| {
        let cost_a = a
            .models
            .iter()
            .map(|m| m.cost_input + m.cost_output)
            .fold(f64::MAX, f64::min);
        let cost_b = b
            .models
            .iter()
            .map(|m| m.cost_input + m.cost_output)
            .fold(f64::MAX, f64::min);
        cost_a
            .partial_cmp(&cost_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates
}

// ── LLM Calling ────────────────────────────────────────────────────

fn call_llm(provider: &ProviderConfig, prompt: &str) -> Result<DiagnosisResult, String> {
    // Pick the cheapest model from this provider
    let model = provider
        .models
        .iter()
        .min_by(|a, b| {
            let ca = a.cost_input + a.cost_output;
            let cb = b.cost_input + b.cost_output;
            ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| "No models available".to_string())?;

    let client = crate::provider::apply_proxy_blocking(
        reqwest::blocking::Client::builder().timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS)),
    )
    .build()
    .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response_text = match provider.api_type {
        ApiType::Anthropic => call_anthropic(&client, provider, &model.id, prompt)?,
        ApiType::OpenaiChat | ApiType::OpenaiResponses => {
            call_openai(&client, provider, &model.id, prompt)?
        }
        ApiType::Codex => return Err("Codex OAuth not supported for diagnosis".to_string()),
    };

    // Try to parse structured JSON response
    parse_diagnosis_response(&response_text, &provider.name)
}

fn call_anthropic(
    client: &reqwest::blocking::Client,
    provider: &ProviderConfig,
    model_id: &str,
    prompt: &str,
) -> Result<String, String> {
    let url = format!("{}/v1/messages", provider.base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model_id,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ]
    });

    let resp = client
        .post(&url)
        .header("x-api-key", &provider.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("Anthropic request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Anthropic API error: {} {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }

    let resp_json: serde_json::Value = resp.json().map_err(|e| format!("Parse error: {}", e))?;
    resp_json["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No text in Anthropic response".to_string())
}

fn call_openai(
    client: &reqwest::blocking::Client,
    provider: &ProviderConfig,
    model_id: &str,
    prompt: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/v1/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "model": model_id,
        "max_tokens": 1024,
        "messages": [
            {"role": "user", "content": prompt}
        ]
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("OpenAI request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "OpenAI API error: {} {}",
            resp.status(),
            resp.text().unwrap_or_default()
        ));
    }

    let resp_json: serde_json::Value = resp.json().map_err(|e| format!("Parse error: {}", e))?;
    resp_json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No content in OpenAI response".to_string())
}

fn parse_diagnosis_response(text: &str, provider_name: &str) -> Result<DiagnosisResult, String> {
    // Try to find JSON in the response (it might be wrapped in markdown code blocks)
    let json_str = if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            &text[start..=end]
        } else {
            text
        }
    } else {
        text
    };

    match serde_json::from_str::<DiagnosisResult>(json_str) {
        Ok(mut result) => {
            result.provider_used = Some(provider_name.to_string());
            Ok(result)
        }
        Err(_) => {
            // LLM didn't return structured JSON, wrap the raw text
            Ok(DiagnosisResult {
                cause: text.chars().take(500).collect(),
                severity: "unknown".to_string(),
                user_actionable: false,
                recommendations: vec!["Review the full diagnosis output for details.".to_string()],
                auto_fix_applied: Vec::new(),
                provider_used: Some(provider_name.to_string()),
            })
        }
    }
}

// ── Log Reading ────────────────────────────────────────────────────

/// Read the most recent log lines from plaintext log files
fn read_recent_logs() -> String {
    let logs_dir = match paths::logs_dir() {
        Ok(d) => d,
        Err(_) => return String::new(),
    };

    if !logs_dir.exists() {
        return String::new();
    }

    // Find the most recently modified log file
    let mut log_files: Vec<_> = std::fs::read_dir(&logs_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let e = e.ok()?;
            let path = e.path();
            if path.extension().map(|ext| ext == "log").unwrap_or(false) {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((path, modified))
            } else {
                None
            }
        })
        .collect();

    log_files.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));

    if let Some((latest_log, _)) = log_files.first() {
        match std::fs::read_to_string(latest_log) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = lines.len().saturating_sub(MAX_LOG_LINES);
                lines[start..].join("\n")
            }
            Err(_) => String::new(),
        }
    } else {
        String::new()
    }
}

// ── Prompt Building ────────────────────────────────────────────────

fn build_crash_summary(journal: &CrashJournal) -> String {
    let mut summary = format!("Total crashes recorded: {}\n", journal.total_crashes);

    // Show recent crashes (last 10)
    let recent = &journal.crashes[journal.crashes.len().saturating_sub(10)..];
    for entry in recent {
        summary.push_str(&format!(
            "- {} | exit_code={} | signal={} | session_crash_count={}\n",
            entry.timestamp,
            entry.exit_code,
            entry.signal.as_deref().unwrap_or("none"),
            entry.crash_count_session,
        ));
    }

    summary
}

fn build_diagnosis_prompt(crash_summary: &str, log_excerpt: &str) -> String {
    format!(
        r#"You are diagnosing why the OpenComputer desktop app (Tauri 2 + Rust + React) keeps crashing.

## Recent Crash History
{crash_summary}

## Recent Log Output (last lines before crash)
```
{log_excerpt}
```

## Task
Analyze the crash patterns and logs. Identify:
1. The most likely root cause of the crashes
2. Whether this is a configuration issue, code bug, or system-level problem
3. Whether the user can fix this themselves

## Response Format
Respond ONLY with a JSON object (no markdown, no explanation outside JSON):
{{
  "cause": "Brief description of the root cause",
  "severity": "low|medium|high|critical",
  "user_actionable": true/false,
  "recommendations": ["Action item 1", "Action item 2"]
}}"#
    )
}

// ── Basic Analysis (fallback when LLM unavailable) ─────────────────

fn basic_analysis(journal: &CrashJournal) -> DiagnosisResult {
    let recent = &journal.crashes[journal.crashes.len().saturating_sub(5)..];

    // Analyze crash patterns
    let has_segfault = recent
        .iter()
        .any(|c| c.exit_code == 139 || c.signal.as_deref() == Some("SIGSEGV"));
    let has_oom = recent
        .iter()
        .any(|c| c.exit_code == 137 || c.signal.as_deref() == Some("SIGKILL"));
    let has_abort = recent
        .iter()
        .any(|c| c.exit_code == 134 || c.signal.as_deref() == Some("SIGABRT"));
    let all_same_code =
        recent.len() > 1 && recent.iter().all(|c| c.exit_code == recent[0].exit_code);

    let (cause, severity, recommendations) = if has_segfault {
        (
            "Segmentation fault (SIGSEGV) - memory access violation".to_string(),
            "critical".to_string(),
            vec![
                "This is likely a code bug. Check for null pointer dereferences or use-after-free."
                    .to_string(),
                "Try updating the app to the latest version.".to_string(),
                "If persists, report this issue with your crash logs.".to_string(),
            ],
        )
    } else if has_oom {
        (
            "Process killed (SIGKILL) - likely out of memory".to_string(),
            "high".to_string(),
            vec![
                "The app may be using too much memory. Close other applications.".to_string(),
                "Check if context compaction settings are too aggressive.".to_string(),
                "Consider reducing the number of active sessions.".to_string(),
            ],
        )
    } else if has_abort {
        (
            "Process aborted (SIGABRT) - internal assertion failure".to_string(),
            "high".to_string(),
            vec![
                "An internal assertion failed. This may indicate corrupted state.".to_string(),
                "Try resetting the app configuration.".to_string(),
            ],
        )
    } else if all_same_code {
        (
            format!("Repeated crashes with exit code {}", recent[0].exit_code),
            "high".to_string(),
            vec![
                "The app is consistently crashing with the same error.".to_string(),
                "Check if a specific configuration or plugin is causing the issue.".to_string(),
                "Try resetting settings or disabling recently added skills.".to_string(),
            ],
        )
    } else {
        (
            format!(
                "Multiple crashes with varying exit codes (total: {})",
                journal.total_crashes
            ),
            "medium".to_string(),
            vec![
                "The app has been crashing intermittently.".to_string(),
                "Check system logs for more details.".to_string(),
                "Ensure your system meets the minimum requirements.".to_string(),
            ],
        )
    };

    DiagnosisResult {
        cause,
        severity,
        user_actionable: true,
        recommendations,
        auto_fix_applied: Vec::new(),
        provider_used: None,
    }
}

// ── Config Restore Helper ──────────────────────────────────────────

fn try_restore_config_from_backup() -> bool {
    let backups = match crate::backup::list_backups() {
        Ok(b) => b,
        Err(_) => return false,
    };

    if let Some(latest) = backups.first() {
        crate::backup::restore_backup(&latest.name).is_ok()
    } else {
        false
    }
}
