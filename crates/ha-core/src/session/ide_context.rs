use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::session::SessionDB;
use crate::util::now_rfc3339;

const MAX_OPEN_TABS: usize = 24;
const MAX_SELECTION_TEXT_CHARS: usize = 600;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdeLineRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdeDiagnosticContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdeSymbolContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdeContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<IdeLineRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_diagnostic: Option<IdeDiagnosticContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_symbol: Option<IdeSymbolContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdeContextSnapshot {
    pub session_id: String,
    pub context: SessionIdeContext,
    pub updated_at: String,
}

impl SessionIdeContext {
    pub fn sanitized(mut self) -> Self {
        self.source = clean_string(self.source, 80);
        self.current_file = clean_path(self.current_file);
        self.open_tabs = self
            .open_tabs
            .into_iter()
            .filter_map(|path| clean_path(Some(path)))
            .take(MAX_OPEN_TABS)
            .collect();
        self.selection = self.selection.map(sanitize_range).filter(|range| {
            range.path.is_some()
                || range.start_line.is_some()
                || range.end_line.is_some()
                || range.text.is_some()
        });
        self.active_diagnostic = self
            .active_diagnostic
            .map(sanitize_diagnostic)
            .filter(|diag| diag.path.is_some() || diag.line.is_some() || diag.message.is_some());
        self.active_symbol = self.active_symbol.map(sanitize_symbol).filter(|symbol| {
            symbol.path.is_some() || symbol.line.is_some() || symbol.name.is_some()
        });
        self
    }

    pub fn relevant_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        push_path(&mut out, self.current_file.as_deref());
        push_path(
            &mut out,
            self.selection
                .as_ref()
                .and_then(|item| item.path.as_deref()),
        );
        push_path(
            &mut out,
            self.active_diagnostic
                .as_ref()
                .and_then(|item| item.path.as_deref()),
        );
        push_path(
            &mut out,
            self.active_symbol
                .as_ref()
                .and_then(|item| item.path.as_deref()),
        );
        for tab in &self.open_tabs {
            push_path(&mut out, Some(tab));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.current_file.is_none()
            && self.selection.is_none()
            && self.open_tabs.is_empty()
            && self.active_diagnostic.is_none()
            && self.active_symbol.is_none()
    }
}

impl SessionDB {
    pub fn save_session_ide_context(
        &self,
        session_id: &str,
        context: SessionIdeContext,
    ) -> Result<SessionIdeContextSnapshot> {
        let meta = self
            .get_session(session_id)?
            .ok_or_else(|| anyhow!("session not found: {session_id}"))?;
        if meta.incognito {
            bail!("Cannot persist IDE context for incognito session {session_id}");
        }
        let context = context.sanitized();
        let updated_at = now_rfc3339();
        let context_json = serde_json::to_string(&context)?;
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO session_ide_context (session_id, context_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                context_json = excluded.context_json,
                updated_at = excluded.updated_at",
            params![session_id, context_json, updated_at],
        )?;
        Ok(SessionIdeContextSnapshot {
            session_id: session_id.to_string(),
            context,
            updated_at,
        })
    }

    pub fn get_session_ide_context(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionIdeContextSnapshot>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT context_json, updated_at FROM session_ide_context WHERE session_id = ?1",
            params![session_id],
            |row| {
                let context_json: String = row.get(0)?;
                let updated_at: String = row.get(1)?;
                let context = serde_json::from_str::<SessionIdeContext>(&context_json)
                    .unwrap_or_default()
                    .sanitized();
                Ok(SessionIdeContextSnapshot {
                    session_id: session_id.to_string(),
                    context,
                    updated_at,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn clear_session_ide_context(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM session_ide_context WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }
}

fn sanitize_range(mut range: IdeLineRange) -> IdeLineRange {
    range.path = clean_path(range.path);
    range.text = clean_string(range.text, MAX_SELECTION_TEXT_CHARS);
    if let (Some(start), Some(end)) = (range.start_line, range.end_line) {
        if end < start {
            range.end_line = Some(start);
        }
    }
    range
}

fn sanitize_diagnostic(mut diagnostic: IdeDiagnosticContext) -> IdeDiagnosticContext {
    diagnostic.path = clean_path(diagnostic.path);
    diagnostic.severity = clean_string(diagnostic.severity, 40);
    diagnostic.message = clean_string(diagnostic.message, 240);
    diagnostic
}

fn sanitize_symbol(mut symbol: IdeSymbolContext) -> IdeSymbolContext {
    symbol.path = clean_path(symbol.path);
    symbol.name = clean_string(symbol.name, 160);
    symbol.kind = clean_string(symbol.kind, 80);
    symbol
}

fn clean_path(value: Option<String>) -> Option<String> {
    clean_string(value, 1024).map(|path| path.replace('\\', "/"))
}

fn clean_string(value: Option<String>, max_chars: usize) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| crate::truncate_utf8(&value, max_chars).to_string())
}

fn push_path(out: &mut Vec<String>, path: Option<&str>) {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return;
    };
    if !out.iter().any(|item| item == path) {
        out.push(path.to_string());
    }
}
