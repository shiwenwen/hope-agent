//! Git 控制平面的 kernel 面：`git_operation_runs` 簿记（表 + 行类型 +
//! 类型化查询）与仓库指纹原语。操作面（index/branch/commit/push/PR/
//! handoff 与启动期对账）在 `ha-vcs::git_control`。
//!
//! 类型随表下沉：`git_operation_runs` 落在 sessions.db，特征 crate 不持
//! raw conn，故 DDL、行类型与全部 SQL 留 kernel；ha-vcs 经这些 pub 方法
//! 读写簿记。`repository_revision`（工作树 BLAKE3 指纹）与其 git 子进程
//! 小簇留 kernel 供 ha-design（code_sync 基线指纹）与 ha-vcs 共用。

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::session::SessionDB;

pub fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS git_operation_runs (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            status TEXT NOT NULL,
            stage TEXT NOT NULL,
            before_head TEXT,
            after_head TEXT,
            result_json TEXT,
            error_code TEXT,
            error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_git_operation_session
            ON git_operation_runs(session_id, updated_at DESC);",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationRun {
    pub id: String,
    pub session_id: String,
    pub operation: String,
    pub status: String,
    pub stage: String,
    pub before_head: Option<String>,
    pub after_head: Option<String>,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

// ── git 子进程小簇（pub：ha-vcs 操作面与 repository_revision 共用）──

/// 构造隔离环境的 git 命令（禁 prompt、隐藏控制台、剥离仓库环境变量）。
pub fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    crate::filesystem::isolate_repository_env(&mut cmd);
    cmd.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::hide_console(&mut cmd);
    cmd
}

pub fn output_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        stderr.trim().to_string()
    }
}

pub fn run_git(root: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(root, args).output()?;
    if !output.status.success() {
        bail!(
            "git_failed: git {} failed: {}",
            args.join(" "),
            output_error(&output)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn run_git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_command(root, args).output()?;
    if !output.status.success() {
        bail!("git_failed: {}", output_error(&output));
    }
    Ok(output.stdout)
}

pub fn untracked_paths(root: &Path) -> Result<Vec<String>> {
    Ok(
        run_git_bytes(root, &["ls-files", "--others", "--exclude-standard", "-z"])?
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect(),
    )
}

/// 工作树状态 BLAKE3 指纹（HEAD + porcelain + diff + untracked 元数据）。
/// 消费方：ha-design `code_sync` 基线校验、ha-vcs `stale_snapshot` 守卫。
pub fn repository_revision(root: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    for args in [
        vec!["rev-parse", "HEAD"],
        vec!["status", "--porcelain=v1", "-z"],
        vec!["diff", "--binary"],
        vec!["diff", "--binary", "--cached"],
    ] {
        match run_git_bytes(root, &args) {
            Ok(bytes) => hasher.update(&bytes),
            Err(err) => hasher.update(err.to_string().as_bytes()),
        };
        hasher.update(&[0]);
    }
    for path in untracked_paths(root)? {
        hasher.update(path.as_bytes());
        if let Ok(meta) = fs::symlink_metadata(root.join(&path)) {
            hasher.update(&meta.len().to_le_bytes());
            if let Ok(modified) = meta.modified().and_then(|v| {
                v.duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)
            }) {
                hasher.update(&modified.as_nanos().to_le_bytes());
            }
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

// ── git_operation_runs 类型化查询（pub 方法：ha-vcs 操作面消费）──────

impl SessionDB {
    pub fn get_git_operation_run(&self, id: &str) -> Result<Option<GitOperationRun>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.query_row("SELECT id, session_id, operation, status, stage, before_head, after_head, result_json, error_code, error_message, created_at, updated_at, completed_at FROM git_operation_runs WHERE id=?1", params![id], row_to_operation).optional().map_err(Into::into)
    }

    /// ha-vcs 操作面建档（status='running', stage='executing'）。
    pub fn insert_git_operation_run(
        &self,
        id: &str,
        session_id: &str,
        operation: &str,
        before_head: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("INSERT INTO git_operation_runs (id,session_id,operation,status,stage,before_head,created_at,updated_at) VALUES (?1,?2,?3,'running','executing',?4,?5,?5)", params![id,session_id,operation,before_head,now])?;
        Ok(())
    }

    /// ha-vcs 操作面完结。`after_head` 取自结果的 head，`result_json`
    /// 是结果的 JSON 序列化（`GitMutationResult` 在 ha-vcs，故由调用方
    /// 序列化后传入）。
    pub fn complete_git_operation_run(
        &self,
        id: &str,
        after_head: Option<&str>,
        result_json: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("UPDATE git_operation_runs SET status='completed',stage='completed',after_head=?2,result_json=?3,updated_at=?4,completed_at=?4 WHERE id=?1", params![id,after_head,result_json,now])?;
        Ok(())
    }

    pub fn fail_git_operation_run(&self, id: &str, code: &str, message: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute("UPDATE git_operation_runs SET status='failed',stage='failed',error_code=?2,error_message=?3,updated_at=?4,completed_at=?4 WHERE id=?1", params![id,code,message,now])?;
        Ok(())
    }

    pub fn set_git_operation_stage(&self, id: &str, stage: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE git_operation_runs SET stage=?2,updated_at=?3 WHERE id=?1 AND status='running'",
            params![id, stage, now],
        )?;
        Ok(())
    }

    /// handoff 收尾：该会话全部非 archived 管理工作树标回 active。
    pub fn mark_handoff_worktrees_active(&self, session_id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE managed_worktrees SET state='active',handed_off_at=?2,updated_at=?2
             WHERE session_id=?1 AND state!='archived'",
            params![session_id, now],
        )?;
        Ok(())
    }

    /// 启动期对账用：列出全部 status='running' 的操作
    /// `(id, session_id, operation, stage)`（对账编排在 ha-vcs，经
    /// `vcs_hooks::git_ops_reconciler` 回调）。
    pub fn list_running_git_operations(&self) -> Result<Vec<(String, String, String, String)>> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let mut statement = conn.prepare(
            "SELECT id,session_id,operation,stage FROM git_operation_runs WHERE status='running'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 启动期对账用：单行标记 interrupted（error_code='process_restarted'）。
    pub fn mark_git_operation_interrupted(&self, id: &str, message: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE git_operation_runs SET status='interrupted',stage='interrupted',
                error_code='process_restarted',error_message=?2,updated_at=?3,completed_at=?3
             WHERE id=?1",
            params![id, message, now],
        )?;
        Ok(())
    }

    /// git 分支变更后同步 managed_worktrees 行的 git_branch 字段。
    pub fn update_managed_worktree_git_branch_for_path(
        &self,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let path = path.to_string_lossy();
        let conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        conn.execute(
            "UPDATE managed_worktrees SET git_branch=?2,updated_at=?3 WHERE path=?1",
            params![path.as_ref(), branch, now],
        )?;
        Ok(())
    }
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitOperationRun> {
    let result_json: Option<String> = row.get(7)?;
    Ok(GitOperationRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        operation: row.get(2)?,
        status: row.get(3)?,
        stage: row.get(4)?,
        before_head: row.get(5)?,
        after_head: row.get(6)?,
        result: result_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        error_code: row.get(8)?,
        error_message: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}
