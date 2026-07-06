//! 设计空间元数据注册表（`design.db`）。
//!
//! 表是**元数据注册表 / 可重建索引**：产物正文（`index.html` / `source/`）与
//! 设计系统正文（`SYSTEM.md`）在磁盘，`reindex` 可从磁盘全量重建（对齐知识空间
//! "索引可重建" 红线，见 `docs/architecture/design-space.md` §4）。

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

// ── Types ──────────────────────────────────────────────────────────

/// 设计项目：顶层容器，聚合一组产物。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignProject {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// 默认设计系统（弱引用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_system_id: Option<String>,
    /// 绑定的 Hope Agent 项目（弱引用，D4 联动）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 产物数量（列表页展示用，读取时聚合）。
    #[serde(default)]
    pub artifact_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

/// 单个可交付产物。对应磁盘一个目录 + 一份自包含 `index.html`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignArtifact {
    pub id: String,
    pub project_id: String,
    pub title: String,
    /// web|mobile|deck|dashboard|poster|document|email|image
    pub kind: String,
    /// 覆盖项目默认设计系统（弱引用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_id: Option<String>,
    /// planned|generating|ready|failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport_w: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewport_h: Option<i64>,
    pub current_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critique_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

/// 产物版本快照（元数据；正文在磁盘 `versions/{n}/`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignArtifactVersion {
    pub id: i64,
    pub artifact_id: String,
    pub version_number: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critique_score: Option<f64>,
    pub created_at: String,
}

/// 设计系统的可重建索引（正文是磁盘 `SYSTEM.md`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignSystemMeta {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// builtin|user|extracted
    pub source: String,
    /// 分组类目（品牌品类 / 原创原型），仅用于 GUI 选择器分组；用户系统为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Column lists / row mappers ─────────────────────────────────────

const PROJECT_COLUMNS: &str = "SELECT p.id, p.title, p.description, p.color, p.default_system_id, \
     p.ha_project_id, p.session_id, p.agent_id, p.created_at, p.updated_at, \
     (SELECT COUNT(*) FROM design_artifacts a WHERE a.project_id = p.id) AS artifact_count, p.metadata \
     FROM design_projects p";

fn map_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesignProject> {
    Ok(DesignProject {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        color: row.get(3)?,
        default_system_id: row.get(4)?,
        ha_project_id: row.get(5)?,
        session_id: row.get(6)?,
        agent_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        artifact_count: row.get(10)?,
        metadata: row.get(11)?,
    })
}

const ARTIFACT_COLUMNS: &str =
    "SELECT id, project_id, title, kind, system_id, status, viewport_w, \
     viewport_h, current_version, critique_score, thumbnail_path, created_at, updated_at, metadata \
     FROM design_artifacts";

fn map_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesignArtifact> {
    Ok(DesignArtifact {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        kind: row.get(3)?,
        system_id: row.get(4)?,
        status: row.get(5)?,
        viewport_w: row.get(6)?,
        viewport_h: row.get(7)?,
        current_version: row.get(8)?,
        critique_score: row.get(9)?,
        thumbnail_path: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        metadata: row.get(13)?,
    })
}

const SYSTEM_COLUMNS: &str =
    "SELECT id, name, slug, source, category, summary, thumbnail_path, created_at, \
     updated_at FROM design_systems";

