//! 内置工具分发注册表（阶段 2.5：静态 match 反转）。
//!
//! `execution.rs` 的巨型 `match name { TOOL_X => adapter(...) }` 反转为查表
//! 分发，使 tool adapter 未来能随特征 crate 搬走并在装配期从 crate 外注册
//! （阶段 3 的硬前置——adapter 留在 ha-core 会造成 core → 特征反向依赖，
//! 搬走则静态 match 编不过，见方案 §阶段 2.5）。
//!
//! **边界（刻意不反转的部分）**：
//! - `async_jobs::retry::is_retry_eligible` 保持代码级白名单（AGENTS.md
//!   红线：开放注册＝工具可自报重试资格）；
//! - `dispatch::resolve_tool_fate` 可见性/执行层兜底、审批 category 枚举、
//!   schema 定义（`definitions/core_tools.rs`）本阶段不动；
//! - MCP 工具不进本表（`mcp__` 前缀在 dispatch 尾部走专属子系统，原位保留）。
//!
//! **冻结语义**：注册只允许发生在首次分发之前（装配期，`init_runtime` /
//! 特征 crate 注册钩子）。首次 `lookup` 把 builtin 表 + 外部注册项冻结成
//! 只读 HashMap——之后的注册调用返回 `Err`（fail-loud，绝不静默丢工具）。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use super::ToolExecContext;

/// 分发 future：与原 match 臂的整体类型一致（`anyhow::Result<String>`），
/// 借用 `args` / `ctx` 存活期内有效。
pub type BuiltinToolFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'a>>;

/// 无捕获 fn 指针 + boxed future——注册项可跨 crate 传递，无 trait 对象
/// 生命周期负担。适配不同 adapter 签名（有/无 ctx、同步小逻辑）用包装 fn。
pub type BuiltinToolHandler = for<'a> fn(&'a Value, &'a ToolExecContext) -> BuiltinToolFuture<'a>;

/// 一个内置（或特征 crate 注册的）工具的分发条目。
pub struct BuiltinToolEntry {
    /// 规范名（`TOOL_*` 常量值）。
    pub name: &'static str,
    /// 历史别名（如 `read` 的 `"read_file"`），与规范名同 handler。
    pub aliases: &'static [&'static str],
    pub handler: BuiltinToolHandler,
}

/// 装配期挂进来的外部条目（特征 crate 经 `register_external_tools` 投递，
/// 冻结时并入只读表）。`None` = 已冻结——「是否还收注册」与「队列消费」是
/// **同一把锁下的同一个事实**：不能用 `FROZEN.get()` 判冻结，否则
/// `get_or_init` 闭包 drain 队列到 `FROZEN` 可见之间存在 TOCTOU 窗口，
/// 该窗口内的注册会被静默吞掉还返回 Ok（channel dispatcher 线程在
/// `freeze_now` 之前就可能驱动首次分发，窗口真实可达）。
static PENDING_EXTERNAL: Mutex<Option<Vec<BuiltinToolEntry>>> = Mutex::new(Some(Vec::new()));

/// 冻结表的值：handler 之外**必须**携带规范名——执行门（fate 兜底 /
/// denied_tools / Skill / Plan allowlist）都发生在 lookup 之前且按名字判定，
/// 别名若不先归一到规范名，就会以一个「没有 `ToolDefinition` 的名字」滑过
/// 全部检查、随后执行规范 handler（codex review P1）。规范化的唯一事实源
/// 就是本表（`canonical_name`），执行层不得再硬编码别名清单。
#[derive(Clone, Copy)]
pub(crate) struct RegisteredTool {
    pub canonical: &'static str,
    pub handler: BuiltinToolHandler,
}

static FROZEN: OnceLock<HashMap<&'static str, RegisteredTool>> = OnceLock::new();

