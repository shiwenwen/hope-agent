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
use std::path::{Path, PathBuf};

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

/// 已登记 link 的安全解析结果。
enum LinkPath {
    /// 非 symlink 本体、canonicalize 后仍在 root 内，可安全读。
    Readable(PathBuf),
    /// 文件确实不存在（可判 deleted）。
    Missing,
    /// symlink 本体 / 中间目录逃逸 → **安全拒读**（不读内容、不判 drift、不建 link）。
    Rejected,
}

/// 解析 `code_dir/rel` 为可安全读的绝对路径。**红线：绝不跟随 symlink**——已登记路径被替换成
/// 指向绑定目录外（如 `~/.ssh/id_rsa`）的 symlink 时（git checkout 恶意分支 / npm postinstall 均
/// 可植入），会经 drift diff / quote / 基线快照把任意文件内容外泄。`symlink_metadata` 判本体非
/// symlink + `canonicalize` 复验仍在 root 内（挡中间目录 symlink 逃逸）。`Err` 仅保留给瞬时 IO
/// 错（权限 / 锁），供上层区分「暂时读不到」（保守跳过 / 不推进游标）与「确实不存在」。
fn resolve_linked_path(code_dir: &str, rel: &str) -> std::io::Result<LinkPath> {
    use std::io::ErrorKind;
    let root = match Path::new(code_dir).canonicalize() {
        Ok(r) => r,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(LinkPath::Missing),
        Err(e) => return Err(e),
    };
    let abs = root.join(rel);
    match std::fs::symlink_metadata(&abs) {
        Ok(m) if m.file_type().is_symlink() => Ok(LinkPath::Rejected),
        Ok(_) => match abs.canonicalize() {
            Ok(canon) if canon.starts_with(&root) => Ok(LinkPath::Readable(canon)),
            Ok(_) => Ok(LinkPath::Rejected),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(LinkPath::Missing),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(LinkPath::Missing),
        Err(e) => Err(e),
    }
}

/// 流式 BLAKE3 + 字节数（有界内存，不整读大文件）——防已收割路径后被写成 GB 级构建产物时整读
/// 进内存 OOM。
fn hash_file_streaming(path: &Path) -> std::io::Result<(String, u64)> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(f);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hasher.finalize().to_hex().to_string(), total))
}

/// 有界读（至多 `max` 字节 + 是否被截断）——防超大文件整读进内存做 diff 展示时 OOM。
fn read_capped(path: &Path, max: usize) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(f).take(max as u64 + 1);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let truncated = buf.len() > max;
    if truncated {
        buf.truncate(max);
    }
    Ok((buf, truncated))
}

