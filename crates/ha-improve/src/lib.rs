//! 学习闭环特征 crate（阶段 5 第八刀，自 ha-core 迁出）：Coding 改进提案
//! 队列、领域评测 fixture / campaign 跑批、领域质量复核，以及建立在它们之上的
//! 四道确定性闸（持续基准 / 领域质量 / 领域就绪 / 领域运营）与 soak 报表。
//!
//! # 分法：台账留 kernel，机器上浮
//!
//! 这一组是全部拆分里唯一被 AGENTS 红线**点名挡过**的：三个模块共 100 处
//! `self.conn.lock()`，红线要求「等 typed repository 边界设计好再切」。本刀
//! 交付的就是那条边界——对三个 `impl SessionDB` 块的 155 个方法做不动点：
//!
//! - **种子**：方法体里直接出现 `self.conn` / `Connection` / `rusqlite::` 的，
//!   留 kernel。
//! - **迭代**：留守方法调用到的方法，也留 kernel。
//! - **收敛**：剩下的 34 个入口一处连接都不碰（32 个原 `impl SessionDB` 方法
//!   + 2 个只被壳层调用、不动点从方法出发够不着的自由函数），正好是这一组的
//!   **顶层入口**（四道闸、提案流水线、报表与执行器）；留下的 123 个方法全是
//!   纯 SQL 访问层。反方向同样有一处自由函数要补判：
//!   `persist_domain_eval_fixture_report` 自己锁连接写表，已收编成 kernel 的
//!   第 124 个类型化方法（而不是留作一个公开的自由 SQL 入口）。
//!
//! 因此 `SessionDB::with_conn_internal` 保持 `pub(crate)` 不动，本 crate 只经
//! **类型化仓储方法**触达 `sessions.db`——**生产代码零裸连接、零直接 SQL**
//! （测试 fixture 走 `test-support` 门控的 `with_conn_for_test`）。
//!
//! **判据不是「不写 `sessions` / `messages`」**——`run_domain_eval_fixture`
//! 照样 `create_session` / `set_session_kind` / `append_message` /
//! `create_chat_turn_with_id`。成立的是更强也更准的那条：**每一次这类写都走
//! kernel 的类型化方法**，不变量与事务边界仍由 kernel 独占。红线担心的是拿到
//! 裸句柄后绕过它们，而本 crate 根本没有句柄（rusqlite 只在 dev-dependencies）。
//! 三个模块自身的 SQL 只碰 `coding_*` / `domain_*` 私有表，21 处
//! `JOIN sessions` 全是只读聚合。
//!
//! 固有 impl 只能待在定义 `SessionDB` 的 crate 里，所以那 32 个方法在本 crate
//! 里是自由函数 `fn f(db: &SessionDB, …)`——这是本刀的主要机械改动。
//!
//! # 反向回调
//!
//! kernel → 本 crate 只有**一槽**（[`ha_core::improve_hooks`]）：工作流跑到
//! 终态时记一条 coding retro。机器的其余入口全部只被壳层调用，是正向依赖。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

pub mod coding_improvement;
pub mod domain_eval;
pub mod domain_quality;

/// 幂等装配：一处接线——`improve_hooks` 单槽。
///
/// 本 crate **不注册任何工具 handler**：学习闭环全是 owner 平面
/// （Tauri 命令 / HTTP 路由 / `hope-agent-eval`），模型侧没有对应工具。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::improve_hooks::register_improve_hooks(ha_core::improve_hooks::ImproveHooks {
            ensure_coding_workflow_retro_for_run:
                coding_improvement::ensure_coding_workflow_retro_for_run,
        })
        .expect("ha_improve::wire() registers the improve hooks exactly once");
    });
}