/// 特征 crate 在装配期注册工具分发条目。必须发生在首次工具分发之前
/// （`init_runtime` 内、`freeze_now()` 调用点之前），冻结后返回 `Err`——
/// 注册失败必须 fail-loud，静默丢弃等于该工具在运行期直接消失。
///
/// **契约（阶段 3 落地前必须遵守）**：注册 handler 的同时必须让该工具有
/// `ToolDefinition`（schema / fate 元数据），否则 `resolve_tool_fate`
/// 可见性兜底对它 no-op（`builtin_fate_error` 对无 definition 的名字返
/// `None`）——工具可执行但不受 `tools.allow/deny` 等约束。冻结时对缺
/// definition 的**全部**注册名（含本入口投递的外部条目）记 warn（见
/// [`freeze_now`]）。别名无须独立 definition：执行门经 `canonical_name`
/// 先归一到规范名再判定。
pub fn register_external_tools(
    entries: Vec<BuiltinToolEntry>,
) -> Result<(), crate::AlreadyRegistered> {
    let mut pending = PENDING_EXTERNAL.lock().unwrap_or_else(|p| p.into_inner());
    match pending.as_mut() {
        Some(queue) => {
            queue.extend(entries);
            Ok(())
        }
        None => Err(crate::AlreadyRegistered(
            "tool registry (already frozen — register during init_runtime)",
        )),
    }
}

/// 特征 crate 在装配期注册**工具 schema 提供者**。
///
/// 与 [`register_external_tools`] 配对使用：那个注册 handler，这个补上
/// `ToolDefinition`。缺了它，`resolve_tool_fate` 的可见性兜底对该工具
/// no-op（见上方契约），`freeze_now` 也会记 warn。
///
/// 传的是**函数指针而非现成 Vec**：schema 构造要读 `cached_config()`
/// （飞书工具的 tier 取决于是否配了账号），而 `wire()` 跑在配置缓存建立
/// 之前；延迟到目录首次汇编时再调即可。
/// **冻结后返回 `Err`**，与 [`register_external_tools`] 同一档：工具目录
/// （`dispatch::ALL_DISPATCHABLE_TOOLS`）是个 `LazyLock`，一旦被
/// `all_dispatchable_tools` / `is_internal_tool` 等入口先行实体化，之后再注册
/// 的 provider **永远进不去那份已缓存的目录**。而 handler 走的是另一套队列、
/// 照样注册成功——结果就是「工具能执行，却因为查不到 `ToolDefinition` 而滑过
/// `resolve_tool_fate` 的 `tools.allow/deny` 可见性兜底」。这正是上面那条契约
/// 要防的失效模式，所以它必须 fail loud 而不是静默 `Ok`。
pub fn register_external_tool_definitions(
    provider: fn() -> Vec<crate::tool_defs::ToolDefinition>,
) -> Result<(), crate::AlreadyRegistered> {
    if CATALOG_REALIZED.load(std::sync::atomic::Ordering::Acquire) {
        return Err(crate::AlreadyRegistered(
            "external tool definitions (tool catalog already realized — register during wire(), before any tool listing)",
        ));
    }
    EXTERNAL_DEFS
        .set(provider)
        .map_err(|_| crate::AlreadyRegistered("external tool definitions provider"))
}

static EXTERNAL_DEFS: OnceLock<fn() -> Vec<crate::tool_defs::ToolDefinition>> = OnceLock::new();
static CATALOG_REALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 工具目录开始汇编——此后任何 provider 注册都太晚了。由
/// `dispatch::ALL_DISPATCHABLE_TOOLS` 的初始化闭包**在读 provider 之前**调用。
pub(crate) fn mark_catalog_realized() {
    CATALOG_REALIZED.store(true, std::sync::atomic::Ordering::Release);
}

/// 外部特征 crate 提供的工具 schema。未注册即空表（该特征不在本构建里）。
pub(crate) fn external_tool_definitions() -> Vec<crate::tool_defs::ToolDefinition> {
    EXTERNAL_DEFS.get().map(|f| f()).unwrap_or_default()
}

