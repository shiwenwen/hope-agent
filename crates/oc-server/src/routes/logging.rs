use axum::extract::Query;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::logging;

use crate::error::AppError;
use crate::routes::helpers::app_state;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryLogsBody {
    pub filter: logging::LogFilter,
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub page_size: u32,
}

/// `POST /api/logs/query`
pub async fn query_logs(
    Json(body): Json<QueryLogsBody>,
) -> Result<Json<logging::LogQueryResult>, AppError> {
    let ps = if body.page_size == 0 {
        50
    } else {
        body.page_size.min(500)
    };
    let pg = if body.page == 0 { 1 } else { body.page };
    Ok(Json(app_state()?.log_db.query(&body.filter, pg, ps)?))
}

/// `GET /api/logs/stats`
pub async fn get_log_stats() -> Result<Json<logging::LogStats>, AppError> {
    let db_path = logging::db_path()?;
    Ok(Json(app_state()?.log_db.get_stats(&db_path)?))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClearLogsBody {
    pub before_date: Option<String>,
}

/// `POST /api/logs/clear`
///
/// Frontend ships `{beforeDate: null}` in the JSON body (it's a POST), so
/// we extract via `Json` rather than `Query`. Default impl lets empty /
/// missing body deserialize to "clear everything".
pub async fn clear_logs(Json(body): Json<ClearLogsBody>) -> Result<Json<Value>, AppError> {
    let n = app_state()?.log_db.clear(body.before_date.as_deref())?;
    Ok(Json(json!({ "removed": n })))
}

/// `GET /api/logs/config`
pub async fn get_log_config() -> Result<Json<logging::LogConfig>, AppError> {
    Ok(Json(app_state()?.logger.get_config()))
}

#[derive(Debug, Deserialize)]
pub struct LogConfigBody {
    pub config: logging::LogConfig,
}

/// `PUT /api/logs/config`
pub async fn save_log_config(
    Json(body): Json<LogConfigBody>,
) -> Result<Json<Value>, AppError> {
    logging::save_log_config(&body.config)?;
    app_state()?.logger.update_config(body.config);
    Ok(Json(json!({ "saved": true })))
}

/// `GET /api/logs/files`
pub async fn list_log_files() -> Result<Json<Vec<logging::LogFileInfo>>, AppError> {
    Ok(Json(logging::list_log_files()?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileQuery {
    pub filename: String,
    pub tail_lines: Option<u32>,
}

/// `GET /api/logs/file?filename=...`
pub async fn read_log_file(Query(q): Query<ReadFileQuery>) -> Result<Json<Value>, AppError> {
    let content = logging::read_log_file(&q.filename, q.tail_lines)?;
    Ok(Json(json!({ "content": content })))
}

/// `GET /api/logs/file-path`
pub async fn get_log_file_path() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "path": logging::current_log_file_path()? })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendLogBody {
    pub level: String,
    pub category: String,
    pub source: String,
    pub message: String,
    pub details: Option<String>,
    pub session_id: Option<String>,
}

/// `POST /api/logs/frontend`
pub async fn frontend_log(Json(body): Json<FrontendLogBody>) -> Result<Json<Value>, AppError> {
    let valid = ["error", "warn", "info", "debug"];
    let level = if valid.contains(&body.level.as_str()) {
        body.level
    } else {
        "info".to_string()
    };
    app_state()?.logger.log(
        &level,
        &body.category,
        &body.source,
        &body.message,
        body.details,
        body.session_id,
        None,
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct FrontendLogBatchBody {
    pub entries: Vec<serde_json::Value>,
}

/// `POST /api/logs/frontend-batch`
pub async fn frontend_log_batch(
    Json(body): Json<FrontendLogBatchBody>,
) -> Result<Json<Value>, AppError> {
    let valid = ["error", "warn", "info", "debug"];
    let lg = &app_state()?.logger;
    for entry in body.entries {
        let level = entry
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info");
        let level = if valid.contains(&level) {
            level
        } else {
            "info"
        };
        let category = entry
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("frontend");
        let source = entry
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("frontend");
        let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let details = entry
            .get("details")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let session_id = entry
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        lg.log(level, category, source, message, details, session_id, None);
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct ExportLogsBody {
    pub filter: logging::LogFilter,
    pub format: String,
}

/// `POST /api/logs/export`
pub async fn export_logs(Json(body): Json<ExportLogsBody>) -> Result<Json<Value>, AppError> {
    let logs = app_state()?.log_db.export(&body.filter)?;
    let out = match body.format.as_str() {
        "csv" => {
            let mut csv =
                String::from("id,timestamp,level,category,source,message,session_id,agent_id\n");
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
            csv
        }
        _ => serde_json::to_string_pretty(&logs)?,
    };
    Ok(Json(json!({ "data": out })))
}