/// 读文件 → (BLAKE3 hex, size, gzip 快照 or None)。`Ok(Some(..))` 读到；`Ok(None)` 文件不存在 /
/// 被安全拒读（symlink 越界）；`Err` 瞬时 IO 错（权限 / 锁——上层据此**不推进游标**，下次重试，
/// 避免永久漏建 link）。BLAKE3 流式（不整读大文件）；仅 ≤ `SNAPSHOT_MAX` 且非二进制才整读做 gzip
/// 快照。
fn read_and_snapshot(
    code_dir: &str,
    rel: &str,
) -> std::io::Result<Option<(String, i64, Option<Vec<u8>>)>> {
    let path = match resolve_linked_path(code_dir, rel)? {
        LinkPath::Readable(p) => p,
        LinkPath::Missing | LinkPath::Rejected => return Ok(None),
    };
    let (hash, size) = hash_file_streaming(&path)?;
    let gz = if size as usize <= SNAPSHOT_MAX {
        match std::fs::read(&path) {
            // 与文件浏览器同口径判二进制（NUL 或非 UTF-8）——非 UTF-8（如 GBK/Latin-1 源码）不存
            // 快照，否则 diff 回放 `from_utf8_lossy` 会出 U+FFFD 乱码。
            Ok(bytes) if !crate::filesystem::looks_binary_bytes(&bytes) => gzip(&bytes).ok(),
            _ => None,
        }
    } else {
        None
    };
    Ok(Some((hash, size as i64, gz)))
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
    'outer: loop {
        let (batch, more) = sdb.load_session_messages_after(&r.session_id, cursor, 500)?;
        if batch.is_empty() {
            break;
        }
        for m in &batch {
            // 工具行两阶段落库（先 INSERT `stream_status='streaming'` 占位、完成才 UPDATE 回填
            // tool_metadata）。遇到仍在流式的行就停在它之前、游标不越过——否则回填后 `id ≤ cursor`
            // 永不再扫，该文件永不建 link、其后代码漂移永不触发（会话进行中收割即会踩到）。
            if m.stream_status.as_deref() == Some("streaming") {
                break 'outer;
            }
            for p in extract_written_paths(std::slice::from_ref(m)) {
                let Some(rel) = rel_within(&code_dir_canon, Path::new(&p)) else {
                    // 落地路径不在绑定目录内——不入库（防越界；实现会话本可能改仓库外文件）。
                    continue;
                };
                match read_and_snapshot(&r.code_dir, &rel) {
                    Ok(Some((hash, size, gz))) => {
                        db.upsert_code_link(&r.id, &rel, &hash, size, gz.as_deref(), &now())?;
                        // 「新回执赢」：删同产物其它回执下同 rel_path 的旧 link。
                        db.delete_links_same_path_in_other_receipts(&r.artifact_id, &r.id, &rel)?;
                        any_new = true;
                    }
                    // 文件此刻不存在（转瞬文件 / 会话删了它）/ 被拒（symlink 越界）→ 跳过不建 link。
                    Ok(None) => {}
                    Err(e) => {
                        // 瞬时读失败（权限 / 锁）——不推进游标越过本行，下次收割重试，避免把「读失败」
                        // 误当「文件不存在」而永久漏建 link。代价：极少数**永久**不可读的中段文件会
                        // 把本回执后续收割卡住（有 warn 可诊断），远好于静默丢追踪。
                        crate::app_warn!(
                            "design",
                            "code_sync",
                            "harvest read failed for {} in receipt {}: {}; holding cursor",
                            rel,
                            r.id,
                            e
                        );
                        break 'outer;
                    }
                }
            }
            cursor = m.id; // 仅在本行完全处理后推进。
        }
        if !more {
            break;
        }
    }

    if cursor != r.harvest_cursor || any_new || r.harvested_at.is_none() {
        // `harvest_revision` 不再被任何读路径消费（drift 判定逐文件重算 BLAKE3），故不再为它起 git
        // 子进程算指纹（省 ~5 个 git 进程/次收割，含两遍全量 binary diff）；列保留兼容、恒 None。
        db.update_receipt_harvest(&r.id, cursor, None, &now())?;
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
    let mut by_path: BTreeMap<String, (String, String, i64)> = BTreeMap::new(); // rel → (code_dir, baseline hash, size)
    for (r, l) in &links {
        by_path.insert(
            l.rel_path.clone(),
            (r.code_dir.clone(), l.blake3.clone(), l.size_bytes),
        );
    }
    let mut files = Vec::new();
    for (rel, (code_dir, baseline, baseline_size)) in &by_path {
        if files.len() >= DRIFT_FILES_MAX {
            break;
        }
        match resolve_linked_path(code_dir, rel) {
            Ok(LinkPath::Readable(path)) => match std::fs::metadata(&path) {
                // size 已变 → 直接判 modified，免读免哈希（大多数漂移文件在此短路）。
                Ok(m) if m.len() as i64 != *baseline_size => files.push(CodeDriftFile {
                    path: rel.clone(),
                    state: "modified".to_string(),
                }),
                // size 相同 → 流式哈希确认内容（有界内存，不整读大文件）。
                Ok(_) => match hash_file_streaming(&path) {
                    Ok((cur, _)) if &cur != baseline => files.push(CodeDriftFile {
                        path: rel.clone(),
                        state: "modified".to_string(),
                    }),
                    Ok(_) => {}  // 未变。
                    Err(_) => {} // 瞬时读失败 → 保守不判 drift。
                },
                Err(_) => {} // 瞬时 stat 失败 → 保守不判 drift。
            },
            Ok(LinkPath::Missing) => {
                // 文件不存在。目录整体失效（外置盘未挂载 / 仓库删）→ 不假 stale（绑定级 stale 另标红）。
                if Path::new(code_dir).is_dir() {
                    files.push(CodeDriftFile {
                        path: rel.clone(),
                        state: "deleted".to_string(),
                    });
                }
            }
            // symlink 越界 → 安全拒读、不判 drift（红线：绝不跟随 symlink 读内容）。
            Ok(LinkPath::Rejected) => {}
            Err(_) => {} // 瞬时路径 IO 错 → 跳过。
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
        // 原子写 `metadata.codeDrift` 单键（`json_set`/`json_remove`），不读-改-写整列——消除
        // watcher 后台线程与前台 metadata 写互相丢键的竞态。
        match &flag {
            Some(f) => {
                if let Ok(j) = serde_json::to_string(f) {
                    db.set_artifact_code_drift(artifact_id, Some(&j))?;
                }
            }
            None => db.set_artifact_code_drift(artifact_id, None)?,
        }
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

/// watcher debounce 回调：某 code_dir 下全部关联产物**先收割后比对**。收割是幂等游标增量、且停
/// 在在途流式行之前，故会把「实现会话自己刚落盘的改动」吸收为基线——否则实现会话仍在写代码时
/// watcher 无法区分改动来源，会把会话自身改动误报为外部漂移（弹「代码实现有更新」横幅、误导用户
/// 点「带到对话」把自己刚写的代码当外部变更回灌）。与 `check_code_drift` 同一收割入口。
pub(crate) fn check_drift_for_dir(code_dir: &str) -> Result<()> {
    let db = get_design_db()?;
    let artifacts: BTreeSet<String> = db
        .links_index_for_dir(code_dir)?
        .into_iter()
        .map(|(_proj, art, _rel)| art)
        .collect();
    if let Some(sdb) = crate::globals::get_session_db() {
        for aid in &artifacts {
            for r in &db.list_receipts_for_artifact(aid)? {
                if let Err(e) = harvest_receipt(db, sdb, r) {
                    crate::app_warn!(
                        "design",
                        "code_sync",
                        "watcher harvest receipt {} failed: {}",
                        r.id,
                        e
                    );
                }
            }
        }
    }
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
        if files.len() >= DRIFT_FILES_MAX {
            break; // 单次响应文件数上限（防 git 切分支致全部 link 漂移时无界膨胀）。
        }
        match resolve_linked_path(code_dir, rel) {
            Ok(LinkPath::Readable(path)) => {
                // 先流式哈希判是否漂移（有界内存）；未漂移则不解压基线快照、不整读——避免对每个未漂移
                // link 白读白解压，也防已收割路径后被写成 GB 级文件时整读 OOM。
                let cur_hash = match hash_file_streaming(&path) {
                    Ok((h, _)) => h,
                    Err(_) => continue,
                };
                if &cur_hash == baseline {
                    continue; // 未漂移。
                }
                let (bytes, read_trunc) = match read_capped(&path, SNAPSHOT_MAX) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // 非文本（二进制 / 非 UTF-8 如 GBK）：不做 lossy 文本 diff（会出 U+FFFD 乱码），出
                // before/after 皆空的占位——DiffPanel 与写工具二进制约定同款渲染。
                if crate::filesystem::looks_binary_bytes(&bytes) {
                    files.push(DriftFileChange {
                        kind: "file_change".to_string(),
                        path: rel.clone(),
                        action: "edit".to_string(),
                        lines_added: 0,
                        lines_removed: 0,
                        before: None,
                        after: None,
                        language: detect_language(rel).to_string(),
                        truncated: false,
                    });
                    if total < DRIFT_QUOTE_TOTAL_MAX {
                        let block =
                            format!("## {rel} [modified]\n（二进制 / 非文本文件，已变更）\n\n");
                        total += block.len();
                        quote.push_str(&block);
                    }
                    continue;
                }
                let before = db
                    .get_link_snapshot(*link_id)?
                    .and_then(|gz| gunzip(&gz).ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
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
                    truncated: bt || at || read_trunc,
                });
                if total < DRIFT_QUOTE_TOTAL_MAX {
                    let snippet = crate::util::truncate_utf8(&after_raw, DRIFT_QUOTE_FILE_MAX);
                    let block = format!("## {rel} [modified]\n{snippet}\n\n");
                    total += block.len();
                    quote.push_str(&block);
                }
            }
            Ok(LinkPath::Missing) => {
                if !Path::new(code_dir).is_dir() {
                    continue; // 目录失效，非文件删除。
                }
                let before = db
                    .get_link_snapshot(*link_id)?
                    .and_then(|gz| gunzip(&gz).ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned());
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
            // symlink 越界 → 安全跳过（红线：不读内容、不入 diff / quote）。
            Ok(LinkPath::Rejected) => continue,
            Err(_) => continue,
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
        match read_and_snapshot(&r.code_dir, &l.rel_path) {
            Ok(Some((hash, size, gz))) => {
                if hash == l.blake3 {
                    continue; // 未变则不重写等值 BLOB（省 gzip 压缩 + WAL 写放大）。
                }
                db.update_link_baseline(l.id, &hash, size, gz.as_deref(), &now())?;
            }
            Ok(None) => {
                // 文件确实删了 / 被拒（symlink 越界）→ 丢弃该 link（不再追踪）；目录失效则保留（转瞬态）。
                if Path::new(&r.code_dir).is_dir() {
                    db.delete_link(l.id)?;
                }
            }
            // 瞬时读失败（权限 / 锁）→ 保留 link 不动，下次同步再试。
            Err(_) => {}
        }
    }
    let artifact = db
        .get_artifact(artifact_id)?
        .with_context(|| format!("artifact not found: {artifact_id}"))?;
    // 原子清 `codeDrift` 单键（不读-改-写整列，避免与前台 metadata 写竞态丢键）。
    if parse_code_drift(artifact.metadata.as_deref()).is_some() {
        db.set_artifact_code_drift(artifact_id, None)?;
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
            persistence_run_id: None,
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
    fn parse_and_flags_equal() {
        let flag = CodeDriftFlag {
            files: vec![CodeDriftFile {
                path: "a.ts".into(),
                state: "modified".into(),
            }],
            checked_at: "t".into(),
            session_id: Some("s".into()),
        };
        // 序列化 → 解析回来（外部 agent / 前端读的 metadata.codeDrift 形状）。
        let j = serde_json::to_string(&flag).unwrap();
        let meta = format!(r#"{{"selfCheck":{{"flag":"ok"}},"codeDrift":{j}}}"#);
        let parsed = parse_code_drift(Some(&meta)).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].state, "modified");
        // 无 codeDrift 键 → None。
        assert_eq!(parse_code_drift(Some(r#"{"selfCheck":{}}"#)), None);

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
        let root = dir.path().to_string_lossy().into_owned();
        // 文本文件：有快照。
        std::fs::write(dir.path().join("a.ts"), data).unwrap();
        let (hash, size, gzo) = read_and_snapshot(&root, "a.ts").unwrap().unwrap();
        assert_eq!(size, data.len() as i64);
        assert_eq!(hash, blake3::hash(data).to_hex().to_string());
        assert_eq!(gunzip(gzo.as_deref().unwrap()).unwrap(), data);

        // 二进制文件（含 NUL）：hash 有、快照 None。
        std::fs::write(dir.path().join("b.bin"), b"a\0b\0c").unwrap();
        let (_h, _s, gzb) = read_and_snapshot(&root, "b.bin").unwrap().unwrap();
        assert!(gzb.is_none());

        // 非 UTF-8（GBK「你好」，无 NUL）：也判二进制、不存快照——否则 diff 回放出 U+FFFD 乱码。
        std::fs::write(dir.path().join("gbk.txt"), b"\xC4\xE3\xBA\xC3").unwrap();
        let (_h, _s, gzk) = read_and_snapshot(&root, "gbk.txt").unwrap().unwrap();
        assert!(gzk.is_none());

        // 不存在的文件 → Ok(None)。
        assert!(read_and_snapshot(&root, "nope").unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_linked_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let root_dir = tempfile::tempdir().unwrap();
        let root = root_dir.path().to_string_lossy().into_owned();
        // 目录外的敏感文件。
        let secret_dir = tempfile::tempdir().unwrap();
        let secret = secret_dir.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        // 普通文件 → Readable。
        std::fs::write(root_dir.path().join("real.ts"), b"x").unwrap();
        assert!(matches!(
            resolve_linked_path(&root, "real.ts"),
            Ok(LinkPath::Readable(_))
        ));
        // 指向目录外的 symlink → Rejected（红线：绝不跟随读到 secret）。
        symlink(&secret, root_dir.path().join("leak.ts")).unwrap();
        assert!(matches!(
            resolve_linked_path(&root, "leak.ts"),
            Ok(LinkPath::Rejected)
        ));
        // read_and_snapshot 对 symlink 越界回 Ok(None)（不读内容）。
        assert!(read_and_snapshot(&root, "leak.ts").unwrap().is_none());
        // 不存在 → Missing。
        assert!(matches!(
            resolve_linked_path(&root, "nope.ts"),
            Ok(LinkPath::Missing)
        ));
    }
}
