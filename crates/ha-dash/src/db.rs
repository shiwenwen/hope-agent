//! 大盘的 **只读** `sessions.db` 连接。
//!
//! # 为什么不复用 kernel 的 `SessionDB`
//!
//! 大盘是纯读侧聚合：七十多条 `SELECT`，一条写都没有。kernel 的
//! `SessionDB::with_conn_internal` 是 `pub(crate)`，**刻意不对特征 crate
//! 暴露**（「核心库 schema 不做跨 crate 隐式 API」），而把这七十多条查询
//! 逐一包成 kernel 的类型化方法，等于把 7k 行大盘 SQL 搬回 kernel——正好
//! 抵消拆分的意义。
//!
//! 于是走第三条路：本 crate 用 `SQLITE_OPEN_READ_ONLY` 自开一条连接。这
//! **比暴露 `with_conn_internal` 更强**——那个拿到的 `&Connection` 仍然能
//! 执行写语句、只读全靠约定；这里的句柄物理上写不了，正好把 AGENTS
//! 「大盘只读」那条红线落到类型系统外的一层强制上。
//!
//! # 一致性与并发
//!
//! `sessions.db` 是 WAL（`session/db.rs` 建库时 `PRAGMA journal_mode=WAL`），
//! 读连接不被写者阻塞，看到的是最近一次提交的快照。大盘本就是最终一致的
//! 报表视图，读不到某个尚未提交的事务中间态是正确行为，不是缺陷。
//!
//! 首次打开失败（例如库文件尚未由 kernel 建出）**不永久降级**：下次调用
//! 重试——与 `recap::db` 的连接缓存同款语义。
//!
//! # 与迁移前的一处 dev-only 分歧
//!
//! `dev_tools::dev_clear_sessions` 只 unlink `sessions.db`、**不重建**，而
//! kernel 的 `SESSION_DB` 是 `OnceLock`、永不重开——所以「库被删后重建、缓存
//! 句柄失效」这个场景在进程内不可达，两边同样一直写那个已 unlink 的 inode。
//! 唯一的差别是 `dev_clear_sessions` **之后首次**打开大盘：迁移前共用 kernel
//! 那个 unlink inode、还能返回陈旧数据；现在本 crate 会拿到 `SQLITE_CANTOPEN`，
//! 大盘面板报错直到重启。dev-only，且报错比返回陈旧数据更诚实，故不特殊处理，
//! 但记在这里免得被当成 bug 排查。

use anyhow::{anyhow, Result};
use rusqlite::{Connection, OpenFlags};
use std::ops::Deref;
use std::sync::{Mutex, MutexGuard, OnceLock};

type ConnCell = Mutex<Option<Connection>>;

fn sessions_cell() -> &'static ConnCell {
    static CONN: OnceLock<ConnCell> = OnceLock::new();
    CONN.get_or_init(|| Mutex::new(None))
}

fn cron_cell() -> &'static ConnCell {
    static CONN: OnceLock<ConnCell> = OnceLock::new();
    CONN.get_or_init(|| Mutex::new(None))
}

/// WAL 读者多数时候不阻塞，但仍有拿到 `SQLITE_BUSY` 的路径：需要重建 /
/// 恢复 `-shm` wal-index 时（上次非正常退出，或 `-shm` 被 `dev_tools` 在活
/// 连接底下删掉），以及撞上 checkpoint 的排他阶段。**超时为 0 会把这些直接
/// 变成一次大盘查询失败**，而迁移前走的那两条 kernel 连接都留了 5 秒重试
/// （`session/db.rs` 的写连接与只读池、`cron/db.rs`）。`busy_timeout` 是
/// 连接级设置，每条连接必须自己设。
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn open_read_only(path: std::path::PathBuf, label: &str) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| anyhow!("open {} read-only ({}): {}", label, path.display(), e))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| anyhow!("set busy_timeout on {}: {}", label, e))?;
    Ok(conn)
}