/// 查规范名或别名对应的条目（含规范名 + handler）。首次调用冻结注册表。
pub(crate) fn lookup(name: &str) -> Option<RegisteredTool> {
    frozen().get(name).copied()
}

/// 别名 → 规范名（规范名自身也命中）。未注册（MCP / 未知名）返 `None`。
/// 执行门规范化的唯一入口——见 `execution.rs::canonical_builtin_tool_name`。
pub(crate) fn canonical_name(name: &str) -> Option<&'static str> {
    frozen().get(name).map(|t| t.canonical)
}

/// 装配期主动冻结（`init_runtime` 尾部调用）：让「重名注册」这类装配 bug
/// 在启动时 panic，而不是推迟到用户第一次工具调用才炸。幂等。
///
/// 冻结后对「注册表有 handler 但无 `ToolDefinition`」的**规范名**记 warn：
/// 这类工具滑过 `resolve_tool_fate` 可见性兜底（见 `register_external_tools`
/// 契约）。校验遍历**冻结后的全表**——外部注册项正是阶段 3 最可能漏配
/// definition 的一批，只查 builtin 等于契约对它们不设防（codex review P1）。
/// 别名不单独检（历史别名本就无独立 definition，随规范名归一）。放在主动
/// 冻结路径而非懒冻结（`lookup`），避免不跑 `init_runtime` 的测试进程刷噪音。
pub fn freeze_now() {
    let defined = defined_tool_names();
    let mut missing: Vec<&'static str> = frozen()
        .values()
        .map(|t| t.canonical)
        .filter(|canonical| !defined.contains(*canonical))
        .collect();
    missing.sort_unstable();
    missing.dedup();
    for canonical in missing {
        app_warn!(
            "tools",
            "registry_freeze",
            "tool `{}` has a dispatch handler but no ToolDefinition — resolve_tool_fate visibility fallback is a no-op for it",
            canonical
        );
    }
    // 反向守卫：有 ToolDefinition 却无 handler ＝ schema 会把该工具广告给
    // 模型、dispatch 却报 Unknown tool。特征 crate 拆出后（阶段 3 起）这
    // 正是「二进制忘调 <feature>::wire()」的症状——builtin 时代不可能发生，
    // 现在必须有启动期信号。ha-core 自身的集成测试进程（init_runtime("test")）
    // 无法 wire 特征 crate（会构成循环依赖），必然触发此 warn——test role
    // 直接跳过，防止「已知噪音」把真实漏接淹掉（警报疲劳比没警报更糟）。
    if crate::runtime_role() == Some("test") {
        return;
    }
    let table = frozen();
    for def in super::dispatch::all_dispatchable_tools() {
        if !table.contains_key(def.name.as_str()) {
            app_warn!(
                "tools",
                "registry_freeze",
                "tool `{}` has a ToolDefinition but no dispatch handler — did this binary forget to wire() its feature crate?",
                def.name
            );
        }
    }
}

/// 拥有 `ToolDefinition` 的工具名全集：静态 catalog + 条件注入的
/// definition（`workflow` 是 session-gated 注入、刻意不进
/// `all_dispatchable_tools`，但它有独立 definition 与专属可见性门
/// `workflow_visibility_error`，不属于「缺 definition」）。
fn defined_tool_names() -> std::collections::HashSet<String> {
    let mut defined: std::collections::HashSet<String> = super::dispatch::all_dispatchable_tools()
        .iter()
        .map(|def| def.name.clone())
        .collect();
    defined.insert(super::get_workflow_tool().name);
    defined
}

