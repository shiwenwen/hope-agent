//! 本地模型特征 crate（阶段 5 首刀，自 ha-core 迁出）：Ollama 生命周期
//! （检测 / 安装 / 启动 / 拉取 / 预载）、模型目录与硬件预算推荐、Ollama
//! Library 元数据抓取、默认模型自维护 watchdog、本地 embedding 后端与其
//! 模型下载执行器。
//!
//! kernel 侧留存：`ha_core::local_model_jobs`（**通用后台任务台账**——DB /
//! 快照类型 / spawn / finish / 进度写入 / 取消暂停 / replay），它同时服务
//! kernel 的 `memory::reembed_job` 与知识库 reembed，本就不是 Ollama 专属；
//! 本 crate 只带走 Ollama 执行器（`local_llm::jobs`）。两者分家是阶段 4
//! 破环的最后一步，见 [backend-separation]。
//!
//! 依赖方向：本 crate → ha-core。`local_llm → knowledge` 是既有的**特征间
//! 单向边**（embedding 模型下载完成后触发知识库 reembed）——knowledge 目前
//! 仍在 ha-core 内，故现阶段表现为普通的 `ha_core::knowledge::…` 调用；等
//! ha-knowledge 拆出后改为 crate 间依赖，方向不变。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。
//!
//! [backend-separation]: ../../../docs/architecture/backend-separation.md

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-media 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod local_embedding;
pub mod local_llm;

/// 幂等装配：注册默认模型自维护 watchdog 的启动任务。
///
/// 档位 `PrimaryOnly`，primary-only 的理由不变：两个进程同时预载同一模型
/// 只会互相抢占，且 secondary 透过共享的 Ollama 守护进程看到的是同一份
/// running 状态。
///
/// **执行点相对迁移前前移了**（原代码是
/// `app_init::start_background_tasks` **末尾独立的**一个 `if primary { … }`，
/// 现在走函数中段的 `run_registered_startup_tasks(PrimaryOnly)`）——与
/// ha-media / ha-acp「原位就在那个 primary 块里」的情形不同，别照抄那句
/// 注释。等价性靠两条：primary 门相同（该档只此一处消费，ACP 路径不消费，
/// 与原先 ACP 不调 `spawn_loop` 一致），且 [`local_llm::auto_maintainer::
/// spawn_loop`] 先 sleep 一个 60s 的 `SWEEP_INTERVAL` 才跑第一轮，而中间
/// 那二百多行在 `start_background_tasks` 自身层面无 `.await`（全是 spawn）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            local_llm::auto_maintainer::spawn_loop,
        )
        .expect("ha_local_llm::wire() registers the auto-maintainer startup task exactly once");
    });
}
