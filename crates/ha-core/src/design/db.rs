//! 设计空间元数据注册表（`design.db`）。
//!
//! 表是**元数据注册表 / 可重建索引**：产物正文（`index.html` / `source/`）与
//! 设计系统正文（`SYSTEM.md`）在磁盘，`reindex` 可从磁盘全量重建（对齐知识空间
//! "索引可重建" 红线，见 `docs/architecture/design-space.md` §4）。

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
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
    /// 代码仓库绑定源之二：Hope Agent 项目（弱引用，目录从其 working_dir 实时
    /// 派生、随用户改动跟随）。与 `code_dir` 互斥，单点见 `service::set_project_code_binding`。
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
    /// 待复查（`status='needs_review'`）产物数（列表页状态徽标用，读取时聚合）。
    #[serde(default)]
    pub needs_review_count: i64,
    /// 代码漂移（`metadata.codeDrift` 非空）产物数——绑定仓库落地后代码侧变更、
    /// 设计稿待更新（列表页 stale 徽标用，读取时聚合）。见 `design::code_sync`。
    #[serde(default)]
    pub code_drift_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    /// 项目对话的初始模型（首页生成时由所选模型写入）。弱引用：provider / 模型
    /// 已删则前端回退 agent 缺省；只作对话初始值，会话内切换不回写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<crate::provider::ActiveModel>,
    /// 代码仓库绑定源之一：本机目录（canonical 绝对路径）。与 `ha_project_id`
    /// 互斥；解析单一入口 `service::resolve_code_dir`（本列 > HA 项目派生）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_dir: Option<String>,
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
    /// 所属文件夹（页面分组，OD path-based 模型）：斜杠分隔的目录路径，空串 = 根。
    #[serde(default)]
    pub folder: String,
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
    /// 溯源：`ai`（生成 / 精修）/ `manual`（可视化编辑 / 换系统）/ `restore`（回滚）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 该版本对应的生成 prompt 摘要（仅 AI 版本有；供历史面板 popover 展示）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_summary: Option<String>,
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
    /// 选择器色板：4 槽语义行 `[bg, support, fg, accent]`（微缩主题条）。tokens.json
    /// 派生、**不落库**（list 时由 `system::system_swatches` 填充，tokens 变更自动跟随）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub swatches: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 元素锚定的批注钉（回灌对话让 AI 精修 + 标记已解决）。锚在 `(artifact, oid)`，
/// `rel_x/rel_y` 是钉相对锚元素包围盒的偏移（`0..1`，重锚渲染用）；`oid=None` = 脱锚。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignComment {
    pub id: i64,
    pub artifact_id: String,
    /// 锚定元素的 `data-ds-oid`（脱锚为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oid: Option<i64>,
    pub rel_x: f64,
    pub rel_y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// 命中元素摘要（≤400 字符，回灌对话上下文用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub body: String,
    pub resolved: bool,
    pub created_at: String,
}

/// 设计系统 → 代码工程的绑定（工程轴 D）：把多平台 token 同步落到外部代码工程目录。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignCodeBinding {
    pub id: i64,
    pub system_id: String,
    /// 代码工程根目录（绝对路径，创建时 canonicalize）。
    pub target_dir: String,
    /// 写入子目录（相对 `target_dir`，空=根）。
    pub subfolder: String,
    /// 要写入的 token 格式 id（css/scss/ts/swift/android/dtcg 子集）。
    pub formats: Vec<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
}

/// 部署历史一条（provider + 公开 URL + 时间）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRecord {
    pub provider: String,
    pub url: String,
    pub created_at: String,
}

/// 「实现到代码」一次落地的回执（`design::code_sync`）。锚定「产物 → 会话 → 落地目录」并记
/// 收割进度（`harvest_cursor` = 已扫到的最大会话 message id，增量幂等）与基线 revision。
#[derive(Debug, Clone)]
pub struct DesignImplementReceipt {
    pub id: String,
    pub artifact_id: String,
    /// 承接实现的普通 chat 会话（sessions.db 弱引用，无跨库 FK）。
    pub session_id: String,
    /// implement 时的 canonical 代码目录快照。
    pub code_dir: String,
    /// implement 时的 git 指纹（非 git 目录 = None）。
    pub base_revision: Option<String>,
    /// 最近一次收割/同步时的 git 指纹（drift 检查短路用，可 None）。
    pub harvest_revision: Option<String>,
    /// 已收割到的最大会话 message id（增量游标）。
    pub harvest_cursor: i64,
    pub created_at: String,
    pub harvested_at: Option<String>,
}

/// 回执下的一个「产物落地文件」链接及其基线（内容 hash + gzip 快照供 diff 回放）。
#[derive(Debug, Clone)]
pub struct DesignCodeLink {
    pub id: i64,
    pub receipt_id: String,
    /// 相对 `receipt.code_dir` 的路径（正斜杠）。
    pub rel_path: String,
    /// 基线内容 BLAKE3（收割/同步时的磁盘态）。
    pub blake3: String,
    pub size_bytes: i64,
    /// gzip 基线快照（≤512KB 原文且非二进制才存；否则 None → UI 降级不出内嵌 diff）。
    pub content_gz: Option<Vec<u8>>,
    pub linked_at: String,
    pub synced_at: String,
}

// ── Column lists / row mappers ─────────────────────────────────────

fn row_to_code_binding(row: &rusqlite::Row) -> rusqlite::Result<DesignCodeBinding> {
    let formats_json: String = row.get(4)?;
    Ok(DesignCodeBinding {
        id: row.get(0)?,
        system_id: row.get(1)?,
        target_dir: row.get(2)?,
        subfolder: row.get(3)?,
        formats: serde_json::from_str(&formats_json).unwrap_or_default(),
        created_at: row.get(5)?,
        last_synced_at: row.get(6)?,
    })
}

const PROJECT_COLUMNS: &str = "SELECT p.id, p.title, p.description, p.color, p.default_system_id, \
     p.ha_project_id, p.session_id, p.agent_id, p.created_at, p.updated_at, \
     (SELECT COUNT(*) FROM design_artifacts a WHERE a.project_id = p.id) AS artifact_count, \
     (SELECT COUNT(*) FROM design_artifacts a WHERE a.project_id = p.id AND a.status = 'needs_review') AS needs_review_count, \
     (SELECT COUNT(*) FROM design_artifacts a WHERE a.project_id = p.id \
        AND a.metadata IS NOT NULL AND json_valid(a.metadata) \
        AND json_extract(a.metadata, '$.codeDrift') IS NOT NULL) AS code_drift_count, \
     p.metadata, p.default_model, p.code_dir \
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
        needs_review_count: row.get(11)?,
        code_drift_count: row.get(12)?,
        metadata: row.get(13)?,
        // TEXT JSON 列;损坏 / 旧行 NULL 一律回 None(弱引用,消费端自兜底)。
        default_model: row
            .get::<_, Option<String>>(14)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        code_dir: row.get(15)?,
    })
}

const ARTIFACT_COLUMNS: &str =
    "SELECT id, project_id, title, kind, system_id, status, viewport_w, \
     viewport_h, current_version, critique_score, thumbnail_path, created_at, updated_at, metadata, \
     COALESCE(folder, '') \
     FROM design_artifacts";