fn frozen() -> &'static HashMap<&'static str, RegisteredTool> {
    FROZEN.get_or_init(|| {
        let mut map: HashMap<&'static str, RegisteredTool> = HashMap::new();
        let mut insert_entry = |entry: &BuiltinToolEntry| {
            let mut insert = |name: &'static str| {
                let tool = RegisteredTool {
                    canonical: entry.name,
                    handler: entry.handler,
                };
                if map.insert(name, tool).is_some() {
                    // 重复注册是装配 bug：两个来源都以为自己拥有该工具名。
                    // fail-loud 而不是静默以后者覆盖（覆盖方向取决于遍历顺序，
                    // 结果不可预测）。
                    panic!("duplicate tool registration: {name}");
                }
            };
            insert(entry.name);
            for alias in entry.aliases {
                insert(alias);
            }
        };
        for entry in super::builtin_registry::builtin_entries() {
            insert_entry(&entry);
        }
        // take() 置 None：从这一刻起注册通道关闭（与队列消费同锁原子）。
        let external = PENDING_EXTERNAL
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .unwrap_or_default();
        for entry in external {
            insert_entry(&entry);
        }
        map
    })
}

/// 把 match 臂表达式包成注册项 handler 的样板宏。`$body` 是在 async 块内
/// 求值的**完整表达式**（含 `.await` / `Ok(...)`，与原 match 臂逐字一致）：
/// `tool_handler!(|args, ctx| exec::tool_exec(args, ctx).await)`。
#[macro_export]
macro_rules! tool_handler {
    (|$args:ident, $ctx:ident| $body:expr) => {{
        fn handler<'a>(
            $args: &'a serde_json::Value,
            $ctx: &'a $crate::tools::ToolExecContext,
        ) -> $crate::tools::registry::BuiltinToolFuture<'a> {
            // 不少条目（feishu 系等）不接 ctx——统一在此消化 unused 告警，
            // 免得 135 个条目各自写 `_ctx` 分裂命名。
            let _ = &$ctx;
            Box::pin(async move { $body })
        }
        handler as $crate::tools::registry::BuiltinToolHandler
    }};
}
// `#[macro_export]` + 此处再导出：特征 crate（ha-channel 的 35 条飞书条目）
// 在自己的 `wire()` 里建注册项，需要同一个样板宏——否则每条都要手写一遍
// `fn handler<'a>(...)`。`macro_export` 把它挂到 crate 根，这行让
// `ha_core::tools::registry::tool_handler!` 的就近路径也可用。
pub use crate::tool_handler;

#[cfg(test)]
mod tests {
    use super::*;

    /// 别名必须归一到规范名（执行门的 fate / deny / allowlist 判定都依赖
    /// 这条链路——漏一个别名 = 该别名滑过 `resolve_tool_fate` 兜底）。
    #[test]
    fn aliases_resolve_to_canonical_names() {
        let cases = [
            ("read_file", crate::tools::TOOL_READ),
            ("write_file", crate::tools::TOOL_WRITE),
            ("patch_file", crate::tools::TOOL_EDIT),
            ("list_dir", crate::tools::TOOL_LS),
            (crate::tools::TOOL_NOTE_MOVE, crate::tools::TOOL_NOTE_RENAME),
        ];
        for (alias, expected) in cases {
            assert_eq!(canonical_name(alias), Some(expected), "alias {alias}");
        }
        // 规范名自身命中；未注册名（MCP / 未知）返 None。
        assert_eq!(
            canonical_name(crate::tools::TOOL_READ),
            Some(crate::tools::TOOL_READ)
        );
        assert_eq!(canonical_name("mcp__srv__tool"), None);
        assert_eq!(canonical_name("no_such_tool"), None);
    }

    /// builtin 全表的规范名必须都有 `ToolDefinition`——否则
    /// `resolve_tool_fate` 可见性兜底对该工具 no-op（`tools.allow/deny`
    /// 失效）。外部注册项由 `freeze_now` 的运行期 warn 覆盖；builtin 在
    /// 这里升格为测试强制，新增工具漏配 definition 直接红。
    #[test]
    fn every_builtin_canonical_name_has_a_definition() {
        let defined = defined_tool_names();
        let missing: Vec<&str> = super::super::builtin_registry::builtin_entries()
            .iter()
            .map(|entry| entry.name)
            .filter(|name| !defined.contains(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "builtin tools lacking a ToolDefinition (fate fallback no-op): {missing:?}"
        );
    }
}
