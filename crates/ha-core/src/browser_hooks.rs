//! 特征 crate 钩子：kernel ↔ 浏览器特征（ha-browser）边界。
//!
//! kernel 在四个点需要浏览器特征的行为，倒转为装配期原子注册（单
//! OnceLock struct，部分注册不可能）。缺失接线语义逐项：broker 不起 /
//! turn finalize no-op / 会话清理返回说明串（cleanup_watcher 记 debug——
//! 未 wire 进程注册不了 browser 工具、产生不了 lease，且持久化 lease 带
//! 24h TTL 自愈，无残留风险）/ 网页捕获 Err
//! fail-explicit（用户显式动作，报错优于静默）。

use std::future::Future;
use std::pin::Pin;

/// 浏览器抓取当前活动标签页的结果（knowledge 网页收集消费；抓取脚本与
/// backend 协商在 ha-browser 侧）。
#[derive(Debug, Clone)]
pub struct BrowserTabCapture {
    pub url: String,
    pub title: String,
    pub selection_text: String,
    pub page_text: String,
}

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub struct BrowserHooks {
    /// 本地 Chrome Extension broker（loopback + discovery 文件）。
    pub spawn_broker: fn(),
    /// 轮结束后的 extension tab finalize 排程（chat_engine 消费）。
    pub schedule_turn_finalize: fn(&str),
    /// 会话删除/焚毁级联：释放 user-tab lease、关闭 agent 创建的 tab；
    /// 返回人读摘要（cleanup_watcher 记 debug）。
    pub cleanup_session: fn(&str) -> BoxFut<String>,
    /// 抓取当前活动标签页（knowledge 网页收集）。
    pub capture_active_tab: fn() -> BoxFut<anyhow::Result<BrowserTabCapture>>,
}

static BROWSER_HOOKS: std::sync::OnceLock<BrowserHooks> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册。重复注册返回 `Err`。
pub fn register_browser_hooks(hooks: BrowserHooks) -> Result<(), crate::AlreadyRegistered> {
    BROWSER_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("browser hooks"))
}

pub(crate) fn browser_hooks() -> Option<&'static BrowserHooks> {
    BROWSER_HOOKS.get()
}

/// 每个 no-op 分支首次未命中时打一次 warn，让「未 wire → 静默 no-op」这条
/// 路径可 grep（`category=browser source=hooks.<slot>`）；避免刷屏。
static UNWIRED_SPAWN_WARN: std::sync::OnceLock<()> = std::sync::OnceLock::new();
static UNWIRED_TURN_FINALIZE_WARN: std::sync::OnceLock<()> = std::sync::OnceLock::new();
static UNWIRED_CLEANUP_WARN: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// 启动 broker（Chrome extension loopback + discovery file）。未 wire 即
/// no-op——扩展连不上，但注册表里也没 browser 工具，故不产生 lease；首次
/// 未命中打一次 warn，方便 headless 部署审计。
pub fn spawn_broker() {
    match browser_hooks() {
        Some(hooks) => (hooks.spawn_broker)(),
        None => {
            UNWIRED_SPAWN_WARN.get_or_init(|| {
                app_warn!(
                    "browser",
                    "hooks.spawn_broker",
                    "ha-browser not wired: Chrome extension broker will not start in this process"
                );
            });
        }
    }
}

/// 轮结束后的 extension tab finalize 排程。未 wire → 依赖 lease 24h TTL 自愈。
pub fn schedule_turn_finalize(session_id: &str) {
    match browser_hooks() {
        Some(hooks) => (hooks.schedule_turn_finalize)(session_id),
        None => {
            UNWIRED_TURN_FINALIZE_WARN.get_or_init(|| {
                app_warn!(
                    "browser",
                    "hooks.schedule_turn_finalize",
                    "ha-browser not wired: turn-end tab finalize skipped for this process (leases rely on 24h TTL)"
                );
            });
        }
    }
}

/// 会话删除时释放 user-tab lease、关闭 agent tab；返回摘要串给
/// cleanup_watcher 记 debug。未 wire → 空清理串 + 首次 warn。
pub async fn cleanup_session(session_id: &str) -> String {
    match browser_hooks() {
        Some(hooks) => (hooks.cleanup_session)(session_id).await,
        None => {
            UNWIRED_CLEANUP_WARN.get_or_init(|| {
                app_warn!(
                    "browser",
                    "hooks.cleanup_session",
                    "ha-browser not wired: session cleanup no-op (leases self-heal via TTL)"
                );
            });
            "browser feature not wired (no extension leases to release)".to_string()
        }
    }
}

/// 抓取当前活动标签页。`BrowserHooks` 结构体本身仍不出 kernel——ha-knowledge
/// 的网页收藏（`knowledge::source::capture_browser_snapshot`）只需要这一项，
/// 故单开薄封装而非放开整组。**fail-explicit**：未 wire 报错而非静默返空，
/// 与迁出前 `source.rs` 里那条 `bail!` 逐字相同（网页捕获是用户显式动作）。
pub async fn capture_active_tab() -> anyhow::Result<BrowserTabCapture> {
    let Some(hooks) = browser_hooks() else {
        anyhow::bail!("browser feature not wired; browser capture unavailable in this binary");
    };
    (hooks.capture_active_tab)().await
}
