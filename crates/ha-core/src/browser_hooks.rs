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