fn acquire(
    cell: &'static ConnCell,
    path: Result<std::path::PathBuf>,
    label: &str,
) -> Result<ReadConn> {
    let mut guard: MutexGuard<'static, Option<Connection>> =
        cell.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(open_read_only(path?, label)?);
    }
    Ok(ReadConn(guard))
}

/// 持锁的只读连接句柄。`Deref<Target = Connection>` 让调用点与迁移前的
/// `MutexGuard<Connection>` 用法逐字相同（`conn.prepare(...)` 等）。
pub(crate) struct ReadConn(MutexGuard<'static, Option<Connection>>);

impl Deref for ReadConn {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        // 构造 `ReadConn` 前 `read_conn()` 已保证内层是 `Some`，且 guard
        // 在整个生命周期内独占该 cell——没有别的路径能把它置回 `None`。
        self.0
            .as_ref()
            .expect("ReadConn is only constructed with an open connection")
    }
}

/// 取 `sessions.db` 只读连接（一次查询一把锁，粒度与迁移前
/// `session_db.conn.lock()` 相同，只是锁的是本 crate 自己的连接、不再与
/// kernel 写路径争同一把）。首次打开失败**不永久降级**：下次调用重试。
pub(crate) fn read_conn() -> Result<ReadConn> {
    acquire(sessions_cell(), ha_core::session::db_path(), "sessions.db")
}

/// 取 `cron.db` 只读连接。大盘的 cron 成功率 / 活跃任务数两处聚合直接读
/// 这张库（迁移前是 `cron_db.conn.lock()`）——同样只读，同样不碰 kernel 的
/// 写路径。
pub(crate) fn read_cron_conn() -> Result<ReadConn> {
    acquire(cron_cell(), ha_core::paths::cron_db_path(), "cron.db")
}

// ── 测试注入口 ──────────────────────────────────────────────────
//
// 只读连接是**进程级全局**、指向 `session::db_path()` 的真实库，而大盘的
// fixture 测试各自建临时库——不给注入口，那些测试读到的永远不是自己写的
// 数据（而且是「静默读到空表」这种最难查的失败）。
//
// 注入口连带一把**串行锁**：全局只有一条连接，两个测试同时把它指向各自的
// fixture 必然互相踩。所有用 fixture 的大盘测试必须先 `lock_dash_db()`，
// 拿着 guard 期间再 `point_at_test_db()`。

#[cfg(test)]
pub(crate) fn lock_dash_db() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 把全局只读连接指向 fixture 库。**必须在持有 [`lock_dash_db`] 时调用。**
///
/// 用 `SQLITE_OPEN_READ_ONLY` 打开，与生产路径同款——测试因此也会踩到
/// 「只读句柄写不了」这条约束，不会出现测试能写、生产不能写的假绿。
#[cfg(test)]
pub(crate) fn point_at_test_db(sessions: &std::path::Path) {
    let conn = Connection::open_with_flags(sessions, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open fixture sessions.db read-only");
    // fixture 库是 `journal_mode=MEMORY`——**回滚日志模式下读者会被写者真阻塞**。
    // 现在不炸只因为每个 dash 测试单线程且被 `lock_dash_db()` 串行；超时是安全
    // 边际，别因为"测试都过了"就省掉。
    conn.busy_timeout(BUSY_TIMEOUT)
        .expect("set busy_timeout on fixture sessions.db");
    *sessions_cell().lock().unwrap_or_else(|e| e.into_inner()) = Some(conn);
}

/// 同上，指向 fixture 的 `cron.db`。
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn point_at_test_cron_db(cron: &std::path::Path) {
    let conn = Connection::open_with_flags(cron, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open fixture cron.db read-only");
    conn.busy_timeout(BUSY_TIMEOUT)
        .expect("set busy_timeout on fixture cron.db");
    *cron_cell().lock().unwrap_or_else(|e| e.into_inner()) = Some(conn);
}
