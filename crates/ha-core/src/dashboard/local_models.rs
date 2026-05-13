// ── Local Models dashboard query ────────────────────────────────
//
// Aggregates token usage / TTFT / error counts for sessions whose
// `provider_name` matches a configured local backend (Ollama / LiteLLM /
// vLLM / LM Studio / SGLang). The "is this provider local?" judgement is
// the single source of truth in [`provider::local::known_local_backends`]
// — the frontend never has to hardcode local hostnames or ports.

use anyhow::Result;
use rusqlite::types::ToSql;
use std::sync::Arc;

use crate::provider::{known_local_backend_matches, known_local_backends, ProviderConfig};
use crate::session::SessionDB;

use super::filters::{build_session_filter, params_ref, FilterClause};
use super::types::*;

/// Walk configured providers and return display names whose `(api_type,
/// base_url)` matches a known local backend. Order follows config order so
/// the UI display is stable across reloads.
pub fn local_provider_names_from(providers: &[ProviderConfig]) -> Vec<String> {
    let backends = known_local_backends();
    providers
        .iter()
        .filter(|p| {
            backends
                .iter()
                .any(|b| known_local_backend_matches(b, &p.api_type, &p.base_url))
        })
        .map(|p| p.name.clone())
        .collect()
}

/// Same as `local_provider_names_from` but reads the live `cached_config()`
/// snapshot. Use this from Tauri / HTTP entry points; the test-friendly form
/// above lets unit tests bypass the global config singleton.
pub fn local_provider_names() -> Vec<String> {
    let cfg = crate::config::cached_config();
    local_provider_names_from(&cfg.providers)
}

/// Aggregate usage for sessions whose `provider_name` is in
/// `local_provider_names`. An empty list short-circuits to all-zero with
/// `local_provider_names = []` so the UI can render an "configure a local
/// backend first" empty state without spurious queries.
pub fn query_local_model_usage(
    session_db: &Arc<SessionDB>,
    filter: &DashboardFilter,
    local_provider_names: &[String],
) -> Result<LocalModelUsage> {
    if local_provider_names.is_empty() {
        return Ok(LocalModelUsage {
            local_provider_names: Vec::new(),
            total_calls: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            avg_ttft_ms: None,
            trend: Vec::new(),
            by_model: Vec::new(),
        });
    }

    let conn = session_db
        .conn
        .lock()
        .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

    let in_placeholders = vec!["?"; local_provider_names.len()].join(", ");
    let in_clause = format!("s.provider_name IN ({in_placeholders})");

    // ── Daily trend ──
    let (trend_sql, trend_params) = build_query(
        filter,
        &in_clause,
        local_provider_names,
        "SELECT DATE(m.timestamp) as d,
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                AVG(CASE WHEN m.ttft_ms IS NOT NULL AND m.role = 'assistant' THEN m.ttft_ms END)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {where_sql}
         GROUP BY d
         ORDER BY d ASC",
    );
    let mut stmt = conn.prepare(&trend_sql)?;
    let rows = stmt.query_map(params_ref(&trend_params).as_slice(), |r| {
        Ok(TokenUsageTrend {
            date: r.get(0)?,
            input_tokens: crate::sql_u64(r, 1)?,
            output_tokens: crate::sql_u64(r, 2)?,
            avg_ttft_ms: r.get(3)?,
        })
    })?;
    let trend: Vec<TokenUsageTrend> = rows.collect::<std::result::Result<_, _>>()?;
    drop(stmt);

    // ── By model ──
    let (by_model_sql, by_model_params) = build_query(
        filter,
        &in_clause,
        local_provider_names,
        "SELECT COALESCE(s.model_id, 'unknown'),
                COALESCE(s.provider_name, 'unknown'),
                COUNT(DISTINCT CASE WHEN m.role = 'assistant' THEN m.id END),
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                AVG(CASE WHEN m.ttft_ms IS NOT NULL AND m.role = 'assistant' THEN m.ttft_ms END),
                COALESCE(SUM(CASE WHEN m.is_error = 1 THEN 1 ELSE 0 END), 0)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {where_sql}
         GROUP BY s.model_id, s.provider_name
         ORDER BY SUM(m.tokens_in) + SUM(m.tokens_out) DESC",
    );
    let mut stmt = conn.prepare(&by_model_sql)?;
    let rows = stmt.query_map(params_ref(&by_model_params).as_slice(), |r| {
        Ok(LocalModelUsageRow {
            model_id: r.get(0)?,
            provider_name: r.get(1)?,
            call_count: crate::sql_u64(r, 2)?,
            input_tokens: crate::sql_u64(r, 3)?,
            output_tokens: crate::sql_u64(r, 4)?,
            avg_ttft_ms: r.get(5)?,
            error_count: crate::sql_u64(r, 6)?,
        })
    })?;
    let by_model: Vec<LocalModelUsageRow> = rows.collect::<std::result::Result<_, _>>()?;
    drop(stmt);

    // ── Totals (single row) ──
    let (totals_sql, totals_params) = build_query(
        filter,
        &in_clause,
        local_provider_names,
        "SELECT COUNT(DISTINCT CASE WHEN m.role = 'assistant' THEN m.id END),
                COALESCE(SUM(m.tokens_in), 0),
                COALESCE(SUM(m.tokens_out), 0),
                AVG(CASE WHEN m.ttft_ms IS NOT NULL AND m.role = 'assistant' THEN m.ttft_ms END)
         FROM messages m
         JOIN sessions s ON s.id = m.session_id
         {where_sql}",
    );
    let (total_calls, total_input_tokens, total_output_tokens, avg_ttft_ms) =
        conn.query_row(&totals_sql, params_ref(&totals_params).as_slice(), |r| {
            Ok((
                crate::sql_u64(r, 0)?,
                crate::sql_u64(r, 1)?,
                crate::sql_u64(r, 2)?,
                r.get::<_, Option<f64>>(3)?,
            ))
        })?;

    Ok(LocalModelUsage {
        local_provider_names: local_provider_names.to_vec(),
        total_calls,
        total_input_tokens,
        total_output_tokens,
        avg_ttft_ms,
        trend,
        by_model,
    })
}

