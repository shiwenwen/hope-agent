//! 评测运行时特征 crate（阶段 5 第四刀，自 ha-core 迁出）：coding 评测
//! fixture runner / gold task pack / strategy 对照（`coding_eval`）、评测编排
//! 与制品仓（`evaluation`，自带 `evals.db`）、任务感知的只读上下文排序
//! （`context_retrieval`）。
//!
//! # 为什么叫「运行时」而不是「improve」
//!
//! 原拆分方案把 `coding_improvement` / `domain_eval` / `domain_quality` 一并
//! 划进一个 `ha-improve`。实际摸底否了：那三个模块共 100 处直接
//! `self.conn.lock()`（含 `conn.transaction()`）写 kernel 的 `sessions.db`，
//! 搬走就必须把 `SessionDB` 的**可写连接**开成跨 crate 公开 API——那会永久
//! 击穿封装（拿到句柄即可绕过 kernel 对 `sessions` / `messages` 的不变量与
//! 事务边界），也会推翻 ha-dash 那刀立下的「特征 crate 不碰 kernel 连接」
//! 契约（ha-dash 因此被逼去自开**只读**连接）。
//!
//! 所以这一刀只收**不碰 kernel 连接**的那三块，名字如实反映内容：
//! improve 域尚未拆完，`coding_improvement` / `domain_eval` / `domain_quality`
//! 仍在 kernel，后续单独设计 typed repository / store 边界再切，
//! **不拿通用 `with_conn` 当过渡方案**。
//!
//! # kernel 侧留存
//!
//! - **`ha_core::coding_eval_defs`**——评测 wire 类型的契约层。kernel 的
//!   `coding_improvement` 存的就是 `GoldTaskPackReport` / `StrategyEffectReport`
//!   的 JSON，提案晋升还要按 `CodingEvalFixture` 校验，类型跟着上浮会成环。
//!   本 crate 的 [`coding_eval`] 对它 glob 再导出，既有路径逐字不变。
//! - **`ha_core::review` / `verification` / `domain_workflow` / `lsp`**——
//!   控制面本体与诊断源，`context_retrieval` 反过来依赖它们（特征 → kernel
//!   单向，合法）。
//!
//! # 无 `wire()`
//!
//! 本 crate **没有任何反向钩子**：kernel 对这三个模块零引用，能力面全部经
//! 壳层暴露（Tauri 命令 / HTTP 路由 / `hope-agent-eval`）。因此它是目前唯一
//! 不需要壳层 `wire()` 装配的特征 crate——**不要**为了「和别人一样」补一个
//! 空的 `wire()`，那只会让漏调 `wire()` 的真问题更难被发现。

pub mod coding_eval;
pub mod context_retrieval;
pub mod evaluation;
