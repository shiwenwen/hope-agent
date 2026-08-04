//! 桌面宠物的 kernel 面：`ChatUiSurface`（chat_turns 表列 wire 类型）、
//! 活动变更通知、配置更新 trampoline。宠物本体（sprite 库 / 导入 /
//! creator / 活动投影）在特征 crate `ha-pet`。
//!
//! kernel 调用点路径不变：`crate::pet::ChatUiSurface`（session / turns /
//! chat_engine / 壳层 chat 入口）、`crate::pet::emit_activity_changed`
//! （session 写路径 / knowledge / approval）、`crate::pet::update_config`
//! （ha-settings 分支 + `/pet` 命令）。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

use ha_config_schema::pet::{PetConfig, PetRef};

/// 主对话投影边界的第一方 UI 表面标记（`chat_turns.ui_surface` 列）。
/// 语义与红线见 AGENTS「桌面宠物」段与 docs/architecture/pet.md。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatUiSurface {
    MainChat,
    QuickChat,
    KnowledgeChat,
    DesignChat,
    PetChat,
}

impl ChatUiSurface {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainChat => "main_chat",
            Self::QuickChat => "quick_chat",
            Self::KnowledgeChat => "knowledge_chat",
            Self::DesignChat => "design_chat",
            Self::PetChat => "pet_chat",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "main_chat" => Some(Self::MainChat),
            "quick_chat" => Some(Self::QuickChat),
            "knowledge_chat" => Some(Self::KnowledgeChat),
            "design_chat" => Some(Self::DesignChat),
            "pet_chat" => Some(Self::PetChat),
            _ => None,
        }
    }
}

/// 活动修订计数：kernel 写路径每次 bump（`emit_activity_changed`），
/// ha-pet 的 snapshot 读它做「查询期间有无并发变更」的重试判定与
/// revision 哈希盐。
static ACTIVITY_REVISION: AtomicU64 = AtomicU64::new(1);

/// 会话/轮次状态变化的 fire-and-forget 通知（前端宠物气泡刷新）。
/// 与迁移前相同：无 EventBus（早期启动/测试）时静默丢弃。
pub fn emit_activity_changed() {
    let revision = ACTIVITY_REVISION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "pet:activity_changed",
            serde_json::json!({ "revision": revision }),
        );
    }
}

/// 当前活动修订值（Acquire；消费方 ha-pet `activity_snapshot`）。
pub fn activity_revision() -> u64 {
    ACTIVITY_REVISION.load(Ordering::Acquire)
}

type PetConfigUpdater = fn(
    Option<bool>,
    Option<PetRef>,
    &'static str,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = anyhow::Result<PetConfig>> + Send + 'static>,
>;

static PET_CONFIG_UPDATER: std::sync::OnceLock<PetConfigUpdater> = std::sync::OnceLock::new();

/// ha-pet 装配期注册配置更新实现（选择校验 + 跨进程库锁 + mutate_config）。
/// 重复注册返回 `Err`。
pub fn register_pet_config_updater(
    updater: PetConfigUpdater,
) -> Result<(), crate::AlreadyRegistered> {
    PET_CONFIG_UPDATER
        .set(updater)
        .map_err(|_| crate::AlreadyRegistered("pet config updater"))
}

/// 更新宠物启用/选中配置（实现在 ha-pet：选择合法性校验、跨进程库锁、
/// `mutate_config(("pet", source))` 写入、事件广播）。未接线 `Err`
/// fail-explicit——消费入口（ha-settings 分支、`/pet` 命令）都是用户显式
/// 动作，报错优于静默。
pub async fn update_config(
    enabled: Option<bool>,
    selected_pet_ref: Option<PetRef>,
    source: &'static str,
) -> anyhow::Result<PetConfig> {
    let Some(updater) = PET_CONFIG_UPDATER.get() else {
        anyhow::bail!("pet feature is not wired in this process (ha_pet::wire() missing)");
    };
    updater(enabled, selected_pet_ref, source).await
}