/// Build the (sql, params) pair for a local-model query by appending the
/// IN-clause to whatever `build_session_filter` produces. The template is
/// expected to contain the literal `{where_sql}` placeholder.
fn build_query(
    filter: &DashboardFilter,
    in_clause: &str,
    local_provider_names: &[String],
    template: &str,
) -> (String, Vec<Box<dyn ToSql>>) {
    let FilterClause {
        where_sql: base_where,
        mut params,
    } = build_session_filter(filter, "s", Some("m"));
    let where_sql = if base_where.is_empty() {
        format!("WHERE {in_clause}")
    } else {
        format!("{base_where} AND {in_clause}")
    };
    for name in local_provider_names {
        params.push(Box::new(name.clone()) as Box<dyn ToSql>);
    }
    (template.replace("{where_sql}", &where_sql), params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ApiType, ProviderConfig};
    use crate::session::SessionDB;
    use chrono::Utc;
    use rusqlite::params;

    fn make_provider(name: &str, base_url: &str, api: ApiType) -> ProviderConfig {
        ProviderConfig::new(name.into(), api, base_url.into(), String::new())
    }

    #[test]
    fn names_helper_picks_only_local_providers() {
        let providers = vec![
            make_provider(
                "Ollama (local)",
                "http://127.0.0.1:11434",
                ApiType::OpenaiChat,
            ),
            make_provider(
                "Anthropic",
                "https://api.anthropic.com/v1",
                ApiType::Anthropic,
            ),
            make_provider("LM Studio", "http://localhost:1234", ApiType::OpenaiChat),
            // wrong api type → not local even on loopback
            make_provider("Fake", "http://127.0.0.1:1234", ApiType::OpenaiResponses),
        ];
        let names = local_provider_names_from(&providers);
        assert_eq!(
            names,
            vec!["Ollama (local)".to_string(), "LM Studio".into()]
        );
    }

    #[test]
    fn names_helper_returns_empty_when_no_local() {
        let providers = vec![make_provider(
            "Anthropic",
            "https://api.anthropic.com/v1",
            ApiType::Anthropic,
        )];
        assert!(local_provider_names_from(&providers).is_empty());
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{}-{}-{}.sqlite3",
            name,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(unique)
    }

    fn insert_session(
        db: &SessionDB,
        id: &str,
        provider_name: Option<&str>,
        model_id: Option<&str>,
    ) {
        let conn = db.conn.lock().expect("lock");
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO sessions (id, title, agent_id, provider_id, provider_name, model_id, created_at, updated_at, is_cron, parent_session_id, incognito, title_source)
             VALUES (?1, NULL, 'ha-main', NULL, ?2, ?3, ?4, ?4, 0, NULL, 0, 'manual')",
            params![id, provider_name, model_id, now],
        )
        .expect("insert session");
    }

    fn insert_message(
        db: &SessionDB,
        session_id: &str,
        role: &str,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
        ttft_ms: Option<u64>,
        is_error: bool,
    ) {
        let conn = db.conn.lock().expect("lock");
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp, tokens_in, tokens_out, ttft_ms, is_error)
             VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                role,
                now,
                tokens_in.map(|v| v as i64),
                tokens_out.map(|v| v as i64),
                ttft_ms.map(|v| v as i64),
                if is_error { 1 } else { 0 }
            ],
        )
        .expect("insert message");
    }

    #[test]
    fn empty_local_names_short_circuits_to_zeros() {
        let path = temp_db_path("local-models-empty");
        let db = Arc::new(SessionDB::open(&path).expect("open"));
        let filter = DashboardFilter::default();
        let usage = query_local_model_usage(&db, &filter, &[]).expect("query");
        assert_eq!(usage.local_provider_names.len(), 0);
        assert_eq!(usage.total_calls, 0);
        assert!(usage.trend.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn aggregates_only_local_provider_sessions() {
        let path = temp_db_path("local-models-aggregate");
        let db = Arc::new(SessionDB::open(&path).expect("open"));

        // Local provider sessions
        insert_session(&db, "s-local-1", Some("Ollama (local)"), Some("qwen3:8b"));
        insert_message(&db, "s-local-1", "user", Some(0), Some(0), None, false);
        insert_message(
            &db,
            "s-local-1",
            "assistant",
            Some(100),
            Some(50),
            Some(120),
            false,
        );
        insert_message(
            &db,
            "s-local-1",
            "assistant",
            Some(80),
            Some(40),
            Some(80),
            true,
        );

        insert_session(&db, "s-local-2", Some("LM Studio"), Some("gemma4:e4b"));
        insert_message(
            &db,
            "s-local-2",
            "assistant",
            Some(200),
            Some(120),
            Some(200),
            false,
        );

        // Non-local session that should NOT show up
        insert_session(&db, "s-remote", Some("Anthropic"), Some("claude-opus-4-7"));
        insert_message(
            &db,
            "s-remote",
            "assistant",
            Some(1_000),
            Some(500),
            Some(800),
            false,
        );

        let names = vec!["Ollama (local)".to_string(), "LM Studio".to_string()];
        let usage =
            query_local_model_usage(&db, &DashboardFilter::default(), &names).expect("query");

        assert_eq!(usage.local_provider_names, names);
        assert_eq!(usage.total_calls, 3, "3 assistant rows from local sessions");
        assert_eq!(usage.total_input_tokens, 100 + 80 + 200);
        assert_eq!(usage.total_output_tokens, 50 + 40 + 120);
        assert_eq!(usage.by_model.len(), 2);

        // gemma4 first (highest tokens 200+120=320), then qwen3 (270)
        assert_eq!(usage.by_model[0].model_id, "gemma4:e4b");
        assert_eq!(usage.by_model[0].provider_name, "LM Studio");
        assert_eq!(usage.by_model[0].call_count, 1);
        assert_eq!(usage.by_model[0].error_count, 0);

        assert_eq!(usage.by_model[1].model_id, "qwen3:8b");
        assert_eq!(usage.by_model[1].provider_name, "Ollama (local)");
        assert_eq!(usage.by_model[1].call_count, 2);
        assert_eq!(usage.by_model[1].error_count, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn excludes_incognito_cron_and_subagent_sessions() {
        let path = temp_db_path("local-models-excludes");
        let db = Arc::new(SessionDB::open(&path).expect("open"));

        // A normal local session we expect to count
        insert_session(&db, "s-ok", Some("Ollama (local)"), Some("qwen3:8b"));
        insert_message(
            &db,
            "s-ok",
            "assistant",
            Some(100),
            Some(50),
            Some(100),
            false,
        );

        // is_cron = 1 — excluded
        {
            let conn = db.conn.lock().expect("lock");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO sessions (id, title, agent_id, provider_id, provider_name, model_id, created_at, updated_at, is_cron, parent_session_id, incognito, title_source)
                 VALUES ('s-cron', NULL, 'ha-main', NULL, 'Ollama (local)', 'qwen3:8b', ?1, ?1, 1, NULL, 0, 'manual')",
                params![now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, timestamp, tokens_in, tokens_out)
                 VALUES ('s-cron', 'assistant', '', ?1, 999, 999)",
                params![now],
            )
            .unwrap();
        }

        // parent_session_id set — subagent, excluded
        {
            let conn = db.conn.lock().expect("lock");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO sessions (id, title, agent_id, provider_id, provider_name, model_id, created_at, updated_at, is_cron, parent_session_id, incognito, title_source)
                 VALUES ('s-sub', NULL, 'ha-main', NULL, 'Ollama (local)', 'qwen3:8b', ?1, ?1, 0, 's-ok', 0, 'manual')",
                params![now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, timestamp, tokens_in, tokens_out)
                 VALUES ('s-sub', 'assistant', '', ?1, 777, 777)",
                params![now],
            )
            .unwrap();
        }

        // incognito = 1 — excluded
        {
            let conn = db.conn.lock().expect("lock");
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO sessions (id, title, agent_id, provider_id, provider_name, model_id, created_at, updated_at, is_cron, parent_session_id, incognito, title_source)
                 VALUES ('s-incog', NULL, 'ha-main', NULL, 'Ollama (local)', 'qwen3:8b', ?1, ?1, 0, NULL, 1, 'manual')",
                params![now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, timestamp, tokens_in, tokens_out)
                 VALUES ('s-incog', 'assistant', '', ?1, 555, 555)",
                params![now],
            )
            .unwrap();
        }

        let names = vec!["Ollama (local)".to_string()];
        let usage =
            query_local_model_usage(&db, &DashboardFilter::default(), &names).expect("query");

        assert_eq!(usage.total_calls, 1);
        assert_eq!(usage.total_input_tokens, 100);
        assert_eq!(usage.total_output_tokens, 50);

        let _ = std::fs::remove_file(&path);
    }
}
