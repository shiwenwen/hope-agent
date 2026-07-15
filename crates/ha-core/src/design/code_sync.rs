//! code→design 回灌（stale 检测 + 引导更新）。
//!
//! `implement_to_code` 把设计稿落成真实代码后，代码侧的后续改动应让设计空间「知道」——
//! 否则 coding 与 design 交替时产物漂移。数据链路 **回执 → 收割 → 比对 → 三动作**：
//!
//! 1. **回执**（`design_implement_receipts`）：一次「实现到代码」的锚点（产物 / 承接会话 /
//!    落地目录 / git 基线 / 已收割的会话 message 游标）。
//! 2. **收割**（harvest）：从承接会话的 `write`/`edit`/`apply_patch` 工具元数据**增量**提取
//!    「产物落地文件」→ 逐文件 BLAKE3 + gzip 快照存 `design_code_links`（基线）。游标幂等，
//!    实现会话自己的后续改动被吸收为基线，**只有会话之外的外部改动会被判为漂移**。
//! 3. **比对**（`check_code_drift`）：逐 link 重算现磁盘 BLAKE3，缺失=deleted、不等=modified，
//!    结果写产物 `metadata.codeDrift`（照 [`selfcheck::merge_into_metadata`] 模式只动本键、**不占
//!    status 列、不 bump updated_at**），翻转才 emit `design:code_drift`。
//! 4. **三动作**：查看变更（`drift_changes` 复用 `tools::diff_util` + 前端 `DiffPanel`）/ 带到
//!    设计对话（`quote` pack）/ 标为已同步（`mark_synced` 重置基线 + 清标）。
//!
//! 实时监听见 [`super::code_watcher`]。红线：只读已授权绑定目录、越界路径丢弃；不写用户代码。

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::db::{DesignArtifact, DesignDb, DesignImplementReceipt};
use super::service::get_design_db;
use crate::session::{SessionDB, SessionMessage};

/// 单文件 gzip 快照上限（原文字节）；超限或二进制不存快照（仍标 stale，UI 降级不出内嵌 diff）。
const SNAPSHOT_MAX: usize = 512 * 1024;
/// `metadata.codeDrift.files` 截断上限（避免病态膨胀）。
const DRIFT_FILES_MAX: usize = 50;
/// 「带到设计对话」quote pack 每文件 / 总预算（照 `service::IMPLEMENT_PART_MAX` 先例）。
const DRIFT_QUOTE_FILE_MAX: usize = 4 * 1024;
const DRIFT_QUOTE_TOTAL_MAX: usize = 24 * 1024;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn emit(event: &str, payload: Value) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(event, payload);
    }
}

// ── metadata.codeDrift 形状 ────────────────────────────────────────

/// 产物 `metadata.codeDrift` 键的形状。
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeDriftFlag {
    pub files: Vec<CodeDriftFile>,
    pub checked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeDriftFile {
    pub path: String,
    /// `"modified"` | `"deleted"`.
    pub state: String,
}

/// 照 [`selfcheck::merge_into_metadata`]：只动 `codeDrift` 键、保留其它键；空对象回 None。
pub fn merge_code_drift_into_metadata(
    existing: Option<&str>,
    flag: Option<&CodeDriftFlag>,
) -> Option<String> {
    let mut obj = existing
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    match flag {
        Some(f) => {
            if let Ok(v) = serde_json::to_value(f) {
                obj.insert("codeDrift".to_string(), v);
            }
        }
        None => {
            obj.remove("codeDrift");
        }
    }
    if obj.is_empty() {
        None
    } else {
        serde_json::to_string(&Value::Object(obj)).ok()
    }
}

pub fn parse_code_drift(metadata: Option<&str>) -> Option<CodeDriftFlag> {
    let v: Value = serde_json::from_str(metadata?).ok()?;
    serde_json::from_value(v.get("codeDrift")?.clone()).ok()
}

/// 语义相等：只比 (path, state) 集合（忽略 checked_at），避免无实变的写盘 / emit 抖动。
fn flags_equal(a: &Option<CodeDriftFlag>, b: &Option<CodeDriftFlag>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            let mut xs: Vec<(&str, &str)> = x
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.state.as_str()))
                .collect();
            let mut ys: Vec<(&str, &str)> = y
                .files
                .iter()
                .map(|f| (f.path.as_str(), f.state.as_str()))
                .collect();
            xs.sort_unstable();
            ys.sort_unstable();
            xs == ys
        }
        _ => false,
    }
}

