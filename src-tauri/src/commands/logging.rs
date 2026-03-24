use tauri::State;
use crate::AppState;
use crate::logging;

#[tauri::command]
pub async fn query_logs_cmd(
    filter: logging::LogFilter,
    page: u32,
    page_size: u32,
    state: State<'_, AppState>,
) -> Result<logging::LogQueryResult, String> {
    let ps = if page_size == 0 { 50 } else { page_size.min(500) };
    let pg = if page == 0 { 1 } else { page };
    state.log_db.query(&filter, pg, ps).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_log_stats_cmd(
    state: State<'_, AppState>,
) -> Result<logging::LogStats, String> {
    let db_path = logging::db_path().map_err(|e| e.to_string())?;
    state.log_db.get_stats(&db_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_logs_cmd(
    before_date: Option<String>,
    state: State<'_, AppState>,
) -> Result<u64, String> {
    state.log_db.clear(before_date.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_log_config_cmd(
    state: State<'_, AppState>,
) -> Result<logging::LogConfig, String> {
    Ok(state.logger.get_config())
}

#[tauri::command]
pub async fn save_log_config_cmd(
    config: logging::LogConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    logging::save_log_config(&config).map_err(|e| e.to_string())?;
    state.logger.update_config(config);
    Ok(())
}

#[tauri::command]
pub async fn list_log_files_cmd() -> Result<Vec<logging::LogFileInfo>, String> {
    logging::list_log_files().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_log_file_cmd(
    filename: String,
    tail_lines: Option<u32>,
) -> Result<String, String> {
    logging::read_log_file(&filename, tail_lines).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_log_file_path_cmd() -> Result<String, String> {
    logging::current_log_file_path().map_err(|e| e.to_string())
}

/// Receive log entries from the frontend and write them to the unified logging system.
#[tauri::command]
pub async fn frontend_log(
    level: String,
    category: String,
    source: String,
    message: String,
    details: Option<String>,
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Validate level
    let valid_levels = ["error", "warn", "info", "debug"];
    let level = if valid_levels.contains(&level.as_str()) { level } else { "info".to_string() };

    state.logger.log(
        &level,
        &category,
        &source,
        &message,
        details,
        session_id,
        None,
    );
    Ok(())
}

/// Receive a batch of log entries from the frontend.
#[tauri::command]
pub async fn frontend_log_batch(
    entries: Vec<serde_json::Value>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let valid_levels = ["error", "warn", "info", "debug"];
    for entry in entries {
        let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
        let level = if valid_levels.contains(&level) { level } else { "info" };
        let category = entry.get("category").and_then(|v| v.as_str()).unwrap_or("frontend");
        let source = entry.get("source").and_then(|v| v.as_str()).unwrap_or("frontend");
        let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let details = entry.get("details").and_then(|v| v.as_str()).map(|s| s.to_string());
        let session_id = entry.get("sessionId").and_then(|v| v.as_str()).map(|s| s.to_string());

        state.logger.log(level, category, source, message, details, session_id, None);
    }
    Ok(())
}

#[tauri::command]
pub async fn export_logs_cmd(
    filter: logging::LogFilter,
    format: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let logs = state.log_db.export(&filter).map_err(|e| e.to_string())?;
    match format.as_str() {
        "csv" => {
            let mut csv = String::from("id,timestamp,level,category,source,message,session_id,agent_id\n");
            for log in &logs {
                csv.push_str(&format!(
                    "{},{},{},{},{},\"{}\",{},{}\n",
                    log.id,
                    log.timestamp,
                    log.level,
                    log.category,
                    log.source,
                    log.message.replace('"', "\"\""),
                    log.session_id.as_deref().unwrap_or(""),
                    log.agent_id.as_deref().unwrap_or(""),
                ));
            }
            Ok(csv)
        }
        _ => {
            serde_json::to_string_pretty(&logs).map_err(|e| e.to_string())
        }
    }
}