fn map_system_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesignSystemMeta> {
    Ok(DesignSystemMeta {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        source: row.get(3)?,
        category: row.get(4)?,
        summary: row.get(5)?,
        thumbnail_path: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

// ── Database ───────────────────────────────────────────────────────

pub struct DesignDb {
    conn: Mutex<Connection>,
}

impl DesignDb {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS design_projects (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                color TEXT,
                default_system_id TEXT,
                ha_project_id TEXT,
                session_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT
            );

            CREATE TABLE IF NOT EXISTS design_artifacts (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES design_projects(id) ON DELETE CASCADE,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                system_id TEXT,
                status TEXT NOT NULL DEFAULT 'ready',
                viewport_w INTEGER,
                viewport_h INTEGER,
                current_version INTEGER DEFAULT 1,
                critique_score REAL,
                thumbnail_path TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                metadata TEXT
            );

            CREATE TABLE IF NOT EXISTS design_artifact_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
                version_number INTEGER NOT NULL,
                message TEXT,
                critique_score REAL,
                created_at TEXT NOT NULL,
                UNIQUE(artifact_id, version_number)
            );

            CREATE TABLE IF NOT EXISTS design_systems (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                slug TEXT NOT NULL,
                source TEXT NOT NULL,
                category TEXT,
                summary TEXT,
                thumbnail_path TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_design_artifacts_project
                ON design_artifacts(project_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_design_versions_artifact
                ON design_artifact_versions(artifact_id, version_number DESC);
            CREATE INDEX IF NOT EXISTS idx_design_projects_session
                ON design_projects(session_id, updated_at DESC);",
        )?;

        // `category` 为后加列：对已存在的旧 design.db 幂等补列（列已存在则忽略错误）。
        let _ = conn.execute("ALTER TABLE design_systems ADD COLUMN category TEXT", []);

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| anyhow::anyhow!("DesignDb lock poisoned: {e}"))
    }

    // ── Projects ───────────────────────────────────────────────────

    pub fn create_project(&self, p: &DesignProject) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_projects
                (id, title, description, color, default_system_id, ha_project_id,
                 session_id, agent_id, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                p.id,
                p.title,
                p.description,
                p.color,
                p.default_system_id,
                p.ha_project_id,
                p.session_id,
                p.agent_id,
                p.created_at,
                p.updated_at,
                p.metadata,
            ],
        )?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Result<Option<DesignProject>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS} WHERE p.id = ?1"))?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_project_row)?;
        match rows.next() {
            Some(Ok(p)) => Ok(Some(p)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<DesignProject>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{PROJECT_COLUMNS} ORDER BY p.updated_at DESC"))?;
        let rows = stmt.query_map([], map_project_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_projects_by_session(&self, session_id: &str) -> Result<Vec<DesignProject>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{PROJECT_COLUMNS} WHERE p.session_id = ?1 ORDER BY p.updated_at DESC"
        ))?;
        let rows = stmt.query_map(rusqlite::params![session_id], map_project_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 更新项目元数据。`None` 字段保持原值（COALESCE 语义）。
    pub fn update_project(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        color: Option<&str>,
        default_system_id: Option<&str>,
        ha_project_id: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_projects SET
                title = COALESCE(?2, title),
                description = COALESCE(?3, description),
                color = COALESCE(?4, color),
                default_system_id = COALESCE(?5, default_system_id),
                ha_project_id = COALESCE(?6, ha_project_id),
                updated_at = ?7
             WHERE id = ?1",
            rusqlite::params![
                id,
                title,
                description,
                color,
                default_system_id,
                ha_project_id,
                updated_at
            ],
        )?;
        Ok(())
    }

    pub fn touch_project(&self, id: &str, updated_at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_projects SET updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, updated_at],
        )?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM design_projects WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    // ── Artifacts ──────────────────────────────────────────────────

    pub fn create_artifact(&self, a: &DesignArtifact) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_artifacts
                (id, project_id, title, kind, system_id, status, viewport_w, viewport_h,
                 current_version, critique_score, thumbnail_path, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                a.id,
                a.project_id,
                a.title,
                a.kind,
                a.system_id,
                a.status,
                a.viewport_w,
                a.viewport_h,
                a.current_version,
                a.critique_score,
                a.thumbnail_path,
                a.created_at,
                a.updated_at,
                a.metadata,
            ],
        )?;
        Ok(())
    }

    pub fn get_artifact(&self, id: &str) -> Result<Option<DesignArtifact>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{ARTIFACT_COLUMNS} WHERE id = ?1"))?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_artifact_row)?;
        match rows.next() {
            Some(Ok(a)) => Ok(Some(a)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_artifacts(&self, project_id: &str) -> Result<Vec<DesignArtifact>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{ARTIFACT_COLUMNS} WHERE project_id = ?1 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map(rusqlite::params![project_id], map_artifact_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 全部产物（跨项目，用于产物库缩略图墙）。
    pub fn list_all_artifacts(&self) -> Result<Vec<DesignArtifact>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{ARTIFACT_COLUMNS} ORDER BY updated_at DESC"))?;
        let rows = stmt.query_map([], map_artifact_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 更新产物状态 / 版本 / 缩略图 / 评分。`None` 字段保持原值。
    #[allow(clippy::too_many_arguments)]
    pub fn update_artifact(
        &self,
        id: &str,
        title: Option<&str>,
        status: Option<&str>,
        current_version: Option<i64>,
        critique_score: Option<f64>,
        thumbnail_path: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET
                title = COALESCE(?2, title),
                status = COALESCE(?3, status),
                current_version = COALESCE(?4, current_version),
                critique_score = COALESCE(?5, critique_score),
                thumbnail_path = COALESCE(?6, thumbnail_path),
                updated_at = ?7
             WHERE id = ?1",
            rusqlite::params![
                id,
                title,
                status,
                current_version,
                critique_score,
                thumbnail_path,
                updated_at
            ],
        )?;
        Ok(())
    }

    /// 反 slop 自查专用：设 `status` + 覆写 `metadata`（含合并后的 `selfCheck` 键），可选
    /// 一并更新 `title` / `current_version`。`update_artifact` 刻意不碰 metadata，故自查
    /// 落盘走此方法（`metadata=None` 清空该列 = 回收自动标记）。
    #[allow(clippy::too_many_arguments)]
    pub fn update_artifact_review(
        &self,
        id: &str,
        title: Option<&str>,
        status: &str,
        current_version: Option<i64>,
        metadata: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET
                title = COALESCE(?2, title),
                status = ?3,
                current_version = COALESCE(?4, current_version),
                metadata = ?5,
                updated_at = ?6
             WHERE id = ?1",
            rusqlite::params![id, title, status, current_version, metadata, updated_at],
        )?;
        Ok(())
    }

    pub fn delete_artifact(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM design_artifacts WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    // ── Versions ───────────────────────────────────────────────────

    pub fn create_version(&self, v: &DesignArtifactVersion) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_artifact_versions
                (artifact_id, version_number, message, critique_score, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                v.artifact_id,
                v.version_number,
                v.message,
                v.critique_score,
                v.created_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_versions(&self, artifact_id: &str) -> Result<Vec<DesignArtifactVersion>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, artifact_id, version_number, message, critique_score, created_at
             FROM design_artifact_versions WHERE artifact_id = ?1 ORDER BY version_number DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![artifact_id], |row| {
            Ok(DesignArtifactVersion {
                id: row.get(0)?,
                artifact_id: row.get(1)?,
                version_number: row.get(2)?,
                message: row.get(3)?,
                critique_score: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 保留最新 `keep` 个版本，剪掉更旧的。
    pub fn cleanup_old_versions(&self, artifact_id: &str, keep: i64) -> Result<u64> {
        let conn = self.lock()?;
        let deleted = conn.execute(
            "DELETE FROM design_artifact_versions
             WHERE artifact_id = ?1 AND version_number NOT IN (
                SELECT version_number FROM design_artifact_versions
                WHERE artifact_id = ?1 ORDER BY version_number DESC LIMIT ?2
             )",
            rusqlite::params![artifact_id, keep],
        )?;
        Ok(deleted as u64)
    }

    // ── Systems (registry over SYSTEM.md) ──────────────────────────

    pub fn upsert_system(&self, s: &DesignSystemMeta) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_systems
                (id, name, slug, source, category, summary, thumbnail_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name, slug = excluded.slug, source = excluded.source,
                category = excluded.category, summary = excluded.summary,
                thumbnail_path = excluded.thumbnail_path, updated_at = excluded.updated_at",
            rusqlite::params![
                s.id,
                s.name,
                s.slug,
                s.source,
                s.category,
                s.summary,
                s.thumbnail_path,
                s.created_at,
                s.updated_at,
            ],
        )?;
        Ok(())
    }

    /// 为缺失分组类目的旧行补齐（仅填 `NULL`，绝不覆盖已有值）。
    pub fn backfill_system_category(&self, id: &str, category: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_systems SET category = ?2 WHERE id = ?1 AND category IS NULL",
            rusqlite::params![id, category],
        )?;
        Ok(())
    }

    pub fn get_system(&self, id: &str) -> Result<Option<DesignSystemMeta>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{SYSTEM_COLUMNS} WHERE id = ?1"))?;
        let mut rows = stmt.query_map(rusqlite::params![id], map_system_row)?;
        match rows.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_systems(&self) -> Result<Vec<DesignSystemMeta>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("{SYSTEM_COLUMNS} ORDER BY source, name"))?;
        let rows = stmt.query_map([], map_system_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_system(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM design_systems WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }
}