// ── 回执创建（implement_to_code 尾部调用）──────────────────────────

/// implement 落地成功后建回执（基线 revision 尽力而为，非 git 目录 = None）。
pub(crate) fn create_receipt_for_implement(
    artifact_id: &str,
    session_id: &str,
    code_dir: &str,
) -> Result<()> {
    let db = get_design_db()?;
    let base_revision = crate::git_control::repository_revision(Path::new(code_dir)).ok();
    let r = DesignImplementReceipt {
        id: uuid::Uuid::new_v4().to_string(),
        artifact_id: artifact_id.to_string(),
        session_id: session_id.to_string(),
        code_dir: code_dir.to_string(),
        base_revision,
        harvest_revision: None,
        harvest_cursor: 0,
        created_at: now(),
        harvested_at: None,
    };
    db.create_implement_receipt(&r)?;
    Ok(())
}

/// 转发 watcher 索引重建（收割/同步/建回执/删产物/绑定变更后调）。
pub(crate) fn refresh_watchers() {
    super::code_watcher::refresh_all();
}

// ── 收割（harvest）──────────────────────────────────────────────

/// 从会话消息切片提取 `write`/`edit`/`apply_patch` 的落地路径（镜像
/// `session/artifacts.rs` 的 `file_change`/`file_changes` 解析——**不含** file_read / media；
/// 改一处改两处的对齐仅限「认哪些 kind + path 字段」，此处不做去重/排序）。
fn extract_written_paths(messages: &[SessionMessage]) -> Vec<String> {
    let mut out = Vec::new();
    for msg in messages {
        let Some(meta) = msg
            .tool_metadata
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        else {
            continue;
        };
        match meta.get("kind").and_then(Value::as_str) {
            Some("file_change") => {
                if let Some(p) = meta.get("path").and_then(Value::as_str) {
                    out.push(p.to_string());
                }
            }
            Some("file_changes") => {
                if let Some(changes) = meta.get("changes").and_then(Value::as_array) {
                    for c in changes {
                        if let Some(p) = c.get("path").and_then(Value::as_str) {
                            out.push(p.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// canonical `root` 下的相对路径（正斜杠）；`abs` 越界返回 None。文件本体可能已删，故只
/// canonicalize 父目录（解 symlink）后做 containment。
fn rel_within(root: &Path, abs: &Path) -> Option<String> {
    let parent = abs.parent()?;
    let file_name = abs.file_name()?;
    let parent_canon = parent.canonicalize().ok()?;
    if !parent_canon.starts_with(root) {
        return None;
    }
    let rel_parent = parent_canon.strip_prefix(root).ok()?;
    let rel = rel_parent.join(file_name);
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn gzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(bytes)?;
    e.finish()
}

fn gunzip(gz: &[u8]) -> std::io::Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut d = GzDecoder::new(gz);
    let mut out = Vec::new();
    d.read_to_end(&mut out)?;
    Ok(out)
}

/// 读文件 → (BLAKE3 hex, size, gzip 快照 or None)。文件不存在返回 None。
fn read_and_snapshot(abs: &Path) -> Option<(String, i64, Option<Vec<u8>>)> {
    let bytes = std::fs::read(abs).ok()?;
    let size = bytes.len() as i64;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let is_binary = bytes.iter().take(8192).any(|&b| b == 0);
    let gz = if !is_binary && bytes.len() <= SNAPSHOT_MAX {
        gzip(&bytes).ok()
    } else {
        None
    };
    Some((hash, size, gz))
}

/// 增量收割一条回执。游标幂等；会话已删且无 links → 删回执。返回是否有新/刷新 link。
fn harvest_receipt(db: &DesignDb, sdb: &SessionDB, r: &DesignImplementReceipt) -> Result<bool> {
    // 会话已删：有 links 则冻结（drift 仍照查已有 links），无 links 则删回执。
    if sdb.get_session(&r.session_id)?.is_none() {
        if db.count_links_for_receipt(&r.id)? == 0 {
            db.delete_receipt(&r.id)?;
            crate::app_warn!(
                "design",
                "code_sync",
                "implement session {} gone with no harvested files; dropped receipt {}",
                r.session_id,
                r.id
            );
        }
        return Ok(false);
    }

    let code_dir_canon = Path::new(&r.code_dir)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&r.code_dir).to_path_buf());

    let mut cursor = r.harvest_cursor;
    let mut any_new = false;
    loop {
        let (batch, more) = sdb.load_session_messages_after(&r.session_id, cursor, 500)?;
        if batch.is_empty() {
            break;
        }
        let max_id = batch.iter().map(|m| m.id).max().unwrap_or(cursor);
        for p in extract_written_paths(&batch) {
            let abs = Path::new(&p);
            let Some(rel) = rel_within(&code_dir_canon, abs) else {
                // 落地路径不在绑定目录内——不入库（防越界；实现会话本可能改仓库外文件）。
                continue;
            };
            if let Some((hash, size, gz)) = read_and_snapshot(abs) {
                db.upsert_code_link(&r.id, &rel, &hash, size, gz.as_deref(), &now())?;
                // 「新回执赢」：删同产物其它回执下同 rel_path 的旧 link。
                db.delete_links_same_path_in_other_receipts(&r.artifact_id, &r.id, &rel)?;
                any_new = true;
            }
            // 文件此刻不存在（转瞬文件 / 会话删了它）→ 跳过不建 link。
        }
        cursor = max_id;
        if !more {
            break;
        }
    }

    if cursor != r.harvest_cursor || any_new || r.harvested_at.is_none() {
        let rev = crate::git_control::repository_revision(&code_dir_canon).ok();
        db.update_receipt_harvest(&r.id, cursor, rev.as_deref(), &now())?;
    }
    Ok(any_new)
}

/// 去重后 prune：被更新回执取代且已收割空的旧回执删除（防 content_gz 累积）。
fn prune_superseded_empty_receipts(db: &DesignDb, artifact_id: &str) -> Result<()> {
    let receipts = db.list_receipts_for_artifact(artifact_id)?; // created_at ASC
    if receipts.len() <= 1 {
        return Ok(());
    }
    let newest_id = receipts.last().map(|r| r.id.clone());
    for r in &receipts {
        if Some(&r.id) != newest_id.as_ref()
            && r.harvested_at.is_some()
            && db.count_links_for_receipt(&r.id)? == 0
        {
            db.delete_receipt(&r.id)?;
        }
    }
    Ok(())
}

// ── 比对（check）────────────────────────────────────────────────

/// 单产物级：逐 link 比对现磁盘，写 metadata（翻转才写+emit），返回状态。
fn compute_drift_for_artifact(
    db: &DesignDb,
    artifact_id: &str,
) -> Result<Option<ArtifactDriftStatus>> {
    let Some(artifact) = db.get_artifact(artifact_id)? else {
        return Ok(None);
    };
    let links = db.list_links_for_artifact(artifact_id)?; // (receipt, link)，无 content_gz
                                                          // 同 rel_path 去重：created_at ASC 排列，后者（更新回执）赢。
    let mut by_path: BTreeMap<String, (String, String)> = BTreeMap::new(); // rel → (code_dir, baseline hash)
    for (r, l) in &links {
        by_path.insert(l.rel_path.clone(), (r.code_dir.clone(), l.blake3.clone()));
    }
    let mut files = Vec::new();
    for (rel, (code_dir, baseline)) in &by_path {
        if files.len() >= DRIFT_FILES_MAX {
            break;
        }
        let abs = Path::new(code_dir).join(rel);
        match std::fs::read(&abs) {
            Ok(bytes) => {
                let cur = blake3::hash(&bytes).to_hex().to_string();
                if &cur != baseline {
                    files.push(CodeDriftFile {
                        path: rel.clone(),
                        state: "modified".to_string(),
                    });
                }
            }
            Err(_) => {
                // 目录整体失效（外置盘未挂载 / 仓库删）→ 不假 stale（绑定级 stale 另标红）。
                if Path::new(code_dir).is_dir() {
                    files.push(CodeDriftFile {
                        path: rel.clone(),
                        state: "deleted".to_string(),
                    });
                }
            }
        }
    }

    let stale = !files.is_empty();
    let flag = stale.then(|| CodeDriftFlag {
        files: files.clone(),
        checked_at: now(),
        session_id: links.last().map(|(r, _)| r.session_id.clone()),
    });
    let old_flag = parse_code_drift(artifact.metadata.as_deref());
    if !flags_equal(&old_flag, &flag) {
        let new_meta = merge_code_drift_into_metadata(artifact.metadata.as_deref(), flag.as_ref());
        db.set_artifact_metadata_quiet(artifact_id, new_meta.as_deref())?;
        emit(
            "design:code_drift",
            json!({ "projectId": artifact.project_id, "artifactId": artifact_id, "stale": stale }),
        );
    }
    Ok(Some(ArtifactDriftStatus {
        artifact_id: artifact_id.to_string(),
        stale,
        files,
    }))
}

/// 检查入口（打开项目/产物、手动、watcher 共用）：先收割后比对。
pub fn check_code_drift(
    project_id: &str,
    artifact_id: Option<&str>,
) -> Result<Vec<ArtifactDriftStatus>> {
    let db = get_design_db()?;
    let receipts = match artifact_id {
        Some(aid) => db.list_receipts_for_artifact(aid)?,
        None => db.list_receipts_for_project(project_id)?,
    };
    if receipts.is_empty() {
        return Ok(Vec::new()); // 无回执 = 未做过 implement，O(1) 空返，零开销。
    }

    let mut any_new = false;
    if let Some(sdb) = crate::globals::get_session_db() {
        for r in &receipts {
            match harvest_receipt(db, sdb, r) {
                Ok(new) => any_new |= new,
                Err(e) => crate::app_warn!(
                    "design",
                    "code_sync",
                    "harvest receipt {} failed: {}",
                    r.id,
                    e
                ),
            }
        }
    }

    let artifact_ids: Vec<String> = match artifact_id {
        Some(aid) => vec![aid.to_string()],
        None => receipts
            .iter()
            .map(|r| r.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    for aid in &artifact_ids {
        let _ = prune_superseded_empty_receipts(db, aid);
    }

    let mut out = Vec::new();
    for aid in &artifact_ids {
        if let Some(status) = compute_drift_for_artifact(db, aid)? {
            out.push(status);
        }
    }
    if any_new {
        refresh_watchers();
    }
    Ok(out)
}

/// watcher debounce 回调：某 code_dir 下全部关联产物重算 drift（外部改动，不涉会话故不收割）。
pub(crate) fn check_drift_for_dir(code_dir: &str) -> Result<()> {
    let db = get_design_db()?;
    let artifacts: BTreeSet<String> = db
        .links_index_for_dir(code_dir)?
        .into_iter()
        .map(|(_proj, art, _rel)| art)
        .collect();
    for aid in &artifacts {
        let _ = compute_drift_for_artifact(db, aid);
    }
    Ok(())
}

// ── 三动作 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDriftStatus {
    pub artifact_id: String,
    pub stale: bool,
    pub files: Vec<CodeDriftFile>,
}

/// `FileChangeMetadata` 兼容形状（喂前端 `DiffPanel`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftFileChange {
    pub kind: String, // 恒 "file_change"
    pub path: String,
    pub action: String, // "edit" | "delete"
    pub lines_added: u32,
    pub lines_removed: u32,
    pub before: Option<String>,
    pub after: Option<String>,
    pub language: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDriftChanges {
    pub code_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    pub files: Vec<DriftFileChange>,
    /// `<code_drift>` 结构化 pack（带到设计对话让 AI 据此更新产物）。
    pub quote: String,
}

/// 查看代码变更 + 组「带到对话」quote pack。
pub fn drift_changes(artifact_id: &str) -> Result<CodeDriftChanges> {
    use crate::tools::diff_util::{compute_line_delta, detect_language, truncate_for_metadata};
    let db = get_design_db()?;
    let links = db.list_links_for_artifact(artifact_id)?;
    let mut by_path: BTreeMap<String, (i64, String, String)> = BTreeMap::new(); // rel → (link id, code_dir, hash)
    let mut code_dir_out = String::new();
    let mut base_rev = None;
    for (r, l) in &links {
        by_path.insert(
            l.rel_path.clone(),
            (l.id, r.code_dir.clone(), l.blake3.clone()),
        );
        code_dir_out = r.code_dir.clone();
        base_rev = r.base_revision.clone();
    }

    let mut files = Vec::new();
    let mut quote = String::from("<code_drift>\n");
    quote.push_str(&format!(
        "artifact_id={artifact_id}\ncode_dir={code_dir_out}\n"
    ));
    quote.push_str("以下是绑定代码仓库里、由本设计稿实现出的文件的当前内容（设计稿落地后代码侧已改动）。请据此更新当前打开的设计稿，使其反映最新代码，保持设计意图与其余内容不变。\n\n");
    let mut total = quote.len();

    for (rel, (link_id, code_dir, baseline)) in &by_path {
        let abs = Path::new(code_dir).join(rel);
        let before = db
            .get_link_snapshot(*link_id)?
            .and_then(|gz| gunzip(&gz).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        match std::fs::read(&abs) {
            Ok(bytes) => {
                let cur_hash = blake3::hash(&bytes).to_hex().to_string();
                if &cur_hash == baseline {
                    continue; // 未漂移
                }
                let after_raw = String::from_utf8_lossy(&bytes).into_owned();
                let before_str = before.clone().unwrap_or_default();
                let (added, removed) = compute_line_delta(&before_str, &after_raw);
                let (before_t, bt) = truncate_for_metadata(&before_str);
                let (after_t, at) = truncate_for_metadata(&after_raw);
                files.push(DriftFileChange {
                    kind: "file_change".to_string(),
                    path: rel.clone(),
                    action: "edit".to_string(),
                    lines_added: added,
                    lines_removed: removed,
                    before: before.as_ref().map(|_| before_t),
                    after: Some(after_t),
                    language: detect_language(rel).to_string(),
                    truncated: bt || at,
                });
                if total < DRIFT_QUOTE_TOTAL_MAX {
                    let snippet = crate::util::truncate_utf8(&after_raw, DRIFT_QUOTE_FILE_MAX);
                    let block = format!("## {rel} [modified]\n{snippet}\n\n");
                    total += block.len();
                    quote.push_str(&block);
                }
            }
            Err(_) => {
                if !Path::new(code_dir).is_dir() {
                    continue; // 目录失效，非文件删除
                }
                files.push(DriftFileChange {
                    kind: "file_change".to_string(),
                    path: rel.clone(),
                    action: "delete".to_string(),
                    lines_added: 0,
                    lines_removed: before
                        .as_ref()
                        .map(|b| b.lines().count() as u32)
                        .unwrap_or(0),
                    before: before.map(|b| truncate_for_metadata(&b).0),
                    after: None,
                    language: detect_language(rel).to_string(),
                    truncated: false,
                });
                if total < DRIFT_QUOTE_TOTAL_MAX {
                    let block = format!("## {rel} [deleted]\n（该文件已删除）\n\n");
                    total += block.len();
                    quote.push_str(&block);
                }
            }
        }
    }
    quote.push_str("</code_drift>");
    Ok(CodeDriftChanges {
        code_dir: code_dir_out,
        base_revision: base_rev,
        files,
        quote,
    })
}

/// 标为已同步：逐 link 重置基线为当前磁盘态（文件已删则删 link），清 `codeDrift` 键 + emit。
pub fn mark_synced(artifact_id: &str) -> Result<DesignArtifact> {
    let db = get_design_db()?;
    let links = db.list_links_for_artifact(artifact_id)?;
    for (r, l) in &links {
        let abs = Path::new(&r.code_dir).join(&l.rel_path);
        match read_and_snapshot(&abs) {
            Some((hash, size, gz)) => {
                db.update_link_baseline(l.id, &hash, size, gz.as_deref(), &now())?;
            }
            None => {
                // 文件确实删了 → 丢弃该 link（不再追踪）；目录失效则保留（转瞬态）。
                if Path::new(&r.code_dir).is_dir() {
                    db.delete_link(l.id)?;
                }
            }
        }
    }
    let artifact = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    let new_meta = merge_code_drift_into_metadata(artifact.metadata.as_deref(), None);
    if new_meta != artifact.metadata {
        db.set_artifact_metadata_quiet(artifact_id, new_meta.as_deref())?;
    }
    emit(
        "design:code_drift",
        json!({ "projectId": artifact.project_id, "artifactId": artifact_id, "stale": false }),
    );
    refresh_watchers();
    db.get_artifact(artifact_id)?
        .with_context(|| format!("artifact vanished: {artifact_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::MessageRole;

    fn msg_meta(id: i64, meta: Option<&str>) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "s".into(),
            role: MessageRole::Tool,
            content: String::new(),
            timestamp: String::new(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: Some("edit".into()),
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: meta.map(str::to_string),
            stream_status: None,
        }
    }

    #[test]
    fn extract_written_paths_covers_change_shapes() {
        let msgs = vec![
            msg_meta(
                1,
                Some(r#"{"kind":"file_change","path":"/repo/a.ts","action":"edit"}"#),
            ),
            msg_meta(
                2,
                Some(
                    r#"{"kind":"file_changes","changes":[
                        {"kind":"file_change","path":"/repo/b.ts","action":"create"},
                        {"kind":"file_change","path":"/repo/c.ts","action":"delete"}]}"#,
                ),
            ),
            // file_read 忽略。
            msg_meta(
                3,
                Some(r#"{"kind":"file_read","path":"/repo/z.ts","lines":9}"#),
            ),
            // 畸形 metadata 忽略。
            msg_meta(4, Some("not json{{")),
            msg_meta(5, None),
        ];
        let paths = extract_written_paths(&msgs);
        assert_eq!(paths, vec!["/repo/a.ts", "/repo/b.ts", "/repo/c.ts"]);
    }

    #[test]
    fn rel_within_containment() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/Button.tsx"), b"x").unwrap();

        // 内部文件 → 相对路径。
        assert_eq!(
            rel_within(&root, &root.join("src/Button.tsx")).as_deref(),
            Some("src/Button.tsx")
        );
        // 根下直接文件。
        std::fs::write(root.join("top.ts"), b"x").unwrap();
        assert_eq!(
            rel_within(&root, &root.join("top.ts")).as_deref(),
            Some("top.ts")
        );
        // 外部路径 → None。
        let outside = tempfile::tempdir().unwrap();
        let outside = outside.path().canonicalize().unwrap();
        std::fs::write(outside.join("evil.ts"), b"x").unwrap();
        assert_eq!(rel_within(&root, &outside.join("evil.ts")), None);
        // `..` 逃逸 → None（父目录 canonicalize 到外部）。
        assert_eq!(rel_within(&root, &root.join("../escape.ts")), None);
    }

    #[test]
    fn merge_parse_and_flags_equal() {
        let flag = CodeDriftFlag {
            files: vec![CodeDriftFile {
                path: "a.ts".into(),
                state: "modified".into(),
            }],
            checked_at: "t".into(),
            session_id: Some("s".into()),
        };
        // 合并进已有 metadata：保留其它键。
        let meta =
            merge_code_drift_into_metadata(Some(r#"{"selfCheck":{"flag":"ok"}}"#), Some(&flag));
        let meta = meta.unwrap();
        assert!(meta.contains("selfCheck"));
        assert!(meta.contains("codeDrift"));
        // 解析回来。
        let parsed = parse_code_drift(Some(&meta)).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].state, "modified");
        // 清标：只删 codeDrift，selfCheck 留存。
        let cleared = merge_code_drift_into_metadata(Some(&meta), None).unwrap();
        assert!(cleared.contains("selfCheck"));
        assert!(!cleared.contains("codeDrift"));
        // 清标后仅剩 codeDrift → 回 None（空对象）。
        assert_eq!(
            merge_code_drift_into_metadata(Some(r#"{"codeDrift":{}}"#), None),
            None
        );

        // flags_equal 忽略 checked_at，只看 (path,state) 集。
        let mut f2 = flag.clone();
        f2.checked_at = "different".into();
        assert!(flags_equal(&Some(flag.clone()), &Some(f2)));
        assert!(!flags_equal(&Some(flag), &None));
        assert!(flags_equal(&None, &None));
    }

    #[test]
    fn gzip_roundtrip_and_snapshot() {
        let data = b"hello \xE4\xB8\x96\xE7\x95\x8C\nline2\n";
        let gz = gzip(data).unwrap();
        assert_eq!(gunzip(&gz).unwrap(), data);

        let dir = tempfile::tempdir().unwrap();
        // 文本文件：有快照。
        let txt = dir.path().join("a.ts");
        std::fs::write(&txt, data).unwrap();
        let (hash, size, gzo) = read_and_snapshot(&txt).unwrap();
        assert_eq!(size, data.len() as i64);
        assert_eq!(hash, blake3::hash(data).to_hex().to_string());
        assert_eq!(gunzip(gzo.as_deref().unwrap()).unwrap(), data);

        // 二进制文件（含 NUL）：hash 有、快照 None。
        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, b"a\0b\0c").unwrap();
        let (_h, _s, gzb) = read_and_snapshot(&bin).unwrap();
        assert!(gzb.is_none());

        // 不存在的文件 → None。
        assert!(read_and_snapshot(&dir.path().join("nope")).is_none());
    }
}
