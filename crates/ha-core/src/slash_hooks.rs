//! Slash 命令**分发钩子**——kernel / IM 渠道 → 装配层的唯一回调面。
//!
//! # 为什么是钩子而不是直接调用（crate-split 破环）
//!
//! [`crate::slash_commands`] 是装配层：它的 handler 逐个调 skills /
//! channel / cron / dashboard / coding_improvement 等未来特征 crate，位置
//! 在依赖图**顶端**。而 IM 渠道（未来的 ha-channel）需要执行 slash 命令，
//! 直接 `use crate::slash_commands::handlers` 就是自下而上的反向边——
//! ha-channel 拆出后会与装配层成环。
//!
//! 契约物（命令表 / wire 类型 / 解析 / 转录落库）已归位
//! [`crate::slash_defs`]，channel 直接用；**只有真正需要装配层能力的三件事**
//! 走这里：命令分发、IM 菜单同步、技能命令的参数元数据。
//!
//! # 未装配语义
//!
//! 三槽**原子注册**（部分注册＝分发能到、菜单同步不到，是难查的半瘫痪）。
//! 未注册时：
//!
//! - `dispatch` → `Err`：无装配的进程里根本没有 handler 可跑，返回错误让
//!   IM worker 走它既有的错误分支，**不会**把命令文本当普通消息喂给模型。
//! - `menu_entries` → **内置命令表**（经同一 `im_menu_filter_and_cap`
//!   收口），与装配层 `im_menu_entries` 里「`list_slash_commands` 失败则
//!   回退内置表」同方向。**刻意不返回空表**：IM 侧的菜单同步是
//!   `set_my_commands` / `bulk_overwrite_global_commands` 这类**覆盖写**，
//!   空表会把平台上已注册的命令整批抹掉——降级不该产生破坏性远端副作用。
//! - `skill_command_help` → `None`：等价于「不是技能命令」，调用方
//!   （`lookup_command_help`）本就有 `None` 分支。

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::slash_defs::types::{CommandResult, SlashCommandDef};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 技能命令的参数元数据（IM 侧渲染选项按钮 / 用法提示用）。字段与
/// `SkillEntry` 的同名项一一对应，避免 channel 认识 skills 的类型。
#[derive(Debug, Clone, Default)]
pub struct SkillCommandHelp {
    pub arg_placeholder: Option<String>,
    pub arg_options: Option<Vec<String>>,
}

pub struct SlashHooks {
    /// `handlers::dispatch(session_id, agent_id, command, args)`
    pub dispatch: for<'a> fn(
        Option<&'a str>,
        &'a str,
        &'a str,
        &'a str,
    ) -> BoxFut<'a, Result<CommandResult, String>>,
    /// `slash_commands::im_menu_entries()`
    pub menu_entries: fn() -> BoxFut<'static, Vec<SlashCommandDef>>,
    /// 技能命令参数元数据；非技能命令返回 `None`。
    pub skill_command_help: fn(&str) -> Option<SkillCommandHelp>,
}

static SLASH_HOOKS: OnceLock<SlashHooks> = OnceLock::new();

/// 装配期注册 slash 分发钩子（三槽原子）。重复注册返回 `Err`。
///
/// `pub(crate)`：唯一注册方是 ha-core 自己的 `app_init`（与
/// `config::register_side_effects` / `filesystem::register_root_resolvers`
/// 同档的**内部**装配钩子）。`pub` 只留给特征 crate 从 `wire()` 注册的钩子
/// （`mcp_hooks` / `vcs_hooks`）——否则先注册者胜出的语义会让下游 crate
/// 悄悄顶替真正的分发装配（slash 面含 `/permission` 等权限命令）。
pub(crate) fn register_slash_hooks(hooks: SlashHooks) -> Result<(), crate::AlreadyRegistered> {
    SLASH_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("slash dispatch hooks"))
}

fn hooks() -> Option<&'static SlashHooks> {
    SLASH_HOOKS.get()
}

/// 执行一条 slash 命令。未装配即 `Err`——调用方（IM worker）已有错误分支，
/// 不会静默把命令当普通消息喂给模型。
pub async fn dispatch(
    session_id: Option<&str>,
    agent_id: &str,
    command: &str,
    args: &str,
) -> Result<CommandResult, String> {
    let Some(h) = hooks() else {
        return Err(
            "slash commands are unavailable in this process (slash hooks not registered)"
                .to_string(),
        );
    };
    (h.dispatch)(session_id, agent_id, command, args).await
}

/// IM 菜单同步用的命令清单。未装配回落**内置命令表**（同一过滤 / 封顶
/// 收口）——绝不返回空表，见模块文档「未装配语义」。
pub async fn im_menu_entries() -> Vec<SlashCommandDef> {
    match hooks() {
        Some(h) => (h.menu_entries)().await,
        None => {
            crate::slash_defs::im_menu_filter_and_cap(crate::slash_defs::registry::all_commands())
        }
    }
}

/// 技能命令的参数元数据。未装配返回 `None`（等价于「不是技能命令」）。
pub fn skill_command_help(name: &str) -> Option<SkillCommandHelp> {
    (hooks()?.skill_command_help)(name)
}