/// 转义 SQL LIKE 通配符（`\` `%` `_`），配合 `ESCAPE '\'` 使文件夹名里的 `_`/`%` 按**字面**匹配。
/// 否则 `app_a/%` 会把 `_` 当单字通配、误匹配同长兄弟 `app-a/...`（跨文件夹误伤，review HIGH 修复）。
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

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
        folder: row.get(14)?,
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
        swatches: Vec::new(),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

const COMMENT_COLUMNS: &str =
    "SELECT id, artifact_id, oid, rel_x, rel_y, tag, snippet, body, resolved, created_at \
     FROM design_comments";

fn map_comment_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesignComment> {
    Ok(DesignComment {
        id: row.get(0)?,
        artifact_id: row.get(1)?,
        oid: row.get(2)?,
        rel_x: row.get(3)?,
        rel_y: row.get(4)?,
        tag: row.get(5)?,
        snippet: row.get(6)?,
        body: row.get(7)?,
        resolved: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
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
                metadata TEXT,
                default_model TEXT,
                code_dir TEXT
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
                origin TEXT,
                prompt_summary TEXT,
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

            CREATE TABLE IF NOT EXISTS design_comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
                oid INTEGER,
                rel_x REAL NOT NULL DEFAULT 0,
                rel_y REAL NOT NULL DEFAULT 0,
                tag TEXT,
                snippet TEXT,
                body TEXT NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS design_code_bindings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                system_id TEXT NOT NULL REFERENCES design_systems(id) ON DELETE CASCADE,
                target_dir TEXT NOT NULL,
                subfolder TEXT NOT NULL DEFAULT '',
                formats TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_synced_at TEXT
            );

            -- B7-1 分享：不可猜 token → 产物只读快照（server 模式公开路由查此表）。
            -- 产物删除级联删分享；每产物至多一条（uq 唯一）以便「已分享则复用同一链接」。
            CREATE TABLE IF NOT EXISTS design_shares (
                token TEXT PRIMARY KEY,
                artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS uq_design_shares_artifact
                ON design_shares(artifact_id);

            CREATE INDEX IF NOT EXISTS idx_design_artifacts_project
                ON design_artifacts(project_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_design_versions_artifact
                ON design_artifact_versions(artifact_id, version_number DESC);
            CREATE INDEX IF NOT EXISTS idx_design_projects_session
                ON design_projects(session_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_design_comments_artifact
                ON design_comments(artifact_id, resolved, id);
            CREATE INDEX IF NOT EXISTS idx_design_code_bindings_system
                ON design_code_bindings(system_id, id);",
        )?;

        // 后加列：对已存在的旧 design.db 幂等补列（列已存在则忽略错误）。
        let _ = conn.execute("ALTER TABLE design_systems ADD COLUMN category TEXT", []);
        // 项目对话初始模型（首页所选模型带入项目）：TEXT JSON（ActiveModel）。
        let _ = conn.execute(
            "ALTER TABLE design_projects ADD COLUMN default_model TEXT",
            [],
        );
        // 代码仓库绑定（本机目录源）：canonical 绝对路径；与 ha_project_id（HA 项目源）
        // 互斥，互斥由 service::set_project_code_binding 单点保证。旧 dev DB 幂等补列。
        let _ = conn.execute("ALTER TABLE design_projects ADD COLUMN code_dir TEXT", []);
        // MCP `design_get_active_context` 的事实源：GUI 打开产物时上报「最近查看」。不进
        // PROJECT_COLUMNS/DTO/mapper（可丢 UI 痕迹，reindex 丢了走 fallback）。
        let _ = conn.execute(
            "ALTER TABLE design_projects ADD COLUMN last_opened_artifact_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE design_projects ADD COLUMN last_opened_at TEXT",
            [],
        );
        // B3 版本溯源（origin: ai/manual/restore + 生成 prompt 摘要）——分支内 dev DB 幂等补列。
        let _ = conn.execute(
            "ALTER TABLE design_artifact_versions ADD COLUMN origin TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE design_artifact_versions ADD COLUMN prompt_summary TEXT",
            [],
        );
        // 产物页面排序（用户可拖动排序）：per-project 位序。旧 dev DB 幂等补列 + 按 created_at
        // 回填 1-based 位序（仅 NULL，幂等）；新库/新行的位序在 create_artifact INSERT 时自增。
        let _ = conn.execute(
            "ALTER TABLE design_artifacts ADD COLUMN position INTEGER",
            [],
        );
        let _ = conn.execute(
            "UPDATE design_artifacts SET position = (
                 SELECT COUNT(*) FROM design_artifacts a2
                 WHERE a2.project_id = design_artifacts.project_id
                   AND (a2.created_at < design_artifacts.created_at
                        OR (a2.created_at = design_artifacts.created_at AND a2.id <= design_artifacts.id))
             ) WHERE position IS NULL",
            [],
        );
        // 页面分组文件夹（OD path-based 模型）：artifact.folder = 斜杠目录路径（空=根）。
        // 旧 dev DB 幂等补列（NOT NULL DEFAULT '' 自动回填旧行）。
        let _ = conn.execute(
            "ALTER TABLE design_artifacts ADD COLUMN folder TEXT NOT NULL DEFAULT ''",
            [],
        );
        // 持久化空文件夹（无产物的文件夹仍要可见/可导航——OD 同样持久化空目录，与路径派生的合并）。
        conn.execute(
            "CREATE TABLE IF NOT EXISTS design_folders (
                project_id TEXT NOT NULL REFERENCES design_projects(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (project_id, path)
            )",
            [],
        )?;
        // 部署历史（每次成功部署一行：provider + 公开 URL + 时间）。审计 + GUI 展示历史链接。
        conn.execute(
            "CREATE TABLE IF NOT EXISTS design_deployments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
                provider TEXT NOT NULL,
                url TEXT NOT NULL,
                created_at TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_design_deployments_artifact
                ON design_deployments(artifact_id, id DESC)",
            [],
        )?;

        // ── code→design 回灌（stale 检测）：implement_to_code 落地回执 + 产物↔落地文件关联 ──
        // 回执 = 一次「实现到代码」的锚点（哪个产物 / 哪个会话 / 落到哪个目录 / 基线 revision +
        // 已收割到的会话消息游标）。links = 从会话 write/edit 元数据收割出的「产物落地文件」及其
        // 基线内容 hash + gzip 快照（供 diff 回放）。两级 CASCADE，会话侧是弱引用（无跨库 FK）。
        conn.execute(
            "CREATE TABLE IF NOT EXISTS design_implement_receipts (
                id TEXT PRIMARY KEY,
                artifact_id TEXT NOT NULL REFERENCES design_artifacts(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                code_dir TEXT NOT NULL,
                base_revision TEXT,
                harvest_revision TEXT,
                harvest_cursor INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                harvested_at TEXT
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_design_receipts_artifact
                ON design_implement_receipts(artifact_id, created_at DESC)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS design_code_links (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                receipt_id TEXT NOT NULL REFERENCES design_implement_receipts(id) ON DELETE CASCADE,
                rel_path TEXT NOT NULL,
                blake3 TEXT NOT NULL,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                content_gz BLOB,
                linked_at TEXT NOT NULL,
                synced_at TEXT NOT NULL,
                UNIQUE(receipt_id, rel_path)
            )",
            [],
        )?;

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
                 session_id, agent_id, created_at, updated_at, metadata, default_model,
                 code_dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                p.default_model
                    .as_ref()
                    .and_then(|m| serde_json::to_string(m).ok()),
                p.code_dir,
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

    /// 更新项目元数据。`None` 字段保持原值（COALESCE 语义）。**不含代码仓库绑定**
    /// （`code_dir` / `ha_project_id`）——那两列的唯一写入口是 `set_project_code_binding`
    /// 单点（保证互斥 + 校验 + stale 语义，review F1：否则 update 可绕过互斥破坏不变量）。
    pub fn update_project(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        color: Option<&str>,
        default_system_id: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_projects SET
                title = COALESCE(NULLIF(?2, ''), title),
                description = COALESCE(?3, description),
                color = COALESCE(?4, color),
                default_system_id = COALESCE(?5, default_system_id),
                updated_at = ?6
             WHERE id = ?1",
            rusqlite::params![id, title, description, color, default_system_id, updated_at],
        )?;
        Ok(())
    }

    /// 代码仓库绑定两列 verbatim 覆写（`None` = 清除，非 COALESCE 语义）。
    /// 「二选一或全空」的互斥由 `service::set_project_code_binding` 单点校验。
    pub fn set_project_code_binding(
        &self,
        id: &str,
        code_dir: Option<&str>,
        ha_project_id: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_projects SET code_dir = ?2, ha_project_id = ?3, updated_at = ?4
             WHERE id = ?1",
            rusqlite::params![id, code_dir, ha_project_id, updated_at],
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
                 current_version, critique_score, thumbnail_path, created_at, updated_at, metadata, folder, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 (SELECT COALESCE(MAX(position), 0) + 1 FROM design_artifacts WHERE project_id = ?2))",
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
                a.folder,
            ],
        )?;
        Ok(())
    }

    /// 移动产物到文件夹（改 `folder`；空串 = 根）。轻量元数据更新，不动正文/版本。
    pub fn set_artifact_folder(&self, id: &str, folder: &str, updated_at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET folder = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, folder, updated_at],
        )?;
        Ok(())
    }

    // ── Folders（页面分组，OD path-based）──────────────────────────────
    /// 项目内全部文件夹路径：产物 folder 去重 ∪ 持久化空文件夹（含所有祖先路径）。
    pub fn list_folder_paths(&self, project_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut set = std::collections::BTreeSet::new();
        // 产物所在文件夹（及其祖先）。
        let mut stmt = conn.prepare(
            "SELECT DISTINCT folder FROM design_artifacts WHERE project_id = ?1 AND folder <> ''",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |r| r.get::<_, String>(0))?;
        for r in rows {
            let p = r?;
            // 派生所有祖先段（a/b/c → a, a/b, a/b/c），确保中间层可导航。
            let mut acc = String::new();
            for seg in p.split('/').filter(|s| !s.is_empty()) {
                if acc.is_empty() {
                    acc = seg.to_string();
                } else {
                    acc = format!("{acc}/{seg}");
                }
                set.insert(acc.clone());
            }
        }
        // 持久化空文件夹（含祖先）。
        let mut stmt2 = conn.prepare("SELECT path FROM design_folders WHERE project_id = ?1")?;
        let rows2 = stmt2.query_map(rusqlite::params![project_id], |r| r.get::<_, String>(0))?;
        for r in rows2 {
            let p = r?;
            let mut acc = String::new();
            for seg in p.split('/').filter(|s| !s.is_empty()) {
                if acc.is_empty() {
                    acc = seg.to_string();
                } else {
                    acc = format!("{acc}/{seg}");
                }
                set.insert(acc.clone());
            }
        }
        Ok(set.into_iter().collect())
    }

    /// 持久化一个（空）文件夹。
    pub fn create_folder(&self, project_id: &str, path: &str, created_at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO design_folders (project_id, path, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![project_id, path, created_at],
        )?;
        Ok(())
    }

    /// 删除持久化文件夹记录（含子文件夹记录，前缀匹配）。产物迁移在 service 层处理。
    pub fn delete_folder_records(&self, project_id: &str, path: &str) -> Result<()> {
        let conn = self.lock()?;
        let like = format!("{}/%", escape_like(path));
        conn.execute(
            "DELETE FROM design_folders WHERE project_id = ?1 \
             AND (path = ?2 OR path LIKE ?3 ESCAPE '\\')",
            rusqlite::params![project_id, path, like],
        )?;
        Ok(())
    }

    /// 文件夹改名/移动：把 `from`（及子路径 `from/…`）前缀替换为 `to`，同时改产物 folder 与持久化记录。
    pub fn rename_folder_prefix(
        &self,
        project_id: &str,
        from: &str,
        to: &str,
        updated_at: &str,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        // LIKE pattern 转义（防 `_`/`%` 误匹配兄弟文件夹，review HIGH）。
        let like = format!("{}/%", escape_like(from));
        // 子路径 substr 从 `from_chars+1`（1-based）起——**SQLite substr 按字符计数、非字节**，故用
        // `chars().count()` 而非 `len()`（否则中文等多字节名的子路径被截断/丢失，review HIGH）。substr
        // 含前导 `/`，故前缀拼 `to`（非 `to/`）不产生双斜杠：`to` || `/login` = `to/login`。
        let from_chars = from.chars().count() as i64;
        // 精确等于 from 的产物 → to。
        tx.execute(
            "UPDATE design_artifacts SET folder = ?3, updated_at = ?4 WHERE project_id = ?1 AND folder = ?2",
            rusqlite::params![project_id, from, to, updated_at],
        )?;
        // 子路径 from/… → to/…（替换前缀）。
        tx.execute(
            "UPDATE design_artifacts SET folder = ?3 || substr(folder, ?4), updated_at = ?5 \
             WHERE project_id = ?1 AND folder LIKE ?2 ESCAPE '\\'",
            rusqlite::params![project_id, like, to, from_chars + 1, updated_at],
        )?;
        // 持久化记录同理。
        tx.execute(
            "UPDATE OR IGNORE design_folders SET path = ?3 WHERE project_id = ?1 AND path = ?2",
            rusqlite::params![project_id, from, to],
        )?;
        tx.execute(
            "UPDATE OR IGNORE design_folders SET path = ?3 || substr(path, ?4) \
             WHERE project_id = ?1 AND path LIKE ?2 ESCAPE '\\'",
            rusqlite::params![project_id, like, to, from_chars + 1],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 把某文件夹（及子文件夹）内的产物全部移到根（删文件夹时用）。
    pub fn detach_artifacts_from_folder(
        &self,
        project_id: &str,
        path: &str,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        let like = format!("{}/%", escape_like(path));
        conn.execute(
            "UPDATE design_artifacts SET folder = '', updated_at = ?4 \
             WHERE project_id = ?1 AND (folder = ?2 OR folder LIKE ?3 ESCAPE '\\')",
            rusqlite::params![project_id, path, like, updated_at],
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
        // 用户可拖动排序 → 按 position 升序（created_at 兜底 tiebreak，回填后一般已全非空）。
        let mut stmt = conn.prepare(&format!(
            "{ARTIFACT_COLUMNS} WHERE project_id = ?1 ORDER BY position ASC, created_at ASC"
        ))?;
        let rows = stmt.query_map(rusqlite::params![project_id], map_artifact_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 轻量改名：仅更新 `title` + `updated_at`（不重渲染、不新增版本、不碰 source）。
    pub fn rename_artifact(&self, id: &str, title: &str, updated_at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET title = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, title, updated_at],
        )?;
        Ok(())
    }

    /// 重排 project 内产物页面顺序：按 `ordered_ids` 下标写 `position`（事务，仅本项目行）。
    pub fn reorder_artifacts(&self, project_id: &str, ordered_ids: &[String]) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        for (idx, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE design_artifacts SET position = ?3 WHERE id = ?1 AND project_id = ?2",
                rusqlite::params![id, project_id, idx as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
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
                title = COALESCE(NULLIF(?2, ''), title),
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
                title = COALESCE(NULLIF(?2, ''), title),
                status = ?3,
                current_version = COALESCE(?4, current_version),
                metadata = ?5,
                updated_at = ?6
             WHERE id = ?1",
            rusqlite::params![id, title, status, current_version, metadata, updated_at],
        )?;
        Ok(())
    }

    /// 只更新 metadata（演讲者备注等旁路数据；不碰 status/version）。
    pub fn update_artifact_metadata(
        &self,
        id: &str,
        metadata: Option<&str>,
        updated_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET metadata = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, metadata, updated_at],
        )?;
        Ok(())
    }

    /// 写 metadata 但**不动 `updated_at`**（code_sync 的 drift 检查/清标专用）——浏览态的
    /// 后台检查不得抬 `updated_at`，否则产物墙 `ORDER BY updated_at DESC` 每次检查都重排。
    pub fn set_artifact_metadata_quiet(&self, id: &str, metadata: Option<&str>) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET metadata = ?2 WHERE id = ?1",
            rusqlite::params![id, metadata],
        )?;
        Ok(())
    }

    /// 就地换设计系统（restyle）：改产物的 `system_id`（弱引用，允许 None = 不用设计系统）。
    pub fn set_artifact_system_id(&self, id: &str, system_id: Option<&str>) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_artifacts SET system_id = ?2 WHERE id = ?1",
            rusqlite::params![id, system_id],
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
                (artifact_id, version_number, message, critique_score, origin, prompt_summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                v.artifact_id,
                v.version_number,
                v.message,
                v.critique_score,
                v.origin,
                v.prompt_summary,
                v.created_at,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_versions(&self, artifact_id: &str) -> Result<Vec<DesignArtifactVersion>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, artifact_id, version_number, message, critique_score, origin, prompt_summary, created_at
             FROM design_artifact_versions WHERE artifact_id = ?1 ORDER BY version_number DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![artifact_id], |row| {
            Ok(DesignArtifactVersion {
                id: row.get(0)?,
                artifact_id: row.get(1)?,
                version_number: row.get(2)?,
                message: row.get(3)?,
                critique_score: row.get(4)?,
                origin: row.get(5)?,
                prompt_summary: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 保留最新 `keep` 个版本，剪掉更旧的。
    /// 版本上限淘汰（W4-O：里程碑保护）。超出 `keep` 时**优先淘汰最旧的 `manual`（微调自动保存）
    /// 版本**，保留 `ai` / `restore` 里程碑与当前（最新 version_number）版本——否则一轮重度可视化
    /// 微调会把 AI 生成的里程碑从 50 条上限里挤掉。仅当 manual 淘尽仍超限才淘汰最旧的 ai/restore。
    /// `origin` 为 NULL 的历史行按 `manual` 处理（优先淘汰）。
    pub fn cleanup_old_versions(&self, artifact_id: &str, keep: i64) -> Result<u64> {
        let conn = self.lock()?;
        let deleted = conn.execute(
            "DELETE FROM design_artifact_versions
             WHERE artifact_id = ?1 AND version_number IN (
                SELECT version_number FROM design_artifact_versions
                WHERE artifact_id = ?1
                  AND version_number <> (
                    SELECT MAX(version_number) FROM design_artifact_versions WHERE artifact_id = ?1
                  )
                ORDER BY (COALESCE(origin, 'manual') = 'manual') DESC, version_number ASC
                LIMIT MAX(
                    0,
                    (SELECT COUNT(*) FROM design_artifact_versions WHERE artifact_id = ?1) - ?2
                )
             )",
            rusqlite::params![artifact_id, keep],
        )?;
        Ok(deleted as u64)
    }

    // ── Shares（B7-1 只读分享）────────────────────────────────────

    /// 幂等建分享：产物已有分享则复用同一 token（不换链接），否则插新行。返回 token。
    pub fn upsert_share(&self, artifact_id: &str, token: &str, created_at: &str) -> Result<String> {
        let conn = self.lock()?;
        if let Ok(existing) = conn.query_row(
            "SELECT token FROM design_shares WHERE artifact_id = ?1",
            rusqlite::params![artifact_id],
            |r| r.get::<_, String>(0),
        ) {
            return Ok(existing);
        }
        conn.execute(
            "INSERT INTO design_shares (token, artifact_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![token, artifact_id, created_at],
        )?;
        Ok(token.to_string())
    }

    /// token → artifact_id（公开路由查此，找不到返回 None）。
    pub fn resolve_share(&self, token: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT artifact_id FROM design_shares WHERE token = ?1",
                rusqlite::params![token],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// 产物当前分享 token（GUI 显示已有链接）。
    pub fn share_token_for_artifact(&self, artifact_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT token FROM design_shares WHERE artifact_id = ?1",
                rusqlite::params![artifact_id],
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// 撤销分享（删 token 行）。返回是否删到。
    pub fn delete_share(&self, token: &str) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM design_shares WHERE token = ?1",
            rusqlite::params![token],
        )?;
        Ok(n > 0)
    }

    /// 记一条成功部署（provider + 公开 URL + 时间）。
    pub fn record_deployment(
        &self,
        artifact_id: &str,
        provider: &str,
        url: &str,
        created_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_deployments (artifact_id, provider, url, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![artifact_id, provider, url, created_at],
        )?;
        Ok(())
    }

    /// 产物部署历史（最新在前，最多 `limit` 条）。
    pub fn list_deployments(&self, artifact_id: &str, limit: u32) -> Result<Vec<DeploymentRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, url, created_at FROM design_deployments
             WHERE artifact_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![artifact_id, limit], |r| {
                Ok(DeploymentRecord {
                    provider: r.get(0)?,
                    url: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
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

    // ── Comments (批注钉) ───────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn add_comment(
        &self,
        artifact_id: &str,
        oid: Option<i64>,
        rel_x: f64,
        rel_y: f64,
        tag: Option<&str>,
        snippet: Option<&str>,
        body: &str,
        created_at: &str,
    ) -> Result<DesignComment> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_comments
                (artifact_id, oid, rel_x, rel_y, tag, snippet, body, resolved, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
            rusqlite::params![
                artifact_id,
                oid,
                rel_x,
                rel_y,
                tag,
                snippet,
                body,
                created_at
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(DesignComment {
            id,
            artifact_id: artifact_id.to_string(),
            oid,
            rel_x,
            rel_y,
            tag: tag.map(str::to_string),
            snippet: snippet.map(str::to_string),
            body: body.to_string(),
            resolved: false,
            created_at: created_at.to_string(),
        })
    }

    pub fn list_comments(&self, artifact_id: &str) -> Result<Vec<DesignComment>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{COMMENT_COLUMNS} WHERE artifact_id = ?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(rusqlite::params![artifact_id], map_comment_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 未解决批注数（W3-J badge）：走 `(artifact_id, resolved, id)` 索引，轻量 COUNT。
    pub fn count_open_comments(&self, artifact_id: &str) -> Result<i64> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM design_comments WHERE artifact_id = ?1 AND resolved = 0",
            rusqlite::params![artifact_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn get_comment(&self, artifact_id: &str, comment_id: i64) -> Result<Option<DesignComment>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "{COMMENT_COLUMNS} WHERE artifact_id = ?1 AND id = ?2"
        ))?;
        let mut rows =
            stmt.query_map(rusqlite::params![artifact_id, comment_id], map_comment_row)?;
        match rows.next() {
            Some(Ok(c)) => Ok(Some(c)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// 重锚：更新 `oid` + `rel` 位（用户拖拽 / 设计变更脱锚）。坐标由 owner 平面校验 + 钳制。
    pub fn update_comment_anchor(
        &self,
        artifact_id: &str,
        comment_id: i64,
        oid: Option<i64>,
        rel_x: f64,
        rel_y: f64,
    ) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE design_comments SET oid = ?3, rel_x = ?4, rel_y = ?5
             WHERE artifact_id = ?1 AND id = ?2",
            rusqlite::params![artifact_id, comment_id, oid, rel_x, rel_y],
        )?;
        Ok(n > 0)
    }

    /// 编辑批注正文。
    pub fn update_comment_body(
        &self,
        artifact_id: &str,
        comment_id: i64,
        body: &str,
    ) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE design_comments SET body = ?3 WHERE artifact_id = ?1 AND id = ?2",
            rusqlite::params![artifact_id, comment_id, body],
        )?;
        Ok(n > 0)
    }

    pub fn set_comment_resolved(
        &self,
        artifact_id: &str,
        comment_id: i64,
        resolved: bool,
    ) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE design_comments SET resolved = ?3 WHERE artifact_id = ?1 AND id = ?2",
            rusqlite::params![artifact_id, comment_id, resolved as i64],
        )?;
        Ok(n > 0)
    }

    pub fn delete_comment(&self, artifact_id: &str, comment_id: i64) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM design_comments WHERE artifact_id = ?1 AND id = ?2",
            rusqlite::params![artifact_id, comment_id],
        )?;
        Ok(n > 0)
    }

    // ── Code bindings (工程轴 D) ────────────────────────────────────

    pub fn add_code_binding(
        &self,
        system_id: &str,
        target_dir: &str,
        subfolder: &str,
        formats: &[String],
        created_at: &str,
    ) -> Result<DesignCodeBinding> {
        let formats_json = serde_json::to_string(formats)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_code_bindings
                (system_id, target_dir, subfolder, formats, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![system_id, target_dir, subfolder, formats_json, created_at],
        )?;
        Ok(DesignCodeBinding {
            id: conn.last_insert_rowid(),
            system_id: system_id.to_string(),
            target_dir: target_dir.to_string(),
            subfolder: subfolder.to_string(),
            formats: formats.to_vec(),
            created_at: created_at.to_string(),
            last_synced_at: None,
        })
    }

    pub fn list_code_bindings(&self, system_id: Option<&str>) -> Result<Vec<DesignCodeBinding>> {
        let conn = self.lock()?;
        let base =
            "SELECT id, system_id, target_dir, subfolder, formats, created_at, last_synced_at \
                    FROM design_code_bindings";
        let mut out = Vec::new();
        match system_id {
            Some(sid) => {
                let mut stmt =
                    conn.prepare(&format!("{base} WHERE system_id = ?1 ORDER BY id DESC"))?;
                let rows = stmt.query_map([sid], row_to_code_binding)?;
                for r in rows {
                    out.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(&format!("{base} ORDER BY id DESC"))?;
                let rows = stmt.query_map([], row_to_code_binding)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    pub fn get_code_binding(&self, id: i64) -> Result<Option<DesignCodeBinding>> {
        Ok(self
            .list_code_bindings(None)?
            .into_iter()
            .find(|b| b.id == id))
    }

    pub fn mark_binding_synced(&self, id: i64, at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_code_bindings SET last_synced_at = ?2 WHERE id = ?1",
            rusqlite::params![id, at],
        )?;
        Ok(())
    }

    pub fn delete_code_binding(&self, id: i64) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM design_code_bindings WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(n > 0)
    }

    // ── active context（MCP get_active_context 事实源）─────────────────

    /// 上报「最近查看的产物」（GUI 打开产物时）。**不动 updated_at**（浏览≠编辑，不得扰动
    /// 最近项目排序——由 service::mark_artifact_opened 保证只调本方法）。
    pub fn set_last_opened(&self, project_id: &str, artifact_id: &str, at: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_projects SET last_opened_artifact_id = ?2, last_opened_at = ?3 WHERE id = ?1",
            rusqlite::params![project_id, artifact_id, at],
        )?;
        Ok(())
    }

    /// 全局最近查看记录：(project_id, artifact_id, opened_at)。无记录 = None。
    pub fn last_opened(&self) -> Result<Option<(String, String, String)>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT id, last_opened_artifact_id, last_opened_at FROM design_projects
                 WHERE last_opened_at IS NOT NULL AND last_opened_artifact_id IS NOT NULL
                 ORDER BY last_opened_at DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row)
    }

    // ── code→design 回灌回执 / 链接（design::code_sync）─────────────────

    pub fn create_implement_receipt(&self, r: &DesignImplementReceipt) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_implement_receipts
                (id, artifact_id, session_id, code_dir, base_revision, harvest_revision,
                 harvest_cursor, created_at, harvested_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                r.id,
                r.artifact_id,
                r.session_id,
                r.code_dir,
                r.base_revision,
                r.harvest_revision,
                r.harvest_cursor,
                r.created_at,
                r.harvested_at,
            ],
        )?;
        Ok(())
    }

    fn map_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesignImplementReceipt> {
        Ok(DesignImplementReceipt {
            id: row.get(0)?,
            artifact_id: row.get(1)?,
            session_id: row.get(2)?,
            code_dir: row.get(3)?,
            base_revision: row.get(4)?,
            harvest_revision: row.get(5)?,
            harvest_cursor: row.get(6)?,
            created_at: row.get(7)?,
            harvested_at: row.get(8)?,
        })
    }

    const RECEIPT_COLUMNS: &'static str =
        "id, artifact_id, session_id, code_dir, base_revision, harvest_revision, \
         harvest_cursor, created_at, harvested_at";

    pub fn list_receipts_for_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Vec<DesignImplementReceipt>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {} FROM design_implement_receipts WHERE artifact_id = ?1 ORDER BY created_at ASC",
            Self::RECEIPT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![artifact_id], Self::map_receipt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 某项目下全部产物的回执（JOIN design_artifacts 过滤 project）。
    pub fn list_receipts_for_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<DesignImplementReceipt>> {
        let conn = self.lock()?;
        let sql = format!(
            "SELECT {} FROM design_implement_receipts r \
             JOIN design_artifacts a ON a.id = r.artifact_id \
             WHERE a.project_id = ?1 ORDER BY r.created_at ASC",
            Self::RECEIPT_COLUMNS
                .split(", ")
                .map(|c| format!("r.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![project_id], Self::map_receipt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn update_receipt_harvest(
        &self,
        id: &str,
        cursor: i64,
        harvest_revision: Option<&str>,
        harvested_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_implement_receipts
             SET harvest_cursor = ?2, harvest_revision = ?3, harvested_at = ?4 WHERE id = ?1",
            rusqlite::params![id, cursor, harvest_revision, harvested_at],
        )?;
        Ok(())
    }

    pub fn delete_receipt(&self, id: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM design_implement_receipts WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    /// upsert 一个 link 基线（收割命中即刷新 hash/快照/synced_at；`linked_at` 保留首见值）。
    pub fn upsert_code_link(
        &self,
        receipt_id: &str,
        rel_path: &str,
        blake3: &str,
        size_bytes: i64,
        content_gz: Option<&[u8]>,
        now: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO design_code_links
                (receipt_id, rel_path, blake3, size_bytes, content_gz, linked_at, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(receipt_id, rel_path) DO UPDATE SET
                blake3 = excluded.blake3,
                size_bytes = excluded.size_bytes,
                content_gz = excluded.content_gz,
                synced_at = excluded.synced_at",
            rusqlite::params![receipt_id, rel_path, blake3, size_bytes, content_gz, now],
        )?;
        Ok(())
    }

    /// 某产物全部回执的 links（JOIN，**不背 content_gz**——列表/比对用轻量态；快照单独取）。
    pub fn list_links_for_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Vec<(DesignImplementReceipt, DesignCodeLink)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT r.id, r.artifact_id, r.session_id, r.code_dir, r.base_revision, \
                    r.harvest_revision, r.harvest_cursor, r.created_at, r.harvested_at, \
                    l.id, l.receipt_id, l.rel_path, l.blake3, l.size_bytes, NULL, l.linked_at, l.synced_at \
             FROM design_code_links l \
             JOIN design_implement_receipts r ON r.id = l.receipt_id \
             WHERE r.artifact_id = ?1 ORDER BY r.created_at ASC, l.rel_path ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![artifact_id], |row| {
            let receipt = DesignImplementReceipt {
                id: row.get(0)?,
                artifact_id: row.get(1)?,
                session_id: row.get(2)?,
                code_dir: row.get(3)?,
                base_revision: row.get(4)?,
                harvest_revision: row.get(5)?,
                harvest_cursor: row.get(6)?,
                created_at: row.get(7)?,
                harvested_at: row.get(8)?,
            };
            let link = DesignCodeLink {
                id: row.get(9)?,
                receipt_id: row.get(10)?,
                rel_path: row.get(11)?,
                blake3: row.get(12)?,
                size_bytes: row.get(13)?,
                content_gz: row.get(14)?,
                linked_at: row.get(15)?,
                synced_at: row.get(16)?,
            };
            Ok((receipt, link))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 单独取一个 link 的 gzip 基线快照（列表查询不背 BLOB，diff 回放时按需取）。
    pub fn get_link_snapshot(&self, link_id: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.lock()?;
        let gz: Option<Vec<u8>> = conn
            .query_row(
                "SELECT content_gz FROM design_code_links WHERE id = ?1",
                rusqlite::params![link_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(gz)
    }

    pub fn delete_link(&self, id: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM design_code_links WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    /// 「新回执赢」去重：删同产物**其它**回执下同 `rel_path` 的旧 link。返回删除数。
    pub fn delete_links_same_path_in_other_receipts(
        &self,
        artifact_id: &str,
        keep_receipt_id: &str,
        rel_path: &str,
    ) -> Result<usize> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM design_code_links WHERE rel_path = ?3 AND receipt_id IN (
                 SELECT id FROM design_implement_receipts
                 WHERE artifact_id = ?1 AND id != ?2
             )",
            rusqlite::params![artifact_id, keep_receipt_id, rel_path],
        )?;
        Ok(n)
    }

    /// 某回执下 link 数（去重后 prune 已收割空回执用）。
    pub fn count_links_for_receipt(&self, receipt_id: &str) -> Result<i64> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM design_code_links WHERE receipt_id = ?1",
            rusqlite::params![receipt_id],
            |row| row.get(0),
        )?;
        Ok(n)
    }

    pub fn update_link_baseline(
        &self,
        id: i64,
        blake3: &str,
        size_bytes: i64,
        content_gz: Option<&[u8]>,
        synced_at: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE design_code_links
             SET blake3 = ?2, size_bytes = ?3, content_gz = ?4, synced_at = ?5 WHERE id = ?1",
            rusqlite::params![id, blake3, size_bytes, content_gz, synced_at],
        )?;
        Ok(())
    }

    /// 有 link 的回执涉及的 DISTINCT code_dir（watcher 建监听目标用）。
    pub fn list_linked_dirs(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT r.code_dir FROM design_implement_receipts r
             WHERE EXISTS (SELECT 1 FROM design_code_links l WHERE l.receipt_id = r.id)",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 某 code_dir 下全部 (project_id, artifact_id, rel_path)（watcher 事件路径过滤 + drift 定位）。
    pub fn links_index_for_dir(&self, code_dir: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT a.project_id, r.artifact_id, l.rel_path
             FROM design_code_links l
             JOIN design_implement_receipts r ON r.id = l.receipt_id
             JOIN design_artifacts a ON a.id = r.artifact_id
             WHERE r.code_dir = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![code_dir], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> (tempfile::TempDir, DesignDb) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = DesignDb::open(&dir.path().join("design.db")).expect("open");
        (dir, db)
    }

    /// 播一个 project + artifact（批注钉 FK 依赖）。返回 artifact id。
    fn seed_artifact(db: &DesignDb) -> String {
        db.create_project(&DesignProject {
            id: "p1".into(),
            title: "P".into(),
            description: None,
            color: None,
            default_system_id: None,
            ha_project_id: None,
            session_id: None,
            agent_id: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            artifact_count: 0,
            needs_review_count: 0,
            code_drift_count: 0,
            metadata: None,
            default_model: None,
            code_dir: None,
        })
        .unwrap();
        db.create_artifact(&DesignArtifact {
            id: "a1".into(),
            project_id: "p1".into(),
            title: "A".into(),
            kind: "web".into(),
            system_id: None,
            status: "ready".into(),
            viewport_w: None,
            viewport_h: None,
            current_version: 1,
            critique_score: None,
            thumbnail_path: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            metadata: None,
            folder: String::new(),
        })
        .unwrap();
        "a1".into()
    }

    #[test]
    fn comment_crud_roundtrip() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        let c = db
            .add_comment(
                &aid,
                Some(3),
                0.5,
                0.25,
                Some("h1"),
                Some("<h1>Hi</h1>"),
                "改大点",
                "t",
            )
            .unwrap();
        assert_eq!(c.oid, Some(3));
        assert!(!c.resolved);
        assert_eq!(db.list_comments(&aid).unwrap().len(), 1);
        // resolve
        assert!(db.set_comment_resolved(&aid, c.id, true).unwrap());
        assert!(db.get_comment(&aid, c.id).unwrap().unwrap().resolved);
        // relocate + detach (oid=None)
        assert!(db
            .update_comment_anchor(&aid, c.id, None, 0.1, 0.9)
            .unwrap());
        let got = db.get_comment(&aid, c.id).unwrap().unwrap();
        assert_eq!(got.oid, None);
        assert_eq!(got.rel_x, 0.1);
        // edit body
        assert!(db.update_comment_body(&aid, c.id, "再大点").unwrap());
        assert_eq!(db.get_comment(&aid, c.id).unwrap().unwrap().body, "再大点");
        // delete
        assert!(db.delete_comment(&aid, c.id).unwrap());
        assert!(db.list_comments(&aid).unwrap().is_empty());
    }

    fn seed_system(db: &DesignDb, id: &str) {
        db.upsert_system(&DesignSystemMeta {
            id: id.to_string(),
            name: id.to_string(),
            slug: id.to_string(),
            source: "user".to_string(),
            category: None,
            summary: None,
            thumbnail_path: None,
            swatches: Vec::new(),
            created_at: "t".to_string(),
            updated_at: "t".to_string(),
        })
        .unwrap();
    }

    #[test]
    fn code_binding_crud_and_cascade() {
        let (_d, db) = open_temp();
        seed_system(&db, "sys-a");
        let formats = vec!["css".to_string(), "ts".to_string()];
        let b = db
            .add_code_binding("sys-a", "/tmp/proj", "src/tokens", &formats, "t")
            .unwrap();
        assert_eq!(b.formats, formats);
        assert_eq!(b.last_synced_at, None);
        assert_eq!(db.list_code_bindings(Some("sys-a")).unwrap().len(), 1);
        // mark synced
        db.mark_binding_synced(b.id, "t2").unwrap();
        assert_eq!(
            db.get_code_binding(b.id).unwrap().unwrap().last_synced_at,
            Some("t2".to_string())
        );
        // delete
        assert!(db.delete_code_binding(b.id).unwrap());
        assert!(db.list_code_bindings(None).unwrap().is_empty());
        // cascade on system delete
        let b2 = db
            .add_code_binding("sys-a", "/tmp/p2", "", &formats, "t")
            .unwrap();
        db.delete_system("sys-a").unwrap();
        assert!(
            db.get_code_binding(b2.id).unwrap().is_none(),
            "系统删除应级联删绑定"
        );
    }

    #[test]
    fn comment_cascades_on_artifact_delete() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        db.add_comment(&aid, None, 0.0, 0.0, None, None, "x", "t")
            .unwrap();
        db.delete_artifact(&aid).unwrap();
        assert!(
            db.list_comments(&aid).unwrap().is_empty(),
            "artifact 删除应级联删批注"
        );
    }

    #[test]
    fn update_missing_comment_returns_false() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        assert!(!db.set_comment_resolved(&aid, 999, true).unwrap());
        assert!(!db.delete_comment(&aid, 999).unwrap());
    }

    #[test]
    fn version_provenance_roundtrips() {
        // B3-3：origin / prompt_summary 写入后经 list_versions 原样读回（列存在 + 映射正确）。
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        db.create_version(&DesignArtifactVersion {
            id: 0,
            artifact_id: aid.clone(),
            version_number: 2,
            message: Some("Generated".into()),
            critique_score: None,
            origin: Some("ai".into()),
            prompt_summary: Some("做一个定价页".into()),
            created_at: "t2".into(),
        })
        .unwrap();
        db.create_version(&DesignArtifactVersion {
            id: 0,
            artifact_id: aid.clone(),
            version_number: 3,
            message: Some("Visual edit".into()),
            critique_score: None,
            origin: Some("manual".into()),
            prompt_summary: None,
            created_at: "t3".into(),
        })
        .unwrap();
        let rows = db.list_versions(&aid).unwrap();
        // 倒序：v3 先于 v2。
        let v3 = rows.iter().find(|v| v.version_number == 3).unwrap();
        assert_eq!(v3.origin.as_deref(), Some("manual"));
        assert_eq!(v3.prompt_summary, None);
        let v2 = rows.iter().find(|v| v.version_number == 2).unwrap();
        assert_eq!(v2.origin.as_deref(), Some("ai"));
        assert_eq!(v2.prompt_summary.as_deref(), Some("做一个定价页"));
    }

    #[test]
    fn cleanup_protects_ai_milestones_and_current() {
        // W4-O：超上限淘汰优先删最旧 manual（微调自动保存），保留 ai 里程碑 + 当前(最新)版本。
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        let mk = |n: i64, origin: &str| DesignArtifactVersion {
            id: 0,
            artifact_id: aid.clone(),
            version_number: n,
            message: None,
            critique_score: None,
            origin: Some(origin.into()),
            prompt_summary: None,
            created_at: format!("t{n}"),
        };
        // v100=ai 里程碑，v101..v104=manual 微调，v105=ai 里程碑，v106=manual，v107=manual(最新/当前)。
        db.create_version(&mk(100, "ai")).unwrap();
        for n in 101..=104 {
            db.create_version(&mk(n, "manual")).unwrap();
        }
        db.create_version(&mk(105, "ai")).unwrap();
        db.create_version(&mk(106, "manual")).unwrap();
        db.create_version(&mk(107, "manual")).unwrap();

        db.cleanup_old_versions(&aid, 4).unwrap();
        let remaining: std::collections::HashSet<i64> = db
            .list_versions(&aid)
            .unwrap()
            .into_iter()
            .map(|v| v.version_number)
            .collect();
        // ai 里程碑保留：
        assert!(remaining.contains(&100), "ai milestone v100 must survive");
        assert!(remaining.contains(&105), "ai milestone v105 must survive");
        // 当前(最新)版本保留（即便是 manual）：
        assert!(
            remaining.contains(&107),
            "current version v107 must survive"
        );
        // 最旧的 manual 微调先被淘汰：
        assert!(
            !remaining.contains(&101),
            "oldest manual v101 must be evicted"
        );
    }

    #[test]
    fn share_upsert_is_idempotent_and_cascades() {
        // B7-1：同产物二次分享复用同一 token（链接不变）；resolve 回产物；删产物级联删分享。
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db); // project p1 + artifact a1
        let t1 = db.upsert_share(&aid, "tok_aaa", "t").unwrap();
        let t2 = db.upsert_share(&aid, "tok_bbb", "t").unwrap();
        assert_eq!(t1, "tok_aaa");
        assert_eq!(t2, "tok_aaa", "二次分享必须复用同一 token");
        assert_eq!(db.resolve_share("tok_aaa").unwrap().as_deref(), Some("a1"));
        assert_eq!(
            db.share_token_for_artifact(&aid).unwrap().as_deref(),
            Some("tok_aaa")
        );
        assert!(db.delete_share("tok_aaa").unwrap());
        assert!(db.resolve_share("tok_aaa").unwrap().is_none());
        // 级联：重建分享后删产物 → 分享行随 ON DELETE CASCADE 消失。
        db.upsert_share(&aid, "tok_ccc", "t").unwrap();
        db.delete_artifact(&aid).unwrap();
        assert!(
            db.resolve_share("tok_ccc").unwrap().is_none(),
            "删产物未级联删分享"
        );
    }

    #[test]
    fn project_needs_review_count_aggregates() {
        // B3-1：项目卡状态徽标读取时聚合 status='needs_review' 的产物数。
        let (_d, db) = open_temp();
        seed_artifact(&db); // a1 = ready
        for (id, status) in [
            ("a2", "needs_review"),
            ("a3", "needs_review"),
            ("a4", "failed"),
        ] {
            db.create_artifact(&DesignArtifact {
                id: id.into(),
                project_id: "p1".into(),
                title: id.into(),
                kind: "web".into(),
                system_id: None,
                status: status.into(),
                viewport_w: None,
                viewport_h: None,
                current_version: 1,
                critique_score: None,
                thumbnail_path: None,
                created_at: "t".into(),
                updated_at: "t".into(),
                metadata: None,
                folder: String::new(),
            })
            .unwrap();
        }
        let p = db.get_project("p1").unwrap().unwrap();
        assert_eq!(p.artifact_count, 4);
        assert_eq!(p.needs_review_count, 2);
    }

    fn seed_receipt(db: &DesignDb, id: &str, artifact_id: &str, session_id: &str) {
        db.create_implement_receipt(&DesignImplementReceipt {
            id: id.into(),
            artifact_id: artifact_id.into(),
            session_id: session_id.into(),
            code_dir: "/repo".into(),
            base_revision: None,
            harvest_revision: None,
            harvest_cursor: 0,
            created_at: id.into(), // 用 id 作 created_at 保证排序确定性
            harvested_at: None,
        })
        .unwrap();
    }

    #[test]
    fn receipt_and_link_crud_roundtrip() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        seed_receipt(&db, "r1", &aid, "sess1");
        assert_eq!(db.list_receipts_for_artifact(&aid).unwrap().len(), 1);
        assert_eq!(db.list_receipts_for_project("p1").unwrap().len(), 1);

        // upsert link（含 content_gz BLOB）。
        db.upsert_code_link("r1", "src/Button.tsx", "hashA", 42, Some(b"gzblob"), "t1")
            .unwrap();
        assert_eq!(db.count_links_for_receipt("r1").unwrap(), 1);
        let links = db.list_links_for_artifact(&aid).unwrap();
        assert_eq!(links.len(), 1);
        // list_links 刻意不背 content_gz（轻量态）。
        assert!(links[0].1.content_gz.is_none());
        assert_eq!(links[0].1.blake3, "hashA");
        // 快照单独取。
        assert_eq!(
            db.get_link_snapshot(links[0].1.id).unwrap().as_deref(),
            Some(&b"gzblob"[..])
        );

        // upsert 冲突刷新 hash + linked_at 保留首见。
        db.upsert_code_link("r1", "src/Button.tsx", "hashB", 50, None, "t2")
            .unwrap();
        let links = db.list_links_for_artifact(&aid).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].1.blake3, "hashB");
        assert_eq!(links[0].1.linked_at, "t1"); // 首见保留
        assert_eq!(links[0].1.synced_at, "t2"); // 刷新

        // 游标推进。
        db.update_receipt_harvest("r1", 99, Some("rev1"), "t3")
            .unwrap();
        let r = &db.list_receipts_for_artifact(&aid).unwrap()[0];
        assert_eq!(r.harvest_cursor, 99);
        assert_eq!(r.harvest_revision.as_deref(), Some("rev1"));

        // linked dirs / index。
        assert_eq!(db.list_linked_dirs().unwrap(), vec!["/repo".to_string()]);
        let idx = db.links_index_for_dir("/repo").unwrap();
        assert_eq!(
            idx,
            vec![("p1".into(), aid.clone(), "src/Button.tsx".into())]
        );
    }

    #[test]
    fn receipt_link_cascade_on_artifact_and_project_delete() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        seed_receipt(&db, "r1", &aid, "sess1");
        db.upsert_code_link("r1", "a.ts", "h", 1, None, "t")
            .unwrap();
        // 删产物 → 回执 + link 级联消失。
        db.delete_artifact(&aid).unwrap();
        assert_eq!(db.list_receipts_for_artifact(&aid).unwrap().len(), 0);
        assert_eq!(db.count_links_for_receipt("r1").unwrap(), 0);
        assert!(db.list_linked_dirs().unwrap().is_empty());
    }

    #[test]
    fn new_receipt_wins_dedup_same_path() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db);
        seed_receipt(&db, "r1", &aid, "sess1");
        seed_receipt(&db, "r2", &aid, "sess2");
        db.upsert_code_link("r1", "shared.ts", "old", 1, None, "t")
            .unwrap();
        db.upsert_code_link("r2", "shared.ts", "new", 2, None, "t")
            .unwrap();
        // 新回执赢：删 r1（其它回执）下同路径旧 link。
        let removed = db
            .delete_links_same_path_in_other_receipts(&aid, "r2", "shared.ts")
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(db.count_links_for_receipt("r1").unwrap(), 0);
        assert_eq!(db.count_links_for_receipt("r2").unwrap(), 1);
    }

    #[test]
    fn last_opened_roundtrip_and_ordering() {
        let (_d, db) = open_temp();
        seed_artifact(&db); // p1 / a1
        assert!(db.last_opened().unwrap().is_none());
        db.set_last_opened("p1", "a1", "2026-07-15T10:00:00Z")
            .unwrap();
        let got = db.last_opened().unwrap().unwrap();
        assert_eq!(
            got,
            ("p1".into(), "a1".into(), "2026-07-15T10:00:00Z".into())
        );
        // set_last_opened 不动 updated_at（浏览≠编辑）。
        assert_eq!(db.get_project("p1").unwrap().unwrap().updated_at, "t");
    }

    #[test]
    fn set_artifact_metadata_quiet_does_not_bump_updated_at() {
        let (_d, db) = open_temp();
        let aid = seed_artifact(&db); // updated_at = "t"
        db.set_artifact_metadata_quiet(&aid, Some(r#"{"codeDrift":{"files":[]}}"#))
            .unwrap();
        let a = db.get_artifact(&aid).unwrap().unwrap();
        assert_eq!(a.updated_at, "t"); // 不动
        assert!(a.metadata.as_deref().unwrap().contains("codeDrift"));
    }

    #[test]
    fn project_code_drift_count_aggregates() {
        let (_d, db) = open_temp();
        seed_artifact(&db); // a1，无 drift
                            // a2 带 codeDrift；a3 metadata 非法 JSON（json_valid 兜底不炸）。
        for (id, meta) in [
            (
                "a2",
                Some(r#"{"codeDrift":{"files":[{"path":"x","state":"modified"}]}}"#),
            ),
            ("a3", Some("not json{{")),
        ] {
            db.create_artifact(&DesignArtifact {
                id: id.into(),
                project_id: "p1".into(),
                title: id.into(),
                kind: "web".into(),
                system_id: None,
                status: "ready".into(),
                viewport_w: None,
                viewport_h: None,
                current_version: 1,
                critique_score: None,
                thumbnail_path: None,
                created_at: "t".into(),
                updated_at: "t".into(),
                metadata: meta.map(str::to_string),
                folder: String::new(),
            })
            .unwrap();
        }
        let p = db.get_project("p1").unwrap().unwrap();
        assert_eq!(p.code_drift_count, 1); // 仅 a2
        assert_eq!(p.artifact_count, 3);
    }
}
