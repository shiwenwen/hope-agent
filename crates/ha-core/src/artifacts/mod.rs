//! Local-first Artifact control plane built on the existing Canvas store.
//!
//! Canvas remains the rendering/runtime compatibility layer. This module owns
//! durable Artifact identity, immutable version metadata, optimistic
//! concurrency, verification, and portable exports.

mod analysis_renderer;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Duration, Utc};
use pulldown_cmark::{html, Event, Options, Parser};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::domain_workflow::{
    DomainArtifactExportGuardInput, DomainArtifactExportGuardReport, ListDomainEvidenceInput,
    RecordDomainEvidenceInput,
};
use crate::paths;

pub const ARTIFACT_SCHEMA_VERSION: &str = "hope.artifact.v1";
pub const ANALYSIS_SCHEMA_VERSION: &str = "hope.analysis-artifact.v1";
pub const EXPORT_GENERATOR_VERSION: &str = env!("CARGO_PKG_VERSION");

static ARTIFACT_PRIVACY_TRANSITION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn lock_privacy_transition() -> Result<std::sync::MutexGuard<'static, ()>> {
    ARTIFACT_PRIVACY_TRANSITION_LOCK
        .lock()
        .map_err(|_| anyhow!("Artifact privacy transition lock is poisoned"))
}

pub(crate) fn ensure_durable_session_allowed(session_id: Option<&str>) -> Result<()> {
    if request_is_incognito(false, session_id)? {
        bail!("durable Canvas and Artifact writes are disabled for incognito sessions");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Report,
    Dashboard,
    DataTable,
    Explainer,
    PrWalkthrough,
    Diagram,
    Slides,
    Custom,
}

impl ArtifactKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Dashboard => "dashboard",
            Self::DataTable => "data_table",
            Self::Explainer => "explainer",
            Self::PrWalkthrough => "pr_walkthrough",
            Self::Diagram => "diagram",
            Self::Slides => "slides",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "report" => Self::Report,
            "dashboard" => Self::Dashboard,
            "data_table" | "data-table" | "table" => Self::DataTable,
            "explainer" => Self::Explainer,
            "pr_walkthrough" | "pr-walkthrough" => Self::PrWalkthrough,
            "diagram" => Self::Diagram,
            "slides" => Self::Slides,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisArtifactV1 {
    #[serde(default = "analysis_schema")]
    pub schema_version: String,
    pub question: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub decision: String,
    #[serde(default = "analysis_ready")]
    pub status: String,
    #[serde(default)]
    pub metric_definitions: Vec<Value>,
    #[serde(default)]
    pub time_range: Option<Value>,
    #[serde(default)]
    pub filters: Vec<Value>,
    #[serde(default)]
    pub grain: Option<String>,
    #[serde(default)]
    pub datasets: Vec<Value>,
    #[serde(default)]
    pub findings: Vec<Value>,
    #[serde(default)]
    pub recommendations: Vec<Value>,
    #[serde(default)]
    pub caveats: Vec<Value>,
    #[serde(default)]
    pub blocks: Vec<Value>,
    #[serde(default)]
    pub charts: Vec<Value>,
    #[serde(default)]
    pub tables: Vec<Value>,
    #[serde(default)]
    pub static_fallbacks: Vec<Value>,
    #[serde(default)]
    pub sources: Vec<Value>,
    #[serde(default)]
    pub data_quality: Vec<Value>,
    #[serde(default)]
    pub claim_validation: Vec<Value>,
}

fn analysis_schema() -> String {
    ANALYSIS_SCHEMA_VERSION.to_string()
}

fn analysis_ready() -> String {
    "ready".to_string()
}

impl AnalysisArtifactV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ANALYSIS_SCHEMA_VERSION {
            bail!(
                "unsupported analysis artifact schema '{}'; expected '{}'",
                self.schema_version,
                ANALYSIS_SCHEMA_VERSION
            );
        }
        if self.question.trim().is_empty() {
            bail!("analysis artifact question must not be empty");
        }
        if !matches!(self.status.as_str(), "ready" | "partial" | "blocked") {
            bail!("analysis artifact status must be ready, partial, or blocked");
        }
        let mut source_ids = HashSet::new();
        for (index, source) in self.sources.iter().enumerate() {
            let id = required_string(source, "id", &format!("source {index}"))?;
            if !source_ids.insert(id.to_string()) {
                bail!("duplicate source id '{id}'");
            }
            let hash = required_string(source, "sha256", &format!("source {index}"))?;
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("source {index} has an invalid SHA-256 snapshot hash");
            }
        }
        let mut dataset_ids = HashSet::new();
        for (index, dataset) in self.datasets.iter().enumerate() {
            let id = required_string(dataset, "id", &format!("dataset {index}"))?;
            if !dataset_ids.insert(id.to_string()) {
                bail!("duplicate dataset id '{id}'");
            }
            let row_count = dataset
                .get("rowCount")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("dataset {index} is missing bounded rowCount"))?;
            let rows = dataset
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("dataset {index} is missing bounded rows"))?;
            if rows.len() > 5_000 {
                bail!("dataset {index} embeds more than 5000 rows; use a bounded summary");
            }
            if rows.len() as u64 > row_count {
                bail!("dataset {index} embeds more rows than rowCount");
            }
            if let Some(ids) = dataset.get("sourceIds").and_then(Value::as_array) {
                for source_id in ids.iter().filter_map(Value::as_str) {
                    if !source_ids.contains(source_id) {
                        bail!("dataset {index} references unknown source '{source_id}'");
                    }
                }
            }
        }
        let fallback_ids = self
            .static_fallbacks
            .iter()
            .chain(self.tables.iter())
            .filter_map(|value| value.get("id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        for (index, chart) in self.charts.iter().enumerate() {
            let source_id = chart
                .get("sourceId")
                .or_else(|| chart.get("source_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("chart {index} is missing sourceId"))?;
            if !source_ids.contains(source_id) {
                bail!("chart {index} references unknown source '{source_id}'");
            }
            let dataset_id = chart
                .get("dataset")
                .or_else(|| chart.get("datasetId"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("chart {index} is missing dataset binding"))?;
            if !dataset_ids.contains(dataset_id) {
                bail!("chart {index} references unknown dataset '{dataset_id}'");
            }
            let fallback_id = chart
                .get("fallbackId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("chart {index} is missing a static fallback"))?;
            if !fallback_ids.contains(fallback_id) {
                bail!("chart {index} references unknown fallback '{fallback_id}'");
            }
        }
        for (index, quality) in self.data_quality.iter().enumerate() {
            required_string(quality, "id", &format!("data-quality check {index}"))?;
            let dataset_id = quality
                .get("datasetId")
                .or_else(|| quality.get("dataset_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow!("data-quality check {index} is missing datasetId"))?;
            if !dataset_ids.contains(dataset_id) {
                bail!("data-quality check {index} references unknown dataset '{dataset_id}'");
            }
            required_string(quality, "check", &format!("data-quality check {index}"))?;
            required_string(quality, "method", &format!("data-quality check {index}"))?;
            let status =
                required_string(quality, "status", &format!("data-quality check {index}"))?;
            if !matches!(
                status,
                "passed" | "failed" | "warning" | "partial" | "inconclusive" | "not_run"
            ) {
                bail!("data-quality check {index} has unsupported status '{status}'");
            }
            if !quality.get("blocking").is_some_and(Value::is_boolean) {
                bail!("data-quality check {index} is missing boolean blocking");
            }
        }
        for (index, claim) in self.claim_validation.iter().enumerate() {
            required_string(claim, "claim", &format!("claim validation {index}"))?;
            required_string(claim, "metric", &format!("claim validation {index}"))?;
            required_string(claim, "denominator", &format!("claim validation {index}"))?;
            required_string(claim, "method", &format!("claim validation {index}"))?;
            let verdict = required_string(claim, "verdict", &format!("claim validation {index}"))?;
            if !matches!(
                verdict,
                "supported" | "unsupported" | "conflict" | "inconclusive"
            ) {
                bail!("claim validation {index} has unsupported verdict '{verdict}'");
            }
            let claim_source_ids = claim
                .get("sourceIds")
                .or_else(|| claim.get("source_ids"))
                .and_then(Value::as_array)
                .filter(|values| !values.is_empty())
                .ok_or_else(|| {
                    anyhow!("claim validation {index} is missing non-empty sourceIds")
                })?;
            for source_id in claim_source_ids {
                let source_id = source_id.as_str().ok_or_else(|| {
                    anyhow!("claim validation {index} contains a non-string sourceId")
                })?;
                if !source_ids.contains(source_id) {
                    bail!("claim validation {index} references unknown source '{source_id}'");
                }
            }
            if let Some(confidence) = claim.get("confidence") {
                let confidence = confidence.as_f64().ok_or_else(|| {
                    anyhow!("claim validation {index} confidence must be a number")
                })?;
                if !(0.0..=1.0).contains(&confidence) {
                    bail!("claim validation {index} confidence must be between 0 and 1");
                }
            }
        }
        if self.status == "ready"
            && self.data_quality.iter().any(|check| {
                check.get("blocking").and_then(Value::as_bool) == Some(true)
                    && check.get("status").and_then(Value::as_str) == Some("failed")
            })
        {
            bail!("ready analysis artifact contains a failed blocking data-quality check");
        }
        Ok(())
    }
}

fn required_string<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("{label} is missing {key}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub content_type: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub goal_id: Option<String>,
    pub lifecycle_state: String,
    pub privacy: String,
    pub current_version: i64,
    pub current_hash: String,
    pub payload_kind: String,
    pub analysis_status: Option<String>,
    pub source_count: usize,
    pub source_summaries: Vec<ArtifactSourceSummary>,
    pub evidence_summary: Value,
    pub capabilities: Value,
    pub verification: Option<VerificationReport>,
    pub project_path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactSourceSummary {
    pub id: String,
    pub label: String,
    pub source_type: String,
    pub sha256: String,
    pub access_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactVersionSummary {
    pub version_number: i64,
    pub parent_version: Option<i64>,
    pub content_hash: String,
    pub payload_kind: String,
    pub message: Option<String>,
    pub producer: Value,
    pub verification: Option<VerificationReport>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationReport {
    pub status: String,
    pub checks: Vec<VerificationCheck>,
    pub verified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactExportReceipt {
    pub id: String,
    pub artifact_id: String,
    pub version_number: i64,
    pub format: String,
    pub status: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub verification: Option<VerificationReport>,
    pub error: Option<String>,
    pub internal_path: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateArtifactInput {
    pub file_path: PathBuf,
    pub title: Option<String>,
    pub kind: ArtifactKind,
    pub privacy: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub goal_id: Option<String>,
    pub producer: Value,
    pub allowed_roots: Option<Vec<PathBuf>>,
    pub incognito: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateArtifactInput {
    pub artifact_id: String,
    pub file_path: PathBuf,
    pub expected_version: i64,
    pub title: Option<String>,
    pub message: Option<String>,
    pub producer: Value,
    pub allowed_roots: Option<Vec<PathBuf>>,
    pub incognito: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ListArtifactsInput {
    pub limit: usize,
    pub offset: usize,
    pub kind: Option<String>,
    pub lifecycle_state: Option<String>,
}

struct PreparedPayload {
    payload_kind: String,
    analysis_status: Option<String>,
    source_json: String,
    payload_json: String,
    canonical_bytes: Vec<u8>,
    index_html: String,
    markdown: Option<String>,
    suggested_title: Option<String>,
    capabilities: Value,
}

struct RestoreSource {
    html: Option<String>,
    css: Option<String>,
    js: Option<String>,
    content: Option<String>,
    content_type: String,
    payload_kind: Option<String>,
    payload_json: Option<String>,
    producer_json: Option<String>,
    sources_json: Option<String>,
    capabilities_json: Option<String>,
}

struct PendingPayloadBlob {
    path: PathBuf,
    bytes: Vec<u8>,
}

impl PendingPayloadBlob {
    fn materialize(&self) -> Result<bool> {
        if self.path.exists() {
            return Ok(false);
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::platform::write_atomic(&self.path, &self.bytes)?;
        Ok(true)
    }
}

struct ManagedFilesSnapshot {
    project_dir: PathBuf,
    project_dir_existed: bool,
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl ManagedFilesSnapshot {
    fn capture(project_dir: &Path) -> Result<Self> {
        let files = ["index.html", "artifact.json", "content.md"]
            .into_iter()
            .map(|name| {
                let path = project_dir.join(name);
                let contents = if path.exists() {
                    Some(fs::read(&path)?)
                } else {
                    None
                };
                Ok((path, contents))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            project_dir: project_dir.to_path_buf(),
            project_dir_existed: project_dir.exists(),
            files,
        })
    }

    fn restore(&self) -> Result<()> {
        for (path, contents) in &self.files {
            match contents {
                Some(contents) => crate::platform::write_atomic(path, contents)?,
                None if path.exists() => fs::remove_file(path)?,
                None => {}
            }
        }
        if !self.project_dir_existed && self.project_dir.exists() {
            fs::remove_dir_all(&self.project_dir)?;
        }
        Ok(())
    }
}

pub struct ArtifactService {
    conn: Connection,
}

/// Legacy Canvas remains readable and may keep using its historical mutation
/// path. Once a record has been produced through the Artifact control plane,
/// all mutations must flow through ArtifactService so hashes, evidence and
/// optimistic concurrency cannot diverge from `canvas_projects`.
pub fn ensure_legacy_canvas_mutation_allowed(artifact_id: &str) -> Result<()> {
    let service = ArtifactService::open()?;
    let record: Option<(String, String, bool)> = service
        .conn
        .query_row(
            "SELECT ar.producer_json, ar.capabilities_json,
                    EXISTS(SELECT 1 FROM artifact_version_meta av WHERE av.artifact_id = ar.id)
               FROM artifact_records ar WHERE ar.id = ?1",
            [artifact_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((producer_json, capabilities_json, has_version_meta)) = record else {
        return Ok(());
    };
    let legacy_producer = serde_json::from_str::<Value>(&producer_json)
        .ok()
        .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|kind| kind == "legacy_canvas");
    let legacy_capability = serde_json::from_str::<Value>(&capabilities_json)
        .ok()
        .and_then(|value| value.get("legacy").and_then(Value::as_bool))
        == Some(true);
    if legacy_producer && (!has_version_meta || legacy_capability) {
        return Ok(());
    }
    bail!(
        "Canvas mutation is disabled for managed Artifact '{}'; use the artifact tool with expected_version",
        artifact_id
    )
}

/// Refresh the Artifact façade after a successful mutation through the
/// compatibility Canvas API. Managed Artifacts never reach this path; legacy
/// Canvas records use it to keep the current hash and immutable metadata in
/// sync until the old API is retired.
pub fn sync_legacy_canvas_current_version(artifact_id: &str) -> Result<()> {
    let service = ArtifactService::open()?;
    sync_one_legacy_canvas_record(&service.conn, artifact_id)?;
    backfill_legacy_current_version(&service.conn, artifact_id)
}

impl ArtifactService {
    pub fn open() -> Result<Self> {
        let db_path = paths::canvas_db_path()?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        ensure_tables(&conn)?;
        sync_legacy_canvas_records(&conn)?;
        cleanup_expired_exports(&conn)?;
        gc_unreferenced_blobs(&conn)?;
        Ok(Self { conn })
    }

    pub fn list(&self, input: ListArtifactsInput) -> Result<Vec<ArtifactRecord>> {
        sync_legacy_canvas_records(&self.conn)?;
        let limit = input.limit.clamp(1, 200) as i64;
        let offset = input.offset as i64;
        let kind = input.kind.as_deref().filter(|v| !v.trim().is_empty());
        let state = input
            .lifecycle_state
            .as_deref()
            .filter(|v| !v.trim().is_empty());
        let mut stmt = self.conn.prepare(
            "SELECT cp.id, cp.title, ar.kind, cp.content_type, cp.session_id,
                    ar.project_id, cp.agent_id, ar.goal_id, ar.lifecycle_state,
                    ar.privacy, cp.version_count, ar.current_hash,
                    ar.payload_kind, ar.analysis_status, ar.sources_json,
                    ar.capabilities_json, ar.evidence_summary_json,
                    ar.verification_json, cp.created_at, cp.updated_at
               FROM artifact_records ar
               JOIN canvas_projects cp ON cp.id = ar.id
              WHERE (?1 IS NULL OR ar.kind = ?1)
                AND (?2 IS NULL OR ar.lifecycle_state = ?2)
              ORDER BY cp.updated_at DESC
              LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt.query_map(params![kind, state, limit, offset], map_artifact_row)?;
        let records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        records
            .into_iter()
            .filter_map(
                |record| match request_is_incognito(false, record.session_id.as_deref()) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(record)),
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<Option<ArtifactRecord>> {
        sync_one_legacy_canvas_record(&self.conn, id)?;
        backfill_legacy_current_version(&self.conn, id)?;
        let mut record = self
            .conn
            .query_row(
                "SELECT cp.id, cp.title, ar.kind, cp.content_type, cp.session_id,
                        ar.project_id, cp.agent_id, ar.goal_id, ar.lifecycle_state,
                        ar.privacy, cp.version_count, ar.current_hash,
                        ar.payload_kind, ar.analysis_status, ar.sources_json,
                        ar.capabilities_json, ar.evidence_summary_json,
                        ar.verification_json, cp.created_at, cp.updated_at
                   FROM artifact_records ar
                   JOIN canvas_projects cp ON cp.id = ar.id
                  WHERE ar.id = ?1",
                [id],
                map_artifact_row,
            )
            .optional()
            .map_err(anyhow::Error::from)?;
        if record
            .as_ref()
            .is_some_and(|record| record.payload_kind == "analysis")
        {
            match self.refresh_analysis_projection(id) {
                Ok(true) => {
                    if let Some(record) = record.as_mut() {
                        record.verification = None;
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    // The refresh may already have invalidated verification
                    // before a filesystem failure. Return fail-closed even
                    // when this in-memory record was loaded beforehand.
                    if let Some(record) = record.as_mut() {
                        record.verification = None;
                    }
                    app_warn!(
                        "artifact",
                        "refresh_analysis_projection",
                        "failed to refresh derived analysis preview for {}: {}",
                        id,
                        error
                    );
                }
            }
        }
        match record {
            Some(record) if request_is_incognito(false, record.session_id.as_deref())? => Ok(None),
            other => Ok(other),
        }
    }

    /// Rebuild the current reading surface from the immutable analysis payload.
    /// `index.html` is a derived projection: refreshing it must never change the
    /// Artifact version, canonical hash, evidence, or source snapshot.
    pub fn refresh_analysis_projection(&self, id: &str) -> Result<bool> {
        let payload: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT av.payload_kind, av.payload_json
                   FROM artifact_records ar
                   JOIN canvas_projects cp ON cp.id = ar.id
                   JOIN artifact_version_meta av
                     ON av.artifact_id = ar.id
                    AND av.version_number = cp.version_count
                  WHERE ar.id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((payload_kind, payload_json)) = payload else {
            return Ok(false);
        };
        if payload_kind != "analysis" {
            return Ok(false);
        }
        let analysis: AnalysisArtifactV1 = serde_json::from_str(&payload_json)
            .context("parsing managed AnalysisArtifactV1 projection")?;
        analysis.validate()?;
        let html = render_analysis_html(&analysis);
        let path = paths::canvas_project_dir(id)?.join("index.html");
        if fs::read(&path).ok().as_deref() == Some(html.as_bytes()) {
            return Ok(false);
        }
        // Verification describes the rendered bytes, not only the immutable
        // payload. Clear it before replacing the projection so a failed write
        // leaves the artifact safely unverified rather than falsely passed.
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE artifact_records SET verification_json = NULL WHERE id = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE artifact_version_meta SET verification_json = NULL
              WHERE artifact_id = ?1
                AND version_number = (
                    SELECT version_count FROM canvas_projects WHERE id = ?1
                )",
            [id],
        )?;
        tx.commit()?;
        crate::platform::write_atomic(&path, html.as_bytes())?;
        Ok(true)
    }

    pub fn versions(&self, id: &str) -> Result<Vec<ArtifactVersionSummary>> {
        if self.get(id)?.is_none() {
            bail!("artifact '{}' not found", id);
        }
        let mut stmt = self.conn.prepare(
            "SELECT cv.version_number, av.parent_version,
                    COALESCE(av.content_hash, ''), COALESCE(av.payload_kind, 'freeform'),
                    cv.message, COALESCE(av.producer_json, '{}'),
                    av.verification_json, cv.created_at
               FROM canvas_versions cv
               LEFT JOIN artifact_version_meta av
                 ON av.artifact_id = cv.project_id
                AND av.version_number = cv.version_number
              WHERE cv.project_id = ?1
              ORDER BY cv.version_number DESC",
        )?;
        let rows = stmt.query_map([id], |row| {
            let producer_json: String = row.get(5)?;
            let verification_json: Option<String> = row.get(6)?;
            Ok(ArtifactVersionSummary {
                version_number: row.get(0)?,
                parent_version: row.get(1)?,
                content_hash: row.get(2)?,
                payload_kind: row.get(3)?,
                message: row.get(4)?,
                producer: serde_json::from_str(&producer_json).unwrap_or_else(|_| json!({})),
                verification: verification_json.and_then(|v| serde_json::from_str(&v).ok()),
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_from_file(&mut self, input: CreateArtifactInput) -> Result<ArtifactRecord> {
        let _privacy_guard = lock_privacy_transition()?;
        let privacy = normalize_durable_privacy(&input.privacy)?.to_string();
        if request_is_incognito(input.incognito, input.session_id.as_deref())? {
            bail!("incognito artifacts are memory-only; durable artifact creation is disabled");
        }
        let file_path = validate_source_path(&input.file_path, input.allowed_roots.as_deref())?;
        let prepared = prepare_payload(&file_path)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let title = input
            .title
            .filter(|v| !v.trim().is_empty())
            .or(prepared.suggested_title.clone())
            .unwrap_or_else(|| "Untitled Artifact".to_string());
        let hash = sha256_hex(&prepared.canonical_bytes);
        let project_dir = paths::canvas_project_dir(&id)?;

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO canvas_projects
             (id, title, content_type, session_id, agent_id, created_at, updated_at, version_count, metadata)
             VALUES (?1, ?2, 'html', ?3, ?4, ?5, ?5, 1, NULL)",
            params![id, title, input.session_id, input.agent_id, now],
        )?;
        tx.execute(
            "INSERT INTO canvas_versions
             (project_id, version_number, message, html, css, js, content, created_at)
             VALUES (?1, 1, 'Initial artifact version', ?2, NULL, NULL, ?3, ?4)",
            params![id, prepared.index_html, prepared.markdown, now],
        )?;
        tx.execute(
            "INSERT INTO artifact_records
             (id, kind, project_id, goal_id, lifecycle_state, privacy, current_hash,
              producer_json, capabilities_json, sources_json, evidence_ids_json,
              verification_json, payload_kind, analysis_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7, ?8, ?9, '[]',
                     NULL, ?10, ?11, ?12, ?12)",
            params![
                id,
                input.kind.as_str(),
                input.project_id,
                input.goal_id,
                privacy,
                hash,
                input.producer.to_string(),
                prepared.capabilities.to_string(),
                prepared.source_json,
                prepared.payload_kind,
                prepared.analysis_status,
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO artifact_version_meta
             (artifact_id, version_number, parent_version, payload_kind, payload_json,
              content_hash, producer_json, capabilities_json, sources_json,
              evidence_ids_json, verification_json, created_at)
             VALUES (?1, 1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, '[]', NULL, ?8)",
            params![
                id,
                prepared.payload_kind,
                prepared.payload_json,
                hash,
                input.producer.to_string(),
                prepared.capabilities.to_string(),
                prepared.source_json,
                now
            ],
        )?;
        let pending_blob = register_payload_blob(&tx, &id, 1, &prepared)?;
        if let Err(error) = write_payload_files(&project_dir, &prepared) {
            let _ = fs::remove_dir_all(&project_dir);
            return Err(error);
        }
        let blob_created = match pending_blob.materialize() {
            Ok(created) => created,
            Err(error) => {
                let _ = fs::remove_dir_all(&project_dir);
                return Err(error);
            }
        };
        if let Err(error) = tx.commit() {
            let _ = fs::remove_dir_all(&project_dir);
            remove_blob_if_unreferenced(&self.conn, &pending_blob, blob_created);
            return Err(error.into());
        }
        let evidence = record_artifact_evidence(
            input.goal_id.as_deref(),
            input.session_id.as_deref(),
            input.project_id.as_deref(),
            &id,
            &title,
            input.kind.as_str(),
            &privacy,
            &project_dir,
            &prepared,
            1,
        );
        if !evidence.ids.is_empty() {
            self.conn.execute(
                "UPDATE artifact_records
                    SET evidence_ids_json = ?1, evidence_summary_json = ?2
                  WHERE id = ?3",
                params![
                    serde_json::to_string(&evidence.ids)?,
                    evidence.summary.to_string(),
                    id
                ],
            )?;
            self.conn.execute(
                "UPDATE artifact_version_meta
                    SET evidence_ids_json = ?1, evidence_summary_json = ?2
                  WHERE artifact_id = ?3 AND version_number = 1",
                params![
                    serde_json::to_string(&evidence.ids)?,
                    evidence.summary.to_string(),
                    id
                ],
            )?;
        }
        emit_artifact_event("artifact:created", &id, 1, Some(&title));
        self.get(&id)?
            .ok_or_else(|| anyhow!("artifact disappeared after creation"))
    }

    pub fn update_from_file(&mut self, input: UpdateArtifactInput) -> Result<ArtifactRecord> {
        let _privacy_guard = lock_privacy_transition()?;
        if input.incognito {
            bail!("incognito artifacts are memory-only; durable artifact update is disabled");
        }
        let current = self
            .get(&input.artifact_id)?
            .ok_or_else(|| anyhow!("artifact '{}' not found", input.artifact_id))?;
        if request_is_incognito(false, current.session_id.as_deref())? {
            bail!("incognito artifacts are memory-only; durable artifact update is disabled");
        }
        if current.current_version != input.expected_version {
            bail!(
                "artifact version conflict: expected {}, current {} ({})",
                input.expected_version,
                current.current_version,
                current.current_hash
            );
        }
        let file_path = validate_source_path(&input.file_path, input.allowed_roots.as_deref())?;
        let prepared = prepare_payload(&file_path)?;
        let now = Utc::now().to_rfc3339();
        let hash = sha256_hex(&prepared.canonical_bytes);
        let project_dir = paths::canvas_project_dir(&input.artifact_id)?;
        let title = input
            .title
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| current.title.clone());

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (locked_version, locked_hash): (i64, String) = tx.query_row(
            "SELECT cp.version_count, ar.current_hash
               FROM canvas_projects cp
               JOIN artifact_records ar ON ar.id = cp.id
              WHERE cp.id = ?1",
            [input.artifact_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if locked_version != input.expected_version {
            bail!(
                "artifact version conflict: expected {}, current {} ({})",
                input.expected_version,
                locked_version,
                locked_hash
            );
        }
        let new_version = locked_version + 1;
        tx.execute(
            "INSERT INTO canvas_versions
             (project_id, version_number, message, html, css, js, content, created_at)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)",
            params![
                input.artifact_id,
                new_version,
                input.message,
                prepared.index_html,
                prepared.markdown,
                now
            ],
        )?;
        tx.execute(
            "UPDATE canvas_projects
                SET title = ?1, updated_at = ?2, version_count = ?3
              WHERE id = ?4",
            params![title, now, new_version, input.artifact_id],
        )?;
        tx.execute(
            "UPDATE artifact_records
                SET current_hash = ?1, producer_json = ?2, sources_json = ?3,
                    capabilities_json = ?4, verification_json = NULL, payload_kind = ?5,
                    analysis_status = ?6, updated_at = ?7
              WHERE id = ?8",
            params![
                hash,
                input.producer.to_string(),
                prepared.source_json,
                prepared.capabilities.to_string(),
                prepared.payload_kind,
                prepared.analysis_status,
                now,
                input.artifact_id
            ],
        )?;
        tx.execute(
            "INSERT INTO artifact_version_meta
             (artifact_id, version_number, parent_version, payload_kind, payload_json,
              content_hash, producer_json, capabilities_json, sources_json,
              evidence_ids_json, verification_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '[]', NULL, ?10)",
            params![
                input.artifact_id,
                new_version,
                locked_version,
                prepared.payload_kind,
                prepared.payload_json,
                hash,
                input.producer.to_string(),
                prepared.capabilities.to_string(),
                prepared.source_json,
                now
            ],
        )?;
        let pending_blob = register_payload_blob(&tx, &input.artifact_id, new_version, &prepared)?;
        let snapshot = ManagedFilesSnapshot::capture(&project_dir)?;
        if let Err(error) = write_payload_files(&project_dir, &prepared) {
            if let Err(restore_error) = snapshot.restore() {
                app_warn!(
                    "artifact",
                    "update_rollback",
                    "failed to restore managed files after write error: {}",
                    restore_error
                );
            }
            return Err(error);
        }
        let blob_created = match pending_blob.materialize() {
            Ok(created) => created,
            Err(error) => {
                if let Err(restore_error) = snapshot.restore() {
                    app_warn!(
                        "artifact",
                        "update_rollback",
                        "failed to restore managed files after blob error: {}",
                        restore_error
                    );
                }
                return Err(error);
            }
        };
        if let Err(error) = tx.commit() {
            if let Err(restore_error) = snapshot.restore() {
                app_warn!(
                    "artifact",
                    "update_rollback",
                    "failed to restore managed files after database error: {}",
                    restore_error
                );
            }
            remove_blob_if_unreferenced(&self.conn, &pending_blob, blob_created);
            return Err(error.into());
        }
        let evidence = record_artifact_evidence(
            current.goal_id.as_deref(),
            current.session_id.as_deref(),
            current.project_id.as_deref(),
            &input.artifact_id,
            &title,
            &current.kind,
            &current.privacy,
            &project_dir,
            &prepared,
            new_version,
        );
        if !evidence.ids.is_empty() {
            let ids = serde_json::to_string(&evidence.ids)?;
            self.conn.execute(
                "UPDATE artifact_records
                    SET evidence_ids_json = ?1, evidence_summary_json = ?2
                  WHERE id = ?3",
                params![ids, evidence.summary.to_string(), input.artifact_id],
            )?;
            self.conn.execute(
                "UPDATE artifact_version_meta
                    SET evidence_ids_json = ?1, evidence_summary_json = ?2
                  WHERE artifact_id = ?3 AND version_number = ?4",
                params![
                    serde_json::to_string(&evidence.ids)?,
                    evidence.summary.to_string(),
                    input.artifact_id,
                    new_version
                ],
            )?;
        }
        emit_artifact_event(
            "artifact:updated",
            &input.artifact_id,
            new_version,
            Some(&title),
        );
        self.get(&input.artifact_id)?
            .ok_or_else(|| anyhow!("artifact disappeared after update"))
    }

    pub fn restore(&mut self, artifact_id: &str, version_number: i64) -> Result<ArtifactRecord> {
        let _privacy_guard = lock_privacy_transition()?;
        let current = self
            .get(artifact_id)?
            .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
        if request_is_incognito(false, current.session_id.as_deref())? {
            bail!("incognito artifacts are memory-only; durable artifact restore is disabled");
        }
        let source: RestoreSource = self
            .conn
            .query_row(
                "SELECT cv.html, cv.css, cv.js, cv.content, cp.content_type,
                        av.payload_kind, av.payload_json, av.producer_json,
                        av.sources_json, av.capabilities_json
                   FROM canvas_versions cv
                   JOIN canvas_projects cp ON cp.id = cv.project_id
                   LEFT JOIN artifact_version_meta av
                     ON av.artifact_id = cv.project_id
                    AND av.version_number = cv.version_number
                  WHERE cv.project_id = ?1 AND cv.version_number = ?2",
                params![artifact_id, version_number],
                |row| {
                    Ok(RestoreSource {
                        html: row.get(0)?,
                        css: row.get(1)?,
                        js: row.get(2)?,
                        content: row.get(3)?,
                        content_type: row.get(4)?,
                        payload_kind: row.get(5)?,
                        payload_json: row.get(6)?,
                        producer_json: row.get(7)?,
                        sources_json: row.get(8)?,
                        capabilities_json: row.get(9)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| anyhow!("version {} not found", version_number))?;
        let source_capabilities = source
            .capabilities_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .unwrap_or_else(|| json!({}));
        let metadata_is_legacy = source_capabilities.get("legacy").and_then(Value::as_bool)
            == Some(true)
            || source
                .payload_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| {
                    value
                        .get("sourceFormat")
                        .and_then(Value::as_str)
                        .map(|value| value == "legacy_canvas")
                })
                == Some(true);
        let has_managed_metadata =
            source.payload_kind.is_some() && source.payload_json.is_some() && !metadata_is_legacy;
        let prepared = if has_managed_metadata {
            let payload_kind = source.payload_kind.clone().unwrap_or_default();
            let payload_json = source.payload_json.clone().unwrap_or_default();
            let canonical_bytes = if payload_kind == "analysis" {
                payload_json.as_bytes().to_vec()
            } else {
                serde_json::from_str::<Value>(&payload_json)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("source")
                            .and_then(Value::as_str)
                            .map(|value| value.as_bytes().to_vec())
                    })
                    .unwrap_or_else(|| payload_json.as_bytes().to_vec())
            };
            let analysis_status = (payload_kind == "analysis")
                .then(|| serde_json::from_str::<AnalysisArtifactV1>(&payload_json).ok())
                .flatten()
                .map(|analysis| analysis.status);
            PreparedPayload {
                payload_kind,
                analysis_status,
                source_json: source
                    .sources_json
                    .clone()
                    .unwrap_or_else(|| "[]".to_string()),
                payload_json,
                canonical_bytes,
                index_html: source.html.clone().unwrap_or_default(),
                markdown: source.content.clone(),
                suggested_title: None,
                capabilities: source_capabilities,
            }
        } else {
            let payload = json!({
                "sourceFormat": "legacy_canvas",
                "html": source.html,
                "css": source.css,
                "js": source.js,
                "content": source.content,
            });
            let canonical_bytes = serde_json::to_vec(&payload)?;
            let index_html = crate::tools::canvas::renderer::render_project_page(
                &source.content_type,
                source.html.as_deref(),
                source.css.as_deref(),
                source.js.as_deref(),
                source.content.as_deref(),
                None,
            );
            PreparedPayload {
                payload_kind: "freeform".to_string(),
                analysis_status: None,
                source_json: "[]".to_string(),
                payload_json: payload.to_string(),
                canonical_bytes,
                index_html,
                markdown: (source.content_type == "markdown")
                    .then(|| source.content.clone())
                    .flatten(),
                suggested_title: None,
                capabilities: json!({
                    "network": false,
                    "scripts": source.js.as_ref().is_some_and(|value| !value.is_empty()),
                    "attachments": false,
                    "legacy": true,
                    "schemaVersion": ARTIFACT_SCHEMA_VERSION
                }),
            }
        };
        let restored_hash = sha256_hex(&prepared.canonical_bytes);
        let producer_json = source
            .producer_json
            .unwrap_or_else(|| json!({"type":"legacy_canvas"}).to_string());
        let now = Utc::now().to_rfc3339();
        let project_dir = paths::canvas_project_dir(artifact_id)?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let locked_version: i64 = tx.query_row(
            "SELECT version_count FROM canvas_projects WHERE id = ?1",
            [artifact_id],
            |row| row.get(0),
        )?;
        let new_version = locked_version + 1;
        let (version_html, version_css, version_js, version_content) = if has_managed_metadata {
            (
                Some(prepared.index_html.clone()),
                None,
                None,
                prepared.markdown.clone(),
            )
        } else {
            (
                source.html.clone(),
                source.css.clone(),
                source.js.clone(),
                source.content.clone(),
            )
        };
        tx.execute(
            "INSERT INTO canvas_versions
             (project_id, version_number, message, html, css, js, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                artifact_id,
                new_version,
                format!("Restored from version {version_number}"),
                version_html,
                version_css,
                version_js,
                version_content,
                now
            ],
        )?;
        tx.execute(
            "UPDATE canvas_projects SET updated_at = ?1, version_count = ?2 WHERE id = ?3",
            params![now, new_version, artifact_id],
        )?;
        tx.execute(
            "UPDATE artifact_records
                SET current_hash = ?1, payload_kind = ?2, producer_json = ?3,
                    sources_json = ?4, capabilities_json = ?5,
                    analysis_status = ?6, verification_json = NULL, updated_at = ?7
              WHERE id = ?8",
            params![
                restored_hash,
                prepared.payload_kind,
                producer_json,
                prepared.source_json,
                prepared.capabilities.to_string(),
                prepared.analysis_status,
                now,
                artifact_id
            ],
        )?;
        tx.execute(
            "INSERT INTO artifact_version_meta
             (artifact_id, version_number, parent_version, payload_kind, payload_json,
              content_hash, producer_json, capabilities_json, sources_json,
              evidence_ids_json, verification_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '[]', NULL, ?10)",
            params![
                artifact_id,
                new_version,
                locked_version,
                prepared.payload_kind,
                prepared.payload_json,
                restored_hash,
                producer_json,
                prepared.capabilities.to_string(),
                prepared.source_json,
                now
            ],
        )?;
        let pending_blob = register_payload_blob(&tx, artifact_id, new_version, &prepared)?;
        let snapshot = ManagedFilesSnapshot::capture(&project_dir)?;
        if let Err(error) = write_payload_files(&project_dir, &prepared) {
            if let Err(restore_error) = snapshot.restore() {
                app_warn!(
                    "artifact",
                    "restore_rollback",
                    "failed to restore managed files after write error: {}",
                    restore_error
                );
            }
            return Err(error);
        }
        let blob_created = match pending_blob.materialize() {
            Ok(created) => created,
            Err(error) => {
                if let Err(restore_error) = snapshot.restore() {
                    app_warn!(
                        "artifact",
                        "restore_rollback",
                        "failed to restore managed files after blob error: {}",
                        restore_error
                    );
                }
                return Err(error);
            }
        };
        if let Err(error) = tx.commit() {
            if let Err(restore_error) = snapshot.restore() {
                app_warn!(
                    "artifact",
                    "restore_rollback",
                    "failed to restore managed files after database error: {}",
                    restore_error
                );
            }
            remove_blob_if_unreferenced(&self.conn, &pending_blob, blob_created);
            return Err(error.into());
        }
        emit_artifact_event(
            "artifact:updated",
            artifact_id,
            new_version,
            Some(&current.title),
        );
        self.get(artifact_id)?
            .ok_or_else(|| anyhow!("artifact disappeared after restore"))
    }

    pub fn verify(&self, artifact_id: &str) -> Result<VerificationReport> {
        let artifact = self
            .get(artifact_id)?
            .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
        let html_path = paths::canvas_project_dir(artifact_id)?.join("index.html");
        let body = fs::read_to_string(&html_path)
            .with_context(|| format!("reading {}", html_path.display()))?;
        let mut checks = Vec::new();
        let version_payload: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT payload_kind, payload_json FROM artifact_version_meta
                  WHERE artifact_id = ?1 AND version_number = ?2",
                params![artifact_id, artifact.current_version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((payload_kind, payload_json)) = version_payload.as_ref() {
            let canonical = if payload_kind == "analysis" {
                payload_json.as_bytes().to_vec()
            } else {
                serde_json::from_str::<Value>(payload_json)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("source")
                            .and_then(Value::as_str)
                            .map(str::as_bytes)
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| payload_json.as_bytes().to_vec())
            };
            checks.push(check(
                "content_hash",
                sha256_hex(&canonical) == artifact.current_hash,
                "The current immutable version matches its SHA-256 content hash.",
            ));
            if artifact.capabilities.get("legacy").and_then(Value::as_bool) != Some(true) {
                let managed_payload = fs::read_to_string(
                    paths::canvas_project_dir(artifact_id)?.join("artifact.json"),
                )
                .unwrap_or_default();
                checks.push(check(
                    "managed_payload",
                    managed_payload == payload_json.as_str(),
                    "The managed artifact.json matches version metadata.",
                ));
            }
        }
        checks.push(check(
            "html_document",
            body.contains("<!DOCTYPE html") || body.contains("<html"),
            "A readable HTML document exists.",
        ));
        checks.push(check(
            "content_security_policy",
            body.to_ascii_lowercase()
                .contains("content-security-policy"),
            "The document carries an explicit offline Content Security Policy.",
        ));
        let has_remote = contains_remote_dependency(&body);
        checks.push(check(
            "offline_dependencies",
            !has_remote,
            if has_remote {
                "Remote http(s) resources or network APIs were detected."
            } else {
                "No remote resource references were detected."
            },
        ));
        let has_external_navigation = contains_external_navigation(&body);
        checks.push(check(
            "external_navigation",
            !has_external_navigation,
            if has_external_navigation {
                "External document navigation or redirect code was detected."
            } else {
                "No external document navigation or redirect code was detected."
            },
        ));
        let lower = body.to_ascii_lowercase();
        let has_forbidden_embeds = ["<iframe", "<object", "<embed", "<form"]
            .iter()
            .any(|needle| lower.contains(needle));
        checks.push(check(
            "forbidden_embeds",
            !has_forbidden_embeds,
            "No iframe, object, embed, or form elements are present.",
        ));
        let has_semantic = body.contains("<h1")
            || body.contains("<h2")
            || body.contains("<main")
            || body.contains("<article")
            || body.contains("<p");
        checks.push(check(
            "semantic_fallback",
            has_semantic,
            "The document contains semantic readable content.",
        ));
        let status = if checks.iter().all(|c| c.status == "passed") {
            "passed"
        } else {
            "failed"
        }
        .to_string();
        let report = VerificationReport {
            status,
            checks,
            verified_at: Utc::now().to_rfc3339(),
        };
        let report_json = serde_json::to_string(&report)?;
        self.conn.execute(
            "UPDATE artifact_records SET verification_json = ?1 WHERE id = ?2",
            params![report_json, artifact_id],
        )?;
        self.conn.execute(
            "UPDATE artifact_version_meta SET verification_json = ?1
              WHERE artifact_id = ?2 AND version_number = ?3",
            params![
                serde_json::to_string(&report)?,
                artifact_id,
                artifact.current_version
            ],
        )?;
        emit_artifact_event(
            "artifact:verified",
            artifact_id,
            artifact.current_version,
            Some(&report.status),
        );
        Ok(report)
    }

    /// Record the owner-side audience/redaction review required before a
    /// guarded Artifact can become a shareable package.
    pub fn review_for_export(
        &self,
        artifact_id: &str,
        audience: &str,
        redaction_checked: bool,
    ) -> Result<DomainArtifactExportGuardReport> {
        if !redaction_checked {
            bail!("export review requires an explicit redaction confirmation");
        }
        if audience.trim().is_empty() {
            bail!("export review requires the intended audience");
        }
        let artifact = self
            .get(artifact_id)?
            .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
        let session_id = artifact
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("artifact export review requires a session-scoped Artifact"))?;
        let db = crate::globals::get_session_db()
            .ok_or_else(|| anyhow!("session database is unavailable"))?;
        db.record_domain_evidence(RecordDomainEvidenceInput {
            goal_id: artifact.goal_id.clone(),
            session_id: Some(session_id.to_string()),
            project_id: artifact.project_id.clone(),
            domain: artifact_domain(&artifact).to_string(),
            evidence_type: "artifact_reviewed".to_string(),
            title: format!("{} export review", artifact.title),
            summary: Some(
                "Owner confirmed audience, deliverability, and redaction status".to_string(),
            ),
            source_metadata: json!({
                "artifactId": artifact.id,
                "version": artifact.current_version,
                "audience": audience.trim(),
                "exportReview": true,
                "exportReady": true,
                "redactionChecked": true,
            }),
            confidence: Some(1.0),
            access_scope: Some("session".to_string()),
            redaction_status: Some("none".to_string()),
        })?;
        self.evaluate_export_guard(&artifact)
    }

    pub fn evaluate_export_guard(
        &self,
        artifact: &ArtifactRecord,
    ) -> Result<DomainArtifactExportGuardReport> {
        let session_id = artifact
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("guarded export requires a session-scoped Artifact"))?;
        let db = crate::globals::get_session_db()
            .ok_or_else(|| anyhow!("session database is unavailable"))?;
        db.evaluate_domain_artifact_export_guard(DomainArtifactExportGuardInput {
            goal_id: artifact.goal_id.clone(),
            session_id: Some(session_id.to_string()),
            project_id: artifact.project_id.clone(),
            domain: Some(artifact_domain(artifact).to_string()),
            artifact_path: Some(artifact.project_path.clone()),
            artifact_title: Some(artifact.title.clone()),
            artifact_kind: Some(artifact.kind.clone()),
            ..Default::default()
        })
    }

    fn enforce_export_guard(&self, artifact: &ArtifactRecord) -> Result<()> {
        if !self.requires_export_guard(artifact)? {
            return Ok(());
        }
        let report = self.evaluate_export_guard(artifact)?;
        if report.status != "passed" {
            let blockers = if report.blockers.is_empty() {
                "Artifact Export Guard did not pass".to_string()
            } else {
                report.blockers.join("; ")
            };
            bail!("Artifact Export Guard blocked export: {blockers}");
        }
        if !self.current_version_has_export_review(artifact)? {
            bail!(
                "Artifact Export Guard blocked export: current Artifact version has no owner export review"
            );
        }
        Ok(())
    }

    fn current_version_has_export_review(&self, artifact: &ArtifactRecord) -> Result<bool> {
        let Some(session_id) = artifact.session_id.as_deref() else {
            return Ok(false);
        };
        let db = crate::globals::get_session_db()
            .ok_or_else(|| anyhow!("session database is unavailable"))?;
        let evidence = db.list_domain_evidence(ListDomainEvidenceInput {
            goal_id: artifact.goal_id.clone(),
            session_id: Some(session_id.to_string()),
            project_id: None,
            domain: Some(artifact_domain(artifact).to_string()),
            evidence_type: Some("artifact_reviewed".to_string()),
            limit: Some(200),
        })?;
        Ok(evidence.iter().any(|item| {
            item.source_metadata
                .get("artifactId")
                .and_then(Value::as_str)
                == Some(artifact.id.as_str())
                && item.source_metadata.get("version").and_then(Value::as_i64)
                    == Some(artifact.current_version)
                && item
                    .source_metadata
                    .get("exportReview")
                    .and_then(Value::as_bool)
                    == Some(true)
                && item
                    .source_metadata
                    .get("redactionChecked")
                    .and_then(Value::as_bool)
                    == Some(true)
        }))
    }

    fn requires_export_guard(&self, artifact: &ArtifactRecord) -> Result<bool> {
        if matches!(
            artifact.privacy.as_str(),
            "shareable_snapshot" | "sensitive"
        ) {
            return Ok(true);
        }
        let sources_json: String = self.conn.query_row(
            "SELECT COALESCE(sources_json, '[]') FROM artifact_records WHERE id = ?1",
            [artifact.id.as_str()],
            |row| row.get(0),
        )?;
        let sources = serde_json::from_str::<Vec<Value>>(&sources_json).unwrap_or_default();
        Ok(sources.iter().any(|source| {
            matches!(
                source
                    .get("accessScope")
                    .or_else(|| source.get("access_scope"))
                    .and_then(Value::as_str),
                Some("private" | "connector" | "sensitive")
            ) || source.get("redistributable").and_then(Value::as_bool) == Some(false)
        }))
    }

    pub fn export(&mut self, artifact_id: &str, format: &str) -> Result<ArtifactExportReceipt> {
        // Artifact and legacy Canvas mutations share this lock. Keep it for
        // the complete synchronous export so the verified version, managed
        // projection, package bytes, and receipt all describe one snapshot.
        let _artifact_guard = lock_privacy_transition()?;
        let artifact = self
            .get(artifact_id)?
            .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
        emit_artifact_event(
            "artifact:export_running",
            artifact_id,
            artifact.current_version,
            Some(format),
        );
        if let Err(error) = self.enforce_export_guard(&artifact) {
            emit_artifact_event(
                "artifact:export_failed",
                artifact_id,
                artifact.current_version,
                Some(&error.to_string()),
            );
            return Err(error);
        }
        let mut verification = self.verify(artifact_id)?;
        if verification.status != "passed" {
            emit_artifact_event(
                "artifact:export_failed",
                artifact_id,
                artifact.current_version,
                Some("artifact verification failed"),
            );
            bail!("artifact verification failed; fix blockers before export");
        }
        let export_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires = now + Duration::days(7);
        let version = artifact.current_version;
        let base_name = safe_filename(&artifact.title);
        let html_bytes = fs::read(paths::canvas_project_dir(artifact_id)?.join("index.html"))?;
        self.ensure_export_snapshot_current(&artifact)?;
        let (extension, mime, bytes) = match format {
            "html" => ("html", "text/html; charset=utf-8", html_bytes.clone()),
            "markdown" | "md" => (
                "md",
                "text/markdown; charset=utf-8",
                self.render_markdown_export(artifact_id, version)?
                    .into_bytes(),
            ),
            "zip" => (
                "zip",
                "application/zip",
                self.build_zip_export(&artifact, &verification, &html_bytes)?,
            ),
            "pdf" => {
                return self.persist_failed_export(
                    &export_id,
                    &artifact,
                    "pdf",
                    "application/pdf",
                    "PDF requires an available managed Chromium runtime; use HTML/ZIP or install the browser runtime.",
                    now,
                    expires,
                );
            }
            other => bail!("unsupported artifact export format '{other}'"),
        };
        if extension == "zip" {
            let zip_result = verify_zip_manifest(&bytes);
            let zip_detail = match &zip_result {
                Ok(()) => "ZIP members match manifest size and SHA-256 values.",
                Err(error) => error.as_str(),
            };
            verification
                .checks
                .push(check("zip_manifest", zip_result.is_ok(), zip_detail));
            if zip_result.is_err() {
                verification.status = "failed".to_string();
                bail!("ZIP verification failed before delivery");
            }
        }
        let filename = format!("{base_name}-v{version}.{extension}");
        let export_dir = paths::canvas_dir()?.join("exports");
        fs::create_dir_all(&export_dir)?;
        let output_path = export_dir.join(format!("{export_id}.{extension}"));
        crate::platform::write_atomic(&output_path, &bytes)?;
        let receipt = ArtifactExportReceipt {
            id: export_id,
            artifact_id: artifact_id.to_string(),
            version_number: version,
            format: extension.to_string(),
            status: "ready".to_string(),
            filename,
            mime_type: mime.to_string(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            verification: Some(verification),
            error: None,
            internal_path: Some(output_path.to_string_lossy().into_owned()),
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
        };
        self.insert_export(&receipt)?;
        emit_artifact_event(
            "artifact:export_ready",
            artifact_id,
            version,
            Some(&receipt.id),
        );
        Ok(receipt)
    }

    /// Export through the app-owned managed Chromium when PDF is requested.
    /// Other formats stay on the deterministic synchronous renderer.
    pub async fn export_async(
        &mut self,
        artifact_id: &str,
        format: &str,
    ) -> Result<ArtifactExportReceipt> {
        if format != "pdf" {
            return self.export(artifact_id, format);
        }

        let (artifact, artifact_verification, index_html) = {
            // Do not hold a std::sync mutex across the Chromium await. Take a
            // verified byte snapshot while all Artifact/Canvas writers are
            // excluded, then render that immutable copy after releasing it.
            let _artifact_guard = lock_privacy_transition()?;
            let artifact = self
                .get(artifact_id)?
                .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
            emit_artifact_event(
                "artifact:export_running",
                artifact_id,
                artifact.current_version,
                Some(format),
            );
            if let Err(error) = self.enforce_export_guard(&artifact) {
                emit_artifact_event(
                    "artifact:export_failed",
                    artifact_id,
                    artifact.current_version,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
            let artifact_verification = self.verify(artifact_id)?;
            if artifact_verification.status != "passed" {
                emit_artifact_event(
                    "artifact:export_failed",
                    artifact_id,
                    artifact.current_version,
                    Some("artifact verification failed"),
                );
                bail!("artifact verification failed; fix blockers before export");
            }
            let index_html = fs::read(paths::canvas_project_dir(artifact_id)?.join("index.html"))?;
            self.ensure_export_snapshot_current(&artifact)?;
            (artifact, artifact_verification, index_html)
        };

        let export_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires = now + Duration::days(7);
        let bytes = match render_pdf_with_isolated_chromium(&index_html, &export_id).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return self.persist_failed_export(
                    &export_id,
                    &artifact,
                    "pdf",
                    "application/pdf",
                    &format!(
                        "PDF runtime unavailable or Page.printToPDF failed: {error}. HTML/ZIP/Markdown remain available; install the Hope Chromium runtime from Browser settings."
                    ),
                    now,
                    expires,
                );
            }
        };

        let has_magic = bytes.starts_with(b"%PDF-");
        let page_count = count_pdf_pages(&bytes);
        let extracted = pdf_extract::extract_text_from_mem(&bytes).unwrap_or_default();
        let mut checks = artifact_verification.checks.clone();
        checks.push(check(
            "pdf_magic",
            has_magic,
            "The generated file has a valid PDF signature.",
        ));
        checks.push(check(
            "pdf_page_count",
            page_count > 0,
            &format!("Detected {page_count} printable page(s)."),
        ));
        checks.push(check(
            "pdf_text_extractable",
            !extracted.trim().is_empty(),
            "PDF text can be extracted for search and accessibility.",
        ));
        let pdf_verification = VerificationReport {
            status: if checks.iter().all(|check| check.status == "passed") {
                "passed"
            } else {
                "failed"
            }
            .to_string(),
            checks,
            verified_at: Utc::now().to_rfc3339(),
        };
        if pdf_verification.status != "passed" {
            return self.persist_failed_export_with_verification(
                &export_id,
                &artifact,
                "pdf",
                "application/pdf",
                "PDF was generated but failed signature, page-count, or text-extraction QA.",
                Some(pdf_verification),
                now,
                expires,
            );
        }

        let filename = format!(
            "{}-v{}.pdf",
            safe_filename(&artifact.title),
            artifact.current_version
        );
        let export_dir = paths::canvas_dir()?.join("exports");
        fs::create_dir_all(&export_dir)?;
        let output_path = export_dir.join(format!("{export_id}.pdf"));
        crate::platform::write_atomic(&output_path, &bytes)?;
        let receipt = ArtifactExportReceipt {
            id: export_id,
            artifact_id: artifact.id.clone(),
            version_number: artifact.current_version,
            format: "pdf".to_string(),
            status: "ready".to_string(),
            filename,
            mime_type: "application/pdf".to_string(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            verification: Some(pdf_verification),
            error: None,
            internal_path: Some(output_path.to_string_lossy().into_owned()),
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
        };
        self.insert_export(&receipt)?;
        emit_artifact_event(
            "artifact:export_ready",
            artifact_id,
            artifact.current_version,
            Some(&receipt.id),
        );
        Ok(receipt)
    }

    pub fn get_export(&self, export_id: &str) -> Result<Option<ArtifactExportReceipt>> {
        let receipt = self
            .conn
            .query_row(
                "SELECT id, artifact_id, version_number, format, status, filename,
                        mime_type, size_bytes, sha256, verification_json, error,
                        internal_path, created_at, expires_at
                   FROM artifact_exports WHERE id = ?1",
                [export_id],
                map_export_row,
            )
            .optional()
            .map_err(anyhow::Error::from)?;
        match receipt {
            Some(receipt) if self.get(&receipt.artifact_id)?.is_none() => Ok(None),
            other => Ok(other),
        }
    }

    pub fn archive(&self, artifact_id: &str) -> Result<()> {
        if self.get(artifact_id)?.is_none() {
            bail!("artifact '{}' not found", artifact_id);
        }
        let changed = self.conn.execute(
            "UPDATE artifact_records SET lifecycle_state = 'archived', updated_at = ?1
              WHERE id = ?2",
            params![Utc::now().to_rfc3339(), artifact_id],
        )?;
        if changed == 0 {
            bail!("artifact '{}' not found", artifact_id);
        }
        emit_artifact_event("artifact:archived", artifact_id, 0, None);
        Ok(())
    }

    pub fn delete(&self, artifact_id: &str) -> Result<()> {
        let stored_id: String = self
            .conn
            .query_row(
                "SELECT id FROM artifact_records WHERE id = ?1",
                [artifact_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("artifact '{}' not found", artifact_id))?;
        let project_dir = paths::canvas_project_dir(&stored_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT internal_path FROM artifact_exports
              WHERE artifact_id = ?1 AND internal_path IS NOT NULL",
        )?;
        let export_paths = stmt
            .query_map([artifact_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        self.conn
            .execute("DELETE FROM canvas_projects WHERE id = ?1", [&stored_id])?;
        if project_dir.exists() {
            fs::remove_dir_all(project_dir)?;
        }
        for path in export_paths {
            remove_managed_export_file(&path)?;
        }
        gc_unreferenced_blobs(&self.conn)?;
        emit_artifact_event("artifact:deleted", artifact_id, 0, None);
        Ok(())
    }

    pub fn has_for_session(&self, session_id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                      FROM artifact_records ar
                      JOIN canvas_projects cp ON cp.id = ar.id
                     WHERE cp.session_id = ?1
                )",
                [session_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Burn every durable Artifact linked to an incognito session. The caller
    /// invokes this from the session purge watcher as a privacy backstop.
    pub fn purge_for_session(&self, session_id: &str) -> Result<usize> {
        let _privacy_guard = lock_privacy_transition()?;
        let mut stmt = self.conn.prepare(
            "SELECT ar.id
               FROM artifact_records ar
               JOIN canvas_projects cp ON cp.id = ar.id
              WHERE cp.session_id = ?1",
        )?;
        let ids = stmt
            .query_map([session_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        let mut failures = Vec::new();
        let mut deleted = 0;
        for id in ids {
            match self.delete(&id) {
                Ok(()) => deleted += 1,
                Err(error) => failures.push(format!("{id}: {error}")),
            }
        }
        if !failures.is_empty() {
            bail!(
                "failed to purge {} Artifact(s) for session {}: {}",
                failures.len(),
                session_id,
                failures.join("; ")
            );
        }
        Ok(deleted)
    }

    /// Keep normal-session Artifacts in the Gallery after their source chat is
    /// deleted, while removing the now-invalid session association.
    pub fn detach_from_session(&self, session_id: &str) -> Result<usize> {
        let _privacy_guard = lock_privacy_transition()?;
        self.conn
            .execute(
                "UPDATE canvas_projects
                    SET session_id = NULL, updated_at = ?1
                  WHERE session_id = ?2",
                params![Utc::now().to_rfc3339(), session_id],
            )
            .map_err(Into::into)
    }

    fn render_markdown_export(&self, artifact_id: &str, version: i64) -> Result<String> {
        let row: (Option<String>, String, String) = self.conn.query_row(
            "SELECT cv.content, COALESCE(av.payload_kind, 'freeform'),
                    COALESCE(av.payload_json, '{}')
               FROM canvas_versions cv
               LEFT JOIN artifact_version_meta av
                 ON av.artifact_id = cv.project_id AND av.version_number = cv.version_number
              WHERE cv.project_id = ?1 AND cv.version_number = ?2",
            params![artifact_id, version],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if let Some(markdown) = row.0 {
            return Ok(markdown);
        }
        if row.1 == "analysis" {
            let analysis: AnalysisArtifactV1 = serde_json::from_str(&row.2)?;
            return Ok(render_analysis_markdown(&analysis));
        }
        bail!("this freeform artifact has no Markdown fallback")
    }

    fn build_zip_export(
        &self,
        artifact: &ArtifactRecord,
        verification: &VerificationReport,
        html_bytes: &[u8],
    ) -> Result<Vec<u8>> {
        let payload_json: String = self.conn.query_row(
            "SELECT payload_json FROM artifact_version_meta
              WHERE artifact_id = ?1 AND version_number = ?2",
            params![artifact.id, artifact.current_version],
            |row| row.get(0),
        )?;
        let markdown = self
            .render_markdown_export(&artifact.id, artifact.current_version)
            .ok();
        let verification_bytes = serde_json::to_vec_pretty(verification)?;
        let sources = self.sources_readme(artifact)?;
        let mut files = serde_json::Map::new();
        let mut describe = |path: &str, mime: &str, bytes: &[u8]| {
            files.insert(
                path.to_string(),
                json!({
                    "mimeType": mime,
                    "sha256": sha256_hex(bytes),
                    "sizeBytes": bytes.len()
                }),
            );
        };
        describe("index.html", "text/html; charset=utf-8", html_bytes);
        describe("artifact.json", "application/json", payload_json.as_bytes());
        describe("verification.json", "application/json", &verification_bytes);
        describe(
            "sources/README.md",
            "text/markdown; charset=utf-8",
            sources.as_bytes(),
        );
        if let Some(markdown) = markdown.as_deref() {
            describe(
                "report.md",
                "text/markdown; charset=utf-8",
                markdown.as_bytes(),
            );
        }
        let manifest = json!({
            "schemaVersion": ARTIFACT_SCHEMA_VERSION,
            "artifactId": artifact.id,
            "version": artifact.current_version,
            "title": artifact.title,
            "kind": artifact.kind,
            "privacy": artifact.privacy,
            "generatorVersion": EXPORT_GENERATOR_VERSION,
            "files": files
        });
        let mut cursor = Cursor::new(Vec::<u8>::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            add_zip_file(&mut writer, "index.html", html_bytes, options)?;
            add_zip_file(
                &mut writer,
                "artifact.json",
                payload_json.as_bytes(),
                options,
            )?;
            add_zip_file(
                &mut writer,
                "manifest.json",
                serde_json::to_string_pretty(&manifest)?.as_bytes(),
                options,
            )?;
            add_zip_file(
                &mut writer,
                "verification.json",
                &verification_bytes,
                options,
            )?;
            add_zip_file(
                &mut writer,
                "sources/README.md",
                sources.as_bytes(),
                options,
            )?;
            if let Some(markdown) = markdown {
                add_zip_file(&mut writer, "report.md", markdown.as_bytes(), options)?;
            }
            writer.finish()?;
        }
        Ok(cursor.into_inner())
    }

    fn ensure_export_snapshot_current(&self, snapshot: &ArtifactRecord) -> Result<()> {
        let current = self
            .get(&snapshot.id)?
            .ok_or_else(|| anyhow!("artifact '{}' disappeared during export", snapshot.id))?;
        if current.current_version != snapshot.current_version
            || current.current_hash != snapshot.current_hash
        {
            bail!(
                "artifact changed during export: snapshot version {} ({}), current version {} ({})",
                snapshot.current_version,
                snapshot.current_hash,
                current.current_version,
                current.current_hash
            );
        }
        Ok(())
    }

    fn sources_readme(&self, artifact: &ArtifactRecord) -> Result<String> {
        let sources_json: String = self.conn.query_row(
            "SELECT sources_json FROM artifact_records WHERE id = ?1",
            [&artifact.id],
            |row| row.get(0),
        )?;
        let sources: Vec<Value> = serde_json::from_str(&sources_json).unwrap_or_default();
        let mut out = format!("# Sources for {}\n\n", artifact.title);
        if sources.is_empty() {
            out.push_str("No shareable canonical sources were recorded.\n");
        } else {
            for (index, source) in sources.iter().enumerate() {
                let label = source
                    .get("label")
                    .or_else(|| source.get("title"))
                    .or_else(|| source.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("Source");
                out.push_str(&format!("{}. {}\n", index + 1, label));
            }
        }
        out.push_str(
            "\nSensitive source contents and chat attachments are not included by default.\n",
        );
        Ok(out)
    }

    fn persist_failed_export(
        &self,
        export_id: &str,
        artifact: &ArtifactRecord,
        format: &str,
        mime: &str,
        error: &str,
        created_at: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<ArtifactExportReceipt> {
        self.persist_failed_export_with_verification(
            export_id,
            artifact,
            format,
            mime,
            error,
            artifact.verification.clone(),
            created_at,
            expires_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_failed_export_with_verification(
        &self,
        export_id: &str,
        artifact: &ArtifactRecord,
        format: &str,
        mime: &str,
        error: &str,
        verification: Option<VerificationReport>,
        created_at: chrono::DateTime<Utc>,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<ArtifactExportReceipt> {
        let receipt = ArtifactExportReceipt {
            id: export_id.to_string(),
            artifact_id: artifact.id.clone(),
            version_number: artifact.current_version,
            format: format.to_string(),
            status: "failed".to_string(),
            filename: format!(
                "{}-v{}.{}",
                safe_filename(&artifact.title),
                artifact.current_version,
                format
            ),
            mime_type: mime.to_string(),
            size_bytes: 0,
            sha256: String::new(),
            verification,
            error: Some(error.to_string()),
            internal_path: None,
            created_at: created_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        };
        self.insert_export(&receipt)?;
        emit_artifact_event(
            "artifact:export_failed",
            &artifact.id,
            artifact.current_version,
            Some(error),
        );
        Ok(receipt)
    }

    fn insert_export(&self, receipt: &ArtifactExportReceipt) -> Result<()> {
        self.conn.execute(
            "INSERT INTO artifact_exports
             (id, artifact_id, version_number, format, status, filename, mime_type,
              size_bytes, sha256, verification_json, error, internal_path,
              created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                receipt.id,
                receipt.artifact_id,
                receipt.version_number,
                receipt.format,
                receipt.status,
                receipt.filename,
                receipt.mime_type,
                receipt.size_bytes as i64,
                receipt.sha256,
                receipt
                    .verification
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                receipt.error,
                receipt.internal_path,
                receipt.created_at,
                receipt.expires_at
            ],
        )?;
        Ok(())
    }
}

fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS artifact_records (
            id TEXT PRIMARY KEY REFERENCES canvas_projects(id) ON DELETE CASCADE,
            kind TEXT NOT NULL DEFAULT 'custom',
            project_id TEXT,
            goal_id TEXT,
            lifecycle_state TEXT NOT NULL DEFAULT 'active',
            privacy TEXT NOT NULL DEFAULT 'local_private',
            current_hash TEXT NOT NULL DEFAULT '',
            producer_json TEXT NOT NULL DEFAULT '{}',
            capabilities_json TEXT NOT NULL DEFAULT '{}',
            sources_json TEXT NOT NULL DEFAULT '[]',
            evidence_ids_json TEXT NOT NULL DEFAULT '[]',
            evidence_summary_json TEXT NOT NULL DEFAULT '{}',
            verification_json TEXT,
            payload_kind TEXT NOT NULL DEFAULT 'freeform',
            analysis_status TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_artifact_records_gallery
            ON artifact_records(lifecycle_state, kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS artifact_version_meta (
            artifact_id TEXT NOT NULL REFERENCES artifact_records(id) ON DELETE CASCADE,
            version_number INTEGER NOT NULL,
            parent_version INTEGER,
            payload_kind TEXT NOT NULL DEFAULT 'freeform',
            payload_json TEXT NOT NULL DEFAULT '{}',
            content_hash TEXT NOT NULL DEFAULT '',
            producer_json TEXT NOT NULL DEFAULT '{}',
            capabilities_json TEXT NOT NULL DEFAULT '{}',
            sources_json TEXT NOT NULL DEFAULT '[]',
            evidence_ids_json TEXT NOT NULL DEFAULT '[]',
            evidence_summary_json TEXT NOT NULL DEFAULT '{}',
            verification_json TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (artifact_id, version_number)
        );

        CREATE TABLE IF NOT EXISTS artifact_exports (
            id TEXT PRIMARY KEY,
            artifact_id TEXT NOT NULL REFERENCES artifact_records(id) ON DELETE CASCADE,
            version_number INTEGER NOT NULL,
            format TEXT NOT NULL,
            status TEXT NOT NULL,
            filename TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            size_bytes INTEGER NOT NULL DEFAULT 0,
            sha256 TEXT NOT NULL DEFAULT '',
            verification_json TEXT,
            error TEXT,
            internal_path TEXT,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_artifact_exports_artifact
            ON artifact_exports(artifact_id, created_at DESC);

        CREATE TABLE IF NOT EXISTS artifact_blobs (
            sha256 TEXT PRIMARY KEY,
            size_bytes INTEGER NOT NULL,
            mime_type TEXT NOT NULL,
            internal_path TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS artifact_version_blobs (
            artifact_id TEXT NOT NULL,
            version_number INTEGER NOT NULL,
            sha256 TEXT NOT NULL REFERENCES artifact_blobs(sha256) ON DELETE RESTRICT,
            logical_path TEXT NOT NULL,
            PRIMARY KEY (artifact_id, version_number, logical_path),
            FOREIGN KEY (artifact_id, version_number)
                REFERENCES artifact_version_meta(artifact_id, version_number) ON DELETE CASCADE
        );",
    )?;
    // Existing Canvas databases receive additive columns without moving the
    // store or rewriting historical rows.
    let _ = conn.execute(
        "ALTER TABLE artifact_records ADD COLUMN evidence_summary_json TEXT NOT NULL DEFAULT '{}'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE artifact_version_meta ADD COLUMN evidence_summary_json TEXT NOT NULL DEFAULT '{}'",
        [],
    );
    Ok(())
}

struct RecordedEvidence {
    ids: Vec<String>,
    summary: Value,
}

#[allow(clippy::too_many_arguments)]
fn record_artifact_evidence(
    goal_id: Option<&str>,
    session_id: Option<&str>,
    project_id: Option<&str>,
    artifact_id: &str,
    title: &str,
    kind: &str,
    privacy: &str,
    project_dir: &Path,
    prepared: &PreparedPayload,
    version: i64,
) -> RecordedEvidence {
    let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
        return RecordedEvidence {
            ids: Vec::new(),
            summary: json!({}),
        };
    };
    let Some(db) = crate::globals::get_session_db() else {
        return RecordedEvidence {
            ids: Vec::new(),
            summary: json!({}),
        };
    };
    let analysis = (prepared.payload_kind == "analysis")
        .then(|| serde_json::from_str::<AnalysisArtifactV1>(&prepared.payload_json).ok())
        .flatten();
    let mut ids = Vec::new();
    let mut counts = serde_json::Map::new();
    let mut record = |evidence_type: &str,
                      evidence_title: String,
                      summary: Option<String>,
                      metadata: Value,
                      confidence: Option<f64>,
                      access_scope: &str,
                      redaction_status: &str| {
        let result = db.record_domain_evidence(RecordDomainEvidenceInput {
            goal_id: goal_id.map(ToOwned::to_owned),
            session_id: Some(session_id.to_string()),
            project_id: project_id.map(ToOwned::to_owned),
            domain: if prepared.payload_kind == "analysis" {
                "data_analysis".to_string()
            } else {
                "artifact".to_string()
            },
            evidence_type: evidence_type.to_string(),
            title: evidence_title,
            summary,
            source_metadata: metadata,
            confidence,
            access_scope: Some(access_scope.to_string()),
            redaction_status: Some(redaction_status.to_string()),
        });
        match result {
            Ok(item) => {
                ids.push(item.id);
                let count = counts
                    .get(evidence_type)
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    + 1;
                counts.insert(evidence_type.to_string(), json!(count));
            }
            Err(error) => app_warn!(
                "artifact",
                "evidence",
                "Failed to record {} evidence for {}: {}",
                evidence_type,
                artifact_id,
                error
            ),
        }
    };

    if let Some(analysis) = analysis.as_ref() {
        for source in &analysis.sources {
            let label = source
                .get("label")
                .or_else(|| source.get("title"))
                .or_else(|| source.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("Analysis source");
            let access_scope = match source
                .get("accessScope")
                .or_else(|| source.get("access_scope"))
                .and_then(Value::as_str)
            {
                Some("private") => "private",
                Some("connector") => "connector",
                Some("sensitive") => "private",
                Some("public") => "public",
                _ => "session",
            };
            let redaction_status = source
                .get("redactionStatus")
                .or_else(|| source.get("redaction_status"))
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "none" | "pending" | "sensitive" | "redacted"))
                .unwrap_or("none");
            let mut metadata = source.clone();
            if let Some(object) = metadata.as_object_mut() {
                object.insert("artifactId".to_string(), json!(artifact_id));
                object.insert("version".to_string(), json!(version));
            }
            record(
                "source_cited",
                label.to_string(),
                Some("Canonical source registered in AnalysisArtifactV1".to_string()),
                metadata,
                None,
                access_scope,
                redaction_status,
            );
        }
        if !analysis.data_quality.is_empty() {
            let datasets = analysis
                .data_quality
                .iter()
                .filter_map(|check| {
                    check
                        .get("datasetId")
                        .or_else(|| check.get("dataset_id"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>();
            record(
                "data_quality_checked",
                "Artifact data-quality checks".to_string(),
                Some(format!(
                    "{} deterministic data-quality check result(s) recorded",
                    analysis.data_quality.len()
                )),
                json!({ "artifactId": artifact_id, "version": version, "dataset": datasets.join(","), "checks": analysis.data_quality }),
                Some(1.0),
                "session",
                "none",
            );
        }
        for claim in &analysis.claim_validation {
            let claim_title = claim
                .get("claim")
                .or_else(|| claim.get("metric"))
                .or_else(|| claim.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("Validated analysis claim");
            record(
                "claim_checked",
                claim_title.to_string(),
                Some("Metric interpretation recorded in AnalysisArtifactV1".to_string()),
                json!({
                    "artifactId": artifact_id,
                    "version": version,
                    "claim": claim_title,
                    "metric": claim.get("metric").and_then(Value::as_str),
                    "denominator": claim.get("denominator").and_then(Value::as_str),
                    "verdict": claim.get("verdict").and_then(Value::as_str),
                    "method": claim.get("method").and_then(Value::as_str),
                    "sourceIds": claim.get("sourceIds").or_else(|| claim.get("source_ids")),
                }),
                claim.get("confidence").and_then(Value::as_f64),
                "session",
                "none",
            );
        }
    }
    record(
        "artifact_created",
        title.to_string(),
        Some(format!("Artifact persisted as immutable version {version}")),
        json!({
            "artifact": artifact_id,
            "artifactId": artifact_id,
            "path": project_dir.join("index.html"),
            "version": version,
            "kind": kind,
            "privacy": privacy,
        }),
        Some(1.0),
        if privacy == "sensitive" {
            "private"
        } else {
            "session"
        },
        "none",
    );

    RecordedEvidence {
        ids,
        summary: Value::Object(counts),
    }
}

fn artifact_domain(artifact: &ArtifactRecord) -> &'static str {
    if artifact.payload_kind == "analysis" {
        "data_analysis"
    } else {
        "artifact"
    }
}

fn request_is_incognito(explicit: bool, session_id: Option<&str>) -> Result<bool> {
    if explicit {
        return Ok(true);
    }
    let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(false);
    };
    let db = crate::globals::get_session_db().ok_or_else(|| {
        anyhow!("cannot verify session privacy while the session database is unavailable")
    })?;
    match db.get_session(session_id) {
        Ok(Some(session)) => Ok(session.incognito),
        Ok(None) => Ok(true),
        Err(error) => Err(anyhow!(
            "cannot verify session privacy for '{}': {}",
            session_id,
            error
        )),
    }
}

fn sync_legacy_canvas_records(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO artifact_records
         (id, kind, lifecycle_state, privacy, current_hash, producer_json,
          capabilities_json, sources_json, evidence_ids_json, payload_kind,
          created_at, updated_at)
         SELECT id, 'custom', 'active', 'local_private', '',
                '{\"type\":\"legacy_canvas\"}', '{}', '[]', '[]', 'freeform',
                created_at, updated_at
           FROM canvas_projects",
        [],
    )?;
    Ok(())
}

fn sync_one_legacy_canvas_record(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO artifact_records
         (id, kind, lifecycle_state, privacy, current_hash, producer_json,
          capabilities_json, sources_json, evidence_ids_json, payload_kind,
          created_at, updated_at)
         SELECT id, 'custom', 'active', 'local_private', '',
                '{\"type\":\"legacy_canvas\"}', '{}', '[]', '[]', 'freeform',
                created_at, updated_at
           FROM canvas_projects WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

fn backfill_legacy_current_version(conn: &Connection, id: &str) -> Result<()> {
    let state: Option<(String, i64, bool)> = conn
        .query_row(
            "SELECT ar.current_hash, cp.version_count,
                    EXISTS(
                        SELECT 1 FROM artifact_version_meta av
                         WHERE av.artifact_id = ar.id
                           AND av.version_number = cp.version_count
                    )
               FROM artifact_records ar
               JOIN canvas_projects cp ON cp.id = ar.id
              WHERE ar.id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((current_hash, version, has_current_meta)) = state else {
        return Ok(());
    };
    if !current_hash.is_empty() && has_current_meta {
        return Ok(());
    }
    let snapshot: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = conn
        .query_row(
            "SELECT html, css, js, content, created_at
               FROM canvas_versions
              WHERE project_id = ?1 AND version_number = ?2",
            params![id, version],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let Some((html, css, js, content, created_at)) = snapshot else {
        return Ok(());
    };
    let payload = json!({
        "sourceFormat": "legacy_canvas",
        "html": html,
        "css": css,
        "js": js,
        "content": content,
    });
    let canonical = serde_json::to_vec(&payload)?;
    let hash = sha256_hex(&canonical);
    let capabilities = json!({
        "network": false,
        "scripts": payload.get("js").is_some_and(|value| !value.is_null()),
        "attachments": false,
        "legacy": true,
        "schemaVersion": ARTIFACT_SCHEMA_VERSION
    });
    conn.execute(
        "UPDATE artifact_records
            SET current_hash = ?1, capabilities_json = ?2
          WHERE id = ?3",
        params![hash, capabilities.to_string(), id],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO artifact_version_meta
         (artifact_id, version_number, parent_version, payload_kind, payload_json,
          content_hash, producer_json, capabilities_json, sources_json,
          evidence_ids_json, verification_json, created_at)
         VALUES (?1, ?2, ?3, 'freeform', ?4, ?5, '{\"type\":\"legacy_canvas\"}',
                 ?6, '[]', '[]', NULL, ?7)",
        params![
            id,
            version,
            (version > 1).then_some(version - 1),
            payload.to_string(),
            hash,
            capabilities.to_string(),
            created_at
        ],
    )?;
    Ok(())
}

fn map_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    let sources_json: String = row.get(14)?;
    let capabilities_json: String = row.get(15)?;
    let evidence_summary_json: String = row.get(16)?;
    let verification_json: Option<String> = row.get(17)?;
    let sources = serde_json::from_str::<Vec<Value>>(&sources_json).unwrap_or_default();
    let id: String = row.get(0)?;
    Ok(ArtifactRecord {
        project_path: paths::canvas_project_dir(&id)
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        id,
        title: row.get(1)?,
        kind: row.get(2)?,
        content_type: row.get(3)?,
        session_id: row.get(4)?,
        project_id: row.get(5)?,
        agent_id: row.get(6)?,
        goal_id: row.get(7)?,
        lifecycle_state: row.get(8)?,
        privacy: row.get(9)?,
        current_version: row.get(10)?,
        current_hash: row.get(11)?,
        payload_kind: row.get(12)?,
        analysis_status: row.get(13)?,
        source_count: sources.len(),
        source_summaries: sources.iter().map(source_summary).collect(),
        evidence_summary: serde_json::from_str(&evidence_summary_json)
            .unwrap_or_else(|_| json!({})),
        capabilities: serde_json::from_str(&capabilities_json).unwrap_or_else(|_| json!({})),
        verification: verification_json.and_then(|v| serde_json::from_str(&v).ok()),
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

fn source_summary(source: &Value) -> ArtifactSourceSummary {
    ArtifactSourceSummary {
        id: source
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        label: source
            .get("label")
            .or_else(|| source.get("title"))
            .or_else(|| source.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("Source")
            .to_string(),
        source_type: source
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        sha256: source
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        access_scope: source
            .get("accessScope")
            .or_else(|| source.get("access_scope"))
            .and_then(Value::as_str)
            .unwrap_or("unspecified")
            .to_string(),
    }
}

fn map_export_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactExportReceipt> {
    let verification_json: Option<String> = row.get(9)?;
    Ok(ArtifactExportReceipt {
        id: row.get(0)?,
        artifact_id: row.get(1)?,
        version_number: row.get(2)?,
        format: row.get(3)?,
        status: row.get(4)?,
        filename: row.get(5)?,
        mime_type: row.get(6)?,
        size_bytes: row.get::<_, i64>(7)?.max(0) as u64,
        sha256: row.get(8)?,
        verification: verification_json.and_then(|v| serde_json::from_str(&v).ok()),
        error: row.get(10)?,
        internal_path: row.get(11)?,
        created_at: row.get(12)?,
        expires_at: row.get(13)?,
    })
}

fn validate_source_path(path: &Path, roots: Option<&[PathBuf]>) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("artifact source does not exist: {}", path.display()))?;
    if let Some(roots) = roots {
        let allowed = roots
            .iter()
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| canonical == root || canonical.starts_with(&root));
        if !allowed {
            bail!("artifact source must be inside the active workspace or agent home");
        }
    }
    let extension = canonical
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "html" | "htm" | "md" | "json") {
        bail!("artifact source must be .html, .htm, .md, or artifact .json");
    }
    Ok(canonical)
}

fn prepare_payload(path: &Path) -> Result<PreparedPayload> {
    let bytes = fs::read(path)?;
    if bytes.len() > 25 * 1024 * 1024 {
        bail!("artifact source exceeds the 25 MiB import limit");
    }
    let source = String::from_utf8(bytes.clone()).context("artifact source must be UTF-8 text")?;
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => {
            let analysis: AnalysisArtifactV1 =
                serde_json::from_str(&source).context("parsing AnalysisArtifactV1")?;
            analysis.validate()?;
            let canonical = serde_json::to_vec(&analysis)?;
            let markdown = render_analysis_markdown(&analysis);
            let index_html = render_analysis_html(&analysis);
            if contains_external_navigation(&index_html) {
                bail!("AnalysisArtifactV1 may not contain external navigation links");
            }
            Ok(PreparedPayload {
                payload_kind: "analysis".to_string(),
                analysis_status: Some(analysis.status.clone()),
                source_json: serde_json::to_string(&analysis.sources)?,
                payload_json: String::from_utf8(canonical.clone())?,
                canonical_bytes: canonical,
                index_html,
                markdown: Some(markdown),
                suggested_title: Some(analysis.question.clone()),
                capabilities: default_capabilities(),
            })
        }
        "md" => {
            let body = markdown_to_html(&source);
            if contains_external_navigation(&body) {
                bail!("Markdown may not contain external navigation links");
            }
            Ok(PreparedPayload {
                payload_kind: "freeform".to_string(),
                analysis_status: None,
                source_json: "[]".to_string(),
                payload_json: json!({"sourceFormat":"markdown","source":source}).to_string(),
                canonical_bytes: bytes,
                index_html: wrap_offline_document("Artifact", &body),
                markdown: Some(source),
                suggested_title: None,
                capabilities: default_capabilities(),
            })
        }
        _ => {
            let lower = source.to_ascii_lowercase();
            if ["<iframe", "<object", "<embed", "<form"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                bail!("freeform HTML may not contain iframe, object, embed, or form elements");
            }
            if contains_external_navigation(&source) {
                bail!("freeform HTML may not navigate or redirect to external documents");
            }
            let has_scripts = lower.contains("<script");
            Ok(PreparedPayload {
                payload_kind: "freeform".to_string(),
                analysis_status: None,
                source_json: "[]".to_string(),
                payload_json: json!({"sourceFormat":"html","source":source}).to_string(),
                canonical_bytes: bytes,
                index_html: normalize_imported_html(&source),
                markdown: None,
                suggested_title: extract_html_title(&source),
                capabilities: json!({
                    "network": false,
                    "scripts": has_scripts,
                    "attachments": false,
                    "executableContent": has_scripts,
                    "schemaVersion": ARTIFACT_SCHEMA_VERSION
                }),
            })
        }
    }
}

fn write_payload_files(project_dir: &Path, prepared: &PreparedPayload) -> Result<()> {
    fs::create_dir_all(project_dir)?;
    crate::platform::write_atomic(
        &project_dir.join("index.html"),
        prepared.index_html.as_bytes(),
    )?;
    crate::platform::write_atomic(
        &project_dir.join("artifact.json"),
        prepared.payload_json.as_bytes(),
    )?;
    if let Some(markdown) = prepared.markdown.as_deref() {
        crate::platform::write_atomic(&project_dir.join("content.md"), markdown.as_bytes())?;
    } else {
        let markdown_path = project_dir.join("content.md");
        if markdown_path.exists() {
            fs::remove_file(markdown_path)?;
        }
    }
    Ok(())
}

fn register_payload_blob(
    tx: &rusqlite::Transaction<'_>,
    artifact_id: &str,
    version: i64,
    prepared: &PreparedPayload,
) -> Result<PendingPayloadBlob> {
    let sha256 = sha256_hex(&prepared.canonical_bytes);
    let blob_dir = paths::canvas_dir()?.join("blobs").join(&sha256[..2]);
    let blob_path = blob_dir.join(&sha256);
    let (logical_path, mime_type) = match prepared.payload_kind.as_str() {
        "analysis" => ("artifact.json", "application/json"),
        _ if prepared.markdown.is_some() => ("content.md", "text/markdown; charset=utf-8"),
        _ => ("source.html", "text/html; charset=utf-8"),
    };
    tx.execute(
        "INSERT OR IGNORE INTO artifact_blobs
         (sha256, size_bytes, mime_type, internal_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            sha256,
            prepared.canonical_bytes.len() as i64,
            mime_type,
            blob_path.to_string_lossy(),
            Utc::now().to_rfc3339(),
        ],
    )?;
    tx.execute(
        "INSERT INTO artifact_version_blobs
         (artifact_id, version_number, sha256, logical_path)
         VALUES (?1, ?2, ?3, ?4)",
        params![artifact_id, version, sha256, logical_path],
    )?;
    Ok(PendingPayloadBlob {
        path: blob_path,
        bytes: prepared.canonical_bytes.clone(),
    })
}

fn remove_blob_if_unreferenced(conn: &Connection, pending: &PendingPayloadBlob, created: bool) {
    if !created {
        return;
    }
    let sha256 = sha256_hex(&pending.bytes);
    let referenced = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM artifact_version_blobs WHERE sha256 = ?1)",
            [&sha256],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(true);
    if !referenced {
        let _ = fs::remove_file(&pending.path);
    }
}

fn gc_unreferenced_blobs(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT b.sha256, b.internal_path
           FROM artifact_blobs b
          WHERE NOT EXISTS (
                SELECT 1 FROM artifact_version_blobs vb WHERE vb.sha256 = b.sha256
          )",
    )?;
    let candidates = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (sha256, internal_path) in candidates {
        let path = PathBuf::from(internal_path);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        conn.execute("DELETE FROM artifact_blobs WHERE sha256 = ?1", [sha256])?;
    }
    Ok(())
}

fn cleanup_expired_exports(conn: &Connection) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id, internal_path FROM artifact_exports
          WHERE expires_at < ?1",
    )?;
    let expired = stmt
        .query_map([now], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (id, path) in expired {
        if let Some(path) = path {
            remove_managed_export_file(&path)?;
        }
        conn.execute("DELETE FROM artifact_exports WHERE id = ?1", [id])?;
    }
    Ok(())
}

fn remove_managed_export_file(raw: &str) -> Result<()> {
    let path = PathBuf::from(raw);
    if !path.exists() {
        return Ok(());
    }
    let export_dir = paths::canvas_dir()?.join("exports");
    let canonical_dir = export_dir.canonicalize().unwrap_or(export_dir);
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(&canonical_dir) {
        bail!("refusing to remove export outside managed directory");
    }
    fs::remove_file(canonical)?;
    Ok(())
}

fn render_analysis_html(analysis: &AnalysisArtifactV1) -> String {
    analysis_renderer::render(analysis)
}

fn render_analysis_markdown(analysis: &AnalysisArtifactV1) -> String {
    let mut out = format!("# {}\n\n", analysis.question.trim());
    if !analysis.audience.trim().is_empty() {
        out.push_str(&format!("**Audience:** {}\n\n", analysis.audience.trim()));
    }
    if !analysis.decision.trim().is_empty() {
        out.push_str(&format!("**Decision:** {}\n\n", analysis.decision.trim()));
    }
    out.push_str(&format!("**Status:** {}\n\n", analysis.status));
    if let Some(time_range) = analysis.time_range.as_ref() {
        out.push_str(&format!("**Time range:** {}\n\n", compact_json(time_range)));
    }
    if let Some(grain) = analysis
        .grain
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        out.push_str(&format!("**Grain:** {}\n\n", grain.trim()));
    }
    for block in &analysis.blocks {
        if let Some(title) = block.get("title").and_then(Value::as_str) {
            out.push_str(&format!("## {}\n\n", title.trim()));
        }
        if let Some(body) = block
            .get("body")
            .or_else(|| block.get("markdown"))
            .and_then(Value::as_str)
        {
            out.push_str(body);
            out.push_str("\n\n");
        }
    }
    append_value_list(&mut out, "Findings", &analysis.findings);
    append_value_list(&mut out, "Recommendations", &analysis.recommendations);
    append_value_list(&mut out, "Caveats", &analysis.caveats);
    if !analysis.metric_definitions.is_empty() {
        out.push_str("## Metric definitions\n\n");
        out.push_str("| Metric | Formula | Unit | Window |\n| --- | --- | --- | --- |\n");
        for metric in &analysis.metric_definitions {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                markdown_cell(value_label(metric)),
                markdown_cell(metric.get("formula").and_then(Value::as_str).unwrap_or("—")),
                markdown_cell(metric.get("unit").and_then(Value::as_str).unwrap_or("—")),
                markdown_cell(metric.get("window").and_then(Value::as_str).unwrap_or("—")),
            ));
        }
        out.push('\n');
    }
    if !analysis.charts.is_empty() {
        out.push_str("## Visuals\n\n");
        for chart in &analysis.charts {
            let title = chart
                .get("title")
                .or_else(|| chart.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("Chart");
            out.push_str(&format!("### {}\n\n", title));
            let chart_type = chart.get("type").and_then(Value::as_str).unwrap_or("chart");
            let dataset_id = chart
                .get("dataset")
                .or_else(|| chart.get("datasetId"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            out.push_str(&format!(
                "> {} visualization backed by dataset `{}`. The table/text below is the no-script fallback.\n\n",
                chart_type, dataset_id
            ));
            if let Some(fallback_id) = chart.get("fallbackId").and_then(Value::as_str) {
                if let Some(fallback) = analysis.static_fallbacks.iter().find(|fallback| {
                    fallback.get("id").and_then(Value::as_str) == Some(fallback_id)
                }) {
                    if let Some(text) = fallback
                        .get("text")
                        .or_else(|| fallback.get("description"))
                        .and_then(Value::as_str)
                    {
                        out.push_str(text);
                        out.push_str("\n\n");
                    }
                }
            }
            if let Some(dataset) = analysis
                .datasets
                .iter()
                .find(|dataset| dataset.get("id").and_then(Value::as_str) == Some(dataset_id))
            {
                append_dataset_table(&mut out, dataset);
            }
        }
    }
    if !analysis.tables.is_empty() {
        out.push_str("## Data tables\n\n");
        for table in &analysis.tables {
            if let Some(title) = table.get("title").and_then(Value::as_str) {
                out.push_str(&format!("### {}\n\n", title));
            }
            let dataset_id = table
                .get("datasetId")
                .or_else(|| table.get("dataset"))
                .and_then(Value::as_str);
            if let Some(dataset) = dataset_id.and_then(|id| {
                analysis
                    .datasets
                    .iter()
                    .find(|dataset| dataset.get("id").and_then(Value::as_str) == Some(id))
            }) {
                append_dataset_table(&mut out, dataset);
            }
        }
    }
    append_quality_results(&mut out, &analysis.data_quality);
    append_value_list(&mut out, "Claim validation", &analysis.claim_validation);
    if !analysis.sources.is_empty() {
        out.push_str("## Sources\n\n");
        for (index, source) in analysis.sources.iter().enumerate() {
            let label = source
                .get("label")
                .or_else(|| source.get("title"))
                .or_else(|| source.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("Source");
            let hash = source
                .get("sha256")
                .and_then(Value::as_str)
                .unwrap_or("unhashed");
            let scope = source
                .get("accessScope")
                .or_else(|| source.get("access_scope"))
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            out.push_str(&format!(
                "{}. {} — `{}` — {}\n",
                index + 1,
                label,
                hash,
                scope
            ));
        }
        out.push('\n');
    }
    out
}

fn append_dataset_table(out: &mut String, dataset: &Value) {
    let columns = dataset
        .get("columns")
        .and_then(Value::as_array)
        .map(|columns| columns.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let rows = dataset
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if columns.is_empty() {
        out.push_str("No display columns were provided.\n\n");
        return;
    }
    out.push('|');
    for column in &columns {
        out.push_str(&format!(" {} |", markdown_cell(column)));
    }
    out.push('\n');
    out.push('|');
    for _ in &columns {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in &rows {
        out.push('|');
        for column in &columns {
            let value = row
                .get(*column)
                .map(compact_json)
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(" {} |", markdown_cell(&value)));
        }
        out.push('\n');
    }
    let row_count = dataset
        .get("rowCount")
        .and_then(Value::as_u64)
        .unwrap_or(rows.len() as u64);
    let truncated = dataset
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out.push_str(&format!(
        "\n_{} total row(s); {} embedded{}._\n\n",
        row_count,
        rows.len(),
        if truncated { "; truncated" } else { "" }
    ));
}

fn append_quality_results(out: &mut String, checks: &[Value]) {
    if checks.is_empty() {
        return;
    }
    out.push_str("## Data quality\n\n");
    out.push_str("| Check | Status | Observed | Blocking |\n| --- | --- | --- | --- |\n");
    for check in checks {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            markdown_cell(
                check
                    .get("check")
                    .and_then(Value::as_str)
                    .unwrap_or("check")
            ),
            markdown_cell(
                check
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ),
            markdown_cell(
                &check
                    .get("observed")
                    .map(compact_json)
                    .unwrap_or_else(|| "—".to_string())
            ),
            if check
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "yes"
            } else {
                "no"
            },
        ));
    }
    out.push('\n');
}

fn value_label(value: &Value) -> &str {
    value
        .get("label")
        .or_else(|| value.get("title"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("Item")
}

fn compact_json(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "—".to_string(),
        other => other.to_string(),
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn append_value_list(out: &mut String, title: &str, values: &[Value]) {
    if values.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}\n\n"));
    for value in values {
        let text = value
            .as_str()
            .or_else(|| value.get("summary").and_then(Value::as_str))
            .or_else(|| value.get("text").and_then(Value::as_str))
            .or_else(|| value.get("claim").and_then(Value::as_str));
        match text {
            Some(text) => out.push_str(&format!("- {text}\n")),
            None => out.push_str(&format!("- {}\n", value)),
        }
    }
    out.push('\n');
}

fn markdown_to_html(markdown: &str) -> String {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES,
    );
    let parser = parser.map(|event| match event {
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
        other => other,
    });
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

fn wrap_offline_document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
         <title>{}</title><style>{}</style></head><body><main>{}</main></body></html>",
        OFFLINE_CSP_STATIC,
        escape_html(title),
        OFFLINE_CSS,
        body
    )
}

const OFFLINE_CSP_STATIC: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
const OFFLINE_CSP_FREEFORM: &str = "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; font-src data:; script-src 'unsafe-inline'; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";

fn normalize_imported_html(source: &str) -> String {
    let policy = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
        OFFLINE_CSP_FREEFORM
    );
    if let Some(html) = insert_after_opening_tag(source, "head", &policy) {
        return html;
    }
    let head = format!(
        "<head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">{policy}</head>"
    );
    if let Some(html) = insert_after_opening_tag(source, "html", &head) {
        return html;
    }
    format!("<!DOCTYPE html><html lang=\"en\">{head}<body>{source}</body></html>")
}

fn insert_after_opening_tag(source: &str, tag: &str, insertion: &str) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    let start = lower.find(&format!("<{tag}"))?;
    let end = lower[start..].find('>')? + start + 1;
    let mut output = String::with_capacity(source.len() + insertion.len());
    output.push_str(&source[..end]);
    output.push_str(insertion);
    output.push_str(&source[end..]);
    Some(output)
}

const OFFLINE_CSS: &str = r#"
:root{color-scheme:light dark;font-family:Inter,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}
*{box-sizing:border-box}html,body{max-width:100%;overflow-x:hidden}
body{margin:0;background:#f5f5f7;color:#1d1d1f;line-height:1.6}
main{width:100%;max-width:960px;min-width:0;margin:0 auto;padding:48px 28px;background:#fff;min-height:100vh}
h1{font-size:2.25rem;line-height:1.15}h2{margin-top:2.2rem;border-top:1px solid #e5e5e7;padding-top:1rem}
table{display:block;max-width:100%;overflow-x:auto;border-collapse:collapse;width:100%}th,td{border:1px solid #ddd;padding:.55rem;text-align:left;overflow-wrap:anywhere}
pre{overflow:auto;background:#f3f3f5;padding:1rem;border-radius:.6rem}code{font-family:ui-monospace,SFMono-Regular,monospace}
blockquote{border-left:4px solid #999;margin-left:0;padding-left:1rem;color:#555}
img,svg{max-width:100%;height:auto}@media(max-width:640px){main{padding:24px 16px}h1{font-size:1.7rem}}
@media(prefers-color-scheme:dark){body{background:#111;color:#eee}main{background:#18181b}h2{border-color:#333}th,td{border-color:#444}pre{background:#27272a}blockquote{color:#bbb}}
@media print{body,main{background:#fff;color:#000}main{max-width:none;padding:0}a{color:#000;text-decoration:none}}
"#;

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    let title = html[start..end].trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn default_capabilities() -> Value {
    json!({
        "network": false,
        "scripts": false,
        "attachments": false,
        "schemaVersion": ARTIFACT_SCHEMA_VERSION
    })
}

fn normalize_privacy(value: &str) -> &str {
    match value {
        "shareable_snapshot" | "sensitive" | "incognito" => value,
        _ => "local_private",
    }
}

fn normalize_durable_privacy(value: &str) -> Result<&str> {
    let privacy = normalize_privacy(value);
    if privacy == "incognito" {
        bail!("incognito artifacts are memory-only; durable artifact creation is disabled");
    }
    Ok(privacy)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

async fn render_pdf_with_isolated_chromium(index_html: &[u8], export_id: &str) -> Result<Vec<u8>> {
    use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;

    let resolved =
        crate::browser::profile::resolve_profile(crate::browser::profile::BUILTIN_MANAGED)?;
    let executable = crate::browser::spawn::resolve_chrome_executable_for(
        resolved.executable.as_deref(),
        "artifact_pdf",
    )?;
    let runtime_root = paths::canvas_dir()?.join("pdf-runtime");
    fs::create_dir_all(&runtime_root)?;
    let user_data_dir = runtime_root.join(export_id);
    let port = crate::browser::spawn::pick_managed_port().await?;
    fs::create_dir_all(&user_data_dir)?;
    let index_path = user_data_dir.join("artifact.html");
    if let Err(error) = crate::platform::write_atomic(&index_path, index_html) {
        let _ = fs::remove_dir_all(&user_data_dir);
        return Err(error.into());
    }
    let extra_args = resolved
        .extra_args
        .iter()
        .filter(|arg| {
            !arg.starts_with("--user-data-dir")
                && !arg.starts_with("--remote-debugging-port")
                && !arg.starts_with("--headless")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut state = crate::browser_state::BrowserState::new();
    let render_result = async {
        let spec = crate::browser::spawn::LaunchSpec {
            profile: "artifact_pdf",
            executable: Some(executable.as_str()),
            user_data_dir: &user_data_dir,
            port,
            headless: true,
            extra_args: &extra_args,
        };
        state
            .spawn_chrome_and_connect(spec)
            .await
            .context("launching isolated Chromium for Artifact PDF")?;
        let browser = state
            .browser
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("isolated Chromium connected without a browser handle"))?;
        let file_url = url::Url::from_file_path(&index_path)
            .map_err(|_| anyhow!("cannot convert Artifact path to file URL"))?;
        let page = browser
            .new_page(file_url.as_str())
            .await
            .context("opening Artifact in isolated Chromium")?;
        for _ in 0..30 {
            let ready = page
                .evaluate("document.readyState")
                .await
                .ok()
                .and_then(|value| value.into_value::<String>().ok())
                .is_some_and(|value| value == "complete");
            if ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let params = PrintToPdfParams {
            paper_width: Some(8.27),
            paper_height: Some(11.69),
            landscape: Some(false),
            print_background: Some(true),
            ..Default::default()
        };
        let bytes = page
            .pdf(params)
            .await
            .context("printing Artifact with Page.printToPDF")?;
        let _ = page.close().await;
        Ok(bytes)
    }
    .await;
    state.disconnect().await;
    if let Err(error) = fs::remove_dir_all(&user_data_dir) {
        if user_data_dir.exists() {
            app_warn!(
                "artifact",
                "pdf_cleanup",
                "failed to remove isolated PDF browser profile {}: {}",
                user_data_dir.display(),
                error
            );
        }
    }
    render_result
}

fn count_pdf_pages(bytes: &[u8]) -> usize {
    bytes
        .windows(b"/Type /Page".len())
        .enumerate()
        .filter(|(index, window)| {
            *window == b"/Type /Page"
                && bytes.get(index + b"/Type /Page".len()).copied() != Some(b's')
        })
        .count()
}

fn contains_remote_dependency(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("src=\"http://")
        || lower.contains("src=\"https://")
        || lower.contains("src='http://")
        || lower.contains("src='https://")
        || lower.contains("src=http://")
        || lower.contains("src=https://")
        || lower.contains("url(http://")
        || lower.contains("url(https://")
        || lower.contains("@import")
        || lower.contains("fetch(")
        || lower.contains("xmlhttprequest")
        || lower.contains("websocket(")
        || lower.contains("eventsource(")
        || lower.contains("sendbeacon(")
}

fn contains_external_navigation(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let compact = lower
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    let external_hrefs = [
        "href=\"http://",
        "href=\"https://",
        "href='http://",
        "href='https://",
        "href=http://",
        "href=https://",
        "href=\"file:",
        "href='file:",
        "href=file:",
        "href=\"javascript:",
        "href='javascript:",
        "href=javascript:",
        "href=\"data:text/html",
        "href='data:text/html",
        "href=data:text/html",
    ];
    external_hrefs.iter().any(|needle| compact.contains(needle))
        || compact.contains("http-equiv=\"refresh\"")
        || compact.contains("http-equiv='refresh'")
        || compact.contains("http-equiv=refresh")
        || compact.contains("window.location")
        || compact.contains("document.location")
        || compact.contains("globalthis.location")
        || compact.contains("location.href")
        || compact.contains("location.assign(")
        || compact.contains("location.replace(")
        || compact.contains("window.open(")
        || compact.contains("<base")
}

fn check(name: &str, passed: bool, detail: &str) -> VerificationCheck {
    VerificationCheck {
        name: name.to_string(),
        status: if passed { "passed" } else { "failed" }.to_string(),
        detail: detail.to_string(),
    }
}

fn safe_filename(title: &str) -> String {
    let mut out = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "artifact".to_string()
    } else {
        out.chars().take(80).collect()
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn add_zip_file(
    writer: &mut zip::ZipWriter<&mut Cursor<Vec<u8>>>,
    path: &str,
    bytes: &[u8],
    options: SimpleFileOptions,
) -> Result<()> {
    writer.start_file(path, options)?;
    writer.write_all(bytes)?;
    Ok(())
}

fn verify_zip_manifest(bytes: &[u8]) -> std::result::Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("cannot open generated ZIP: {error}"))?;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("cannot inspect ZIP member: {error}"))?;
        let name = entry.name();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(format!("unsafe ZIP member path: {name}"));
        }
    }
    let manifest: Value = {
        let mut entry = archive
            .by_name("manifest.json")
            .map_err(|error| format!("manifest.json missing: {error}"))?;
        let mut manifest_bytes = Vec::new();
        entry
            .read_to_end(&mut manifest_bytes)
            .map_err(|error| format!("cannot read manifest.json: {error}"))?;
        serde_json::from_slice(&manifest_bytes)
            .map_err(|error| format!("invalid manifest.json: {error}"))?
    };
    let files = manifest
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| "manifest files map is missing".to_string())?;
    for (name, expected) in files {
        let mut entry = archive
            .by_name(name)
            .map_err(|error| format!("manifest member {name} missing: {error}"))?;
        let mut body = Vec::new();
        entry
            .read_to_end(&mut body)
            .map_err(|error| format!("cannot read {name}: {error}"))?;
        let expected_size = expected
            .get("sizeBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("manifest member {name} has no sizeBytes"))?;
        let expected_hash = expected
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("manifest member {name} has no sha256"))?;
        if body.len() as u64 != expected_size || sha256_hex(&body) != expected_hash {
            return Err(format!("manifest mismatch for {name}"));
        }
    }
    Ok(())
}

fn emit_artifact_event(name: &str, artifact_id: &str, version: i64, detail: Option<&str>) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            name,
            json!({
                "artifactId": artifact_id,
                "version": version,
                "detail": detail
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_analysis_chart_bindings() {
        let artifact = AnalysisArtifactV1 {
            schema_version: ANALYSIS_SCHEMA_VERSION.to_string(),
            question: "Why did activation change?".to_string(),
            audience: "Product".to_string(),
            decision: String::new(),
            status: "ready".to_string(),
            metric_definitions: vec![],
            time_range: None,
            filters: vec![],
            grain: None,
            datasets: vec![
                json!({"id":"activation","sourceIds":["events"],"rowCount":0,"rows":[]}),
            ],
            findings: vec![],
            recommendations: vec![],
            caveats: vec![],
            blocks: vec![],
            charts: vec![
                json!({"dataset":"activation","sourceId":"events","fallbackId":"activation-table"}),
            ],
            tables: vec![json!({"id":"activation-table"})],
            static_fallbacks: vec![],
            sources: vec![
                json!({"id":"events","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}),
            ],
            data_quality: vec![],
            claim_validation: vec![],
        };
        assert!(artifact.validate().is_ok());
    }

    #[test]
    fn rejects_unstructured_quality_and_claim_evidence() {
        let mut artifact = AnalysisArtifactV1 {
            schema_version: ANALYSIS_SCHEMA_VERSION.to_string(),
            question: "Why did activation change?".to_string(),
            audience: "Product".to_string(),
            decision: String::new(),
            status: "partial".to_string(),
            metric_definitions: vec![],
            time_range: None,
            filters: vec![],
            grain: None,
            datasets: vec![json!({"id":"activation","rowCount":0,"rows":[]})],
            findings: vec![],
            recommendations: vec![],
            caveats: vec![],
            blocks: vec![],
            charts: vec![],
            tables: vec![],
            static_fallbacks: vec![],
            sources: vec![
                json!({"id":"events","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}),
            ],
            data_quality: vec![json!({})],
            claim_validation: vec![],
        };
        assert!(artifact.validate().is_err());
        artifact.data_quality.clear();
        artifact.claim_validation.push(json!({}));
        assert!(artifact.validate().is_err());
    }

    #[test]
    fn offline_renderer_contains_no_remote_runtime() {
        let html = wrap_offline_document("Title", "<h1>Title</h1><p>Body</p>");
        assert!(!contains_remote_dependency(&html));
        assert!(html.contains("Content-Security-Policy"));
    }

    #[test]
    fn markdown_raw_html_is_escaped_and_navigation_is_detected() {
        let rendered = markdown_to_html(
            "# Safe\n\n<meta http-equiv=\"refresh\" content=\"0;url=https://example.com\">",
        );
        assert!(rendered.contains("&lt;meta"));
        assert!(!rendered.contains("<meta"));
        assert!(contains_external_navigation(
            "<script>window.location='https://example.com'</script>"
        ));
        assert!(contains_external_navigation(
            "<a href=\"https://example.com\">leave</a>"
        ));
    }

    #[test]
    fn filename_is_portable() {
        assert_eq!(
            safe_filename("Quarterly report: Q2 / APAC"),
            "Quarterly-report-Q2-APAC"
        );
    }

    #[test]
    fn durable_privacy_rejects_incognito_label() {
        assert_eq!(
            normalize_durable_privacy("local_private").expect("local privacy"),
            "local_private"
        );
        assert_eq!(
            normalize_durable_privacy("unknown").expect("unknown privacy fallback"),
            "local_private"
        );
        assert!(normalize_durable_privacy("incognito").is_err());
    }

    #[test]
    fn golden_analysis_fixtures_are_schema_valid() {
        let ready: AnalysisArtifactV1 = serde_json::from_str(include_str!(
            "../../tests/fixtures/artifacts/analysis-ready.json"
        ))
        .expect("parse ready fixture");
        ready.validate().expect("validate ready fixture");
        assert_eq!(ready.status, "ready");
        let markdown = render_analysis_markdown(&ready);
        assert!(markdown.contains("63.6% (7/11)"));
        assert!(markdown.contains("| web | 5 | 5 |"));
        assert!(
            markdown.contains("750b3f0dcc6bd06cfd1b0968a0c63114909d2880cc88dd304c59dc0fc8118e49")
        );
        let rendered_html = render_analysis_html(&ready);
        assert!(rendered_html.contains("class=\"report-hero\""));
        assert!(rendered_html.contains("class=\"bar-plot\""));
        assert!(rendered_html.contains("class=\"quality-summary\""));
        assert!(rendered_html.contains("class=\"table-scroll\""));
        assert!(rendered_html.contains("overflow-x:hidden"));
        assert!(!rendered_html.contains("visualization backed by dataset"));
        assert!(!contains_remote_dependency(&rendered_html));

        let blocked: AnalysisArtifactV1 = serde_json::from_str(include_str!(
            "../../tests/fixtures/artifacts/analysis-blocked.json"
        ))
        .expect("parse blocked fixture");
        blocked.validate().expect("validate blocked fixture");
        assert_eq!(blocked.status, "blocked");
    }

    #[test]
    fn analysis_import_rejects_external_navigation() {
        let mut analysis: AnalysisArtifactV1 = serde_json::from_str(include_str!(
            "../../tests/fixtures/artifacts/analysis-ready.json"
        ))
        .expect("parse ready fixture");
        analysis.blocks[0]["body"] =
            Value::String("See [remote details](https://example.com/report)".to_string());
        let temp = tempfile::tempdir().expect("create tempdir");
        let path = temp.path().join("artifact.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&analysis).expect("serialize analysis"),
        )
        .expect("write analysis fixture");

        let error = match prepare_payload(&path) {
            Ok(_) => panic!("external navigation must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("external navigation"));
    }

    #[test]
    fn imported_html_gets_offline_policy() {
        let html = normalize_imported_html(
            "<html><head><title>T</title></head><body><p>ok</p></body></html>",
        );
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("connect-src 'none'"));
        assert!(!contains_remote_dependency(&html));
    }
}
