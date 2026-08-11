//! 技能机器特征 crate（阶段 5 第七刀，自 ha-core 迁出）：内置技能的编译期
//! 嵌入与解包、SKILL.md 目录扫描与 YAML frontmatter 解析、技能创作
//! （create / update / patch，全过 `security_scan`）、五闸自动复盘流水线与
//! draft 归并 curator、`@skill` 提及注入、`context: fork` 派子 agent、
//! GUI / HTTP 命令面，以及 `skill` 工具。
//!
//! # 分法：契约 + 台账 + 纯谓词留 kernel，机器上浮
//!
//! 留在 [`ha_core::skills`] 的五个模块**都不是「技能行为」**：
//!
//! - **`types`**——`SkillEntry` / `SkillStatus` / `SkillSummary` 等 wire 契约，
//!   连同 `skill_cache_version` / `bump_skill_version` 这对目录版本计数器。
//!   slash 命令表（kernel `slash_commands`）与 GUI / HTTP 命令面共用它们。
//! - **`activation`**——**台账**：`session_skill_activation` 表 + 进程内热缓存
//!   的真相源。三个 kernel 调用点读写（`tools::execution` 写、
//!   `system_prompt::sections` 读、`session::cleanup_watcher` 清），`SessionDB`
//!   也在 kernel。同第五刀 `ChannelDB` / 第六刀 `KnowledgeRegistry` 的效果——
//!   **正因它留下，`tools::execution` 的条件激活块与 cleanup_watcher 的清理
//!   一行未改。**
//! - **`requirements` / `prompt` / `slash`**——对契约类型的纯谓词与纯渲染
//!   （环境依赖检查、prompt 段拼装、slash 名字归一 / 健康度）。不碰文件系统、
//!   不调 LLM、不出网；放进钩子只会凭空多出一层「未装配语义」。
//!
//! 契约留下消掉了绝大多数切边：analyzer 迁出前对本组报 19 条需切边 + 16 条
//! 装配边，真正需要回调的只有九处。
//!
//! # 反向回调
//!
//! kernel → 本 crate 的**唯一**回调面是 [`ha_core::skills_hooks`]（九槽原子
//! 注册）：行为 8 + 装配 1。未装配语义逐槽在该模块文档里论证——目录类返空
//! （等价于一个 SKILL.md 都没有）、复盘 no-op，而**激活 / 写四槽返 `Err`**
//! （用户或学习循环显式触发的动作不能静默成功）。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进制
//! 必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core / ha-knowledge / ha-channel 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod skills;
pub mod tools;

/// 幂等装配：两处接线——`skills_hooks` 九槽 + `skill` 工具的分发条目，
/// 外加一次目录版本 bump（见 [`invalidate_catalog_caches`]）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        register_hooks();
        register_skill_tool();
        invalidate_catalog_caches();
    });
}

fn register_hooks() {
    use ha_core::skills_hooks as h;

    fn auto_review_post_turn(
        session_id: &str,
        cfg: &ha_config_schema::skills::SkillsAutoReviewConfig,
        turn_tokens: usize,
        new_messages: usize,
        tool_use_count: usize,
        user_correction: bool,
    ) {
        // 与迁出前 `chat_engine::engine` 里那一块逐位相同——kernel 只保留了
        // 四个信号标量的计算（`user_correction` 要那个 caller 传入的 SessionDB）
        // 与**唯一那份**已 sanitize 的配置快照，两侧共用。
        let signals = skills::auto_review::TriggerSignals {
            turn_tokens,
            new_messages,
            tool_use_count,
            user_correction,
        };
        let Some(gate) = skills::auto_review::touch_and_maybe_trigger(session_id, signals, cfg)
        else {
            return;
        };
        let session_id_for_review = session_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = skills::auto_review::run_review_cycle(
                &session_id_for_review,
                skills::auto_review::ReviewTrigger::PostTurn,
                gate,
                None,
            )
            .await
            {
                app_warn!(
                    "skills",
                    "auto_review",
                    "post-turn review cycle failed: {}",
                    e
                );
            }
            skills::auto_review::sweep_stale(7 * 24 * 3600);
        });
    }

    fn create_managed_skill_draft(
        skill_id: &str,
        description: &str,
        body_md: &str,
        rationale: Option<String>,
    ) -> anyhow::Result<std::path::PathBuf> {
        // `CreateOpts` 的另两个字段在 Coding Improvement 调用点是定值，故没有
        // 跨 crate；迁出前那三行就写死在 `apply_skill_candidate_plan` 里。
        skills::author::create_skill(
            skill_id,
            description,
            body_md,
            skills::author::CreateOpts {
                status: ha_core::skills::SkillStatus::Draft,
                authored_by: "coding-improvement".to_string(),
                rationale,
            },
        )
    }

    h::register_skills_hooks(h::SkillsHooks {
        invocable_skills: skills::get_invocable_skills,
        load_all_skills_with_budget: skills::load_all_skills_with_budget,
        resolve_named_skill_mentions: skills::resolve_named_skill_mentions,
        render_skill_inline: |skill, args| Box::pin(tools::skill::render_inline(skill, args)),
        spawn_skill_fork: |skill, args, parent_session_id, agent_id, skip_parent_injection| {
            Box::pin(skills::spawn_skill_fork(
                skill,
                args,
                parent_session_id,
                agent_id,
                skip_parent_injection,
            ))
        },
        auto_review_post_turn,
        create_managed_skill_draft,
        set_managed_skill_status: skills::author::set_skill_status,
        spawn_auto_curator_loop: skills::auto_review::curator::spawn_auto_curator_loop,
    })
    .expect("ha_skills::wire() registers the skills hooks exactly once");
}

fn register_skill_tool() {
    ha_core::tools::registry::register_external_tools(tools::skill_dispatch_entries())
        .expect("ha_skills::wire() registers the skill tool handler before registry freeze");
}

/// 目录刚刚变得可用——bump 一次版本号，作废任何在 `wire()` 之前算出的
/// 「目录里没有 `paths:` 技能」缓存。
///
/// 装配契约要求 `wire()` 先于 `init_runtime`，正常路径下这条是空转
/// （计数器从 0 变 1，没有缓存可作废）。它防的是契约被破坏时那个**静默且
/// 永久**的后果：`tools::execution` 的 `HAS_PATHS_SKILLS_CACHE` 以
/// `skill_cache_version()` 为 key，未装配时 `invocable_skills` 返空目录
/// → 缓存 `(v, false)`；而装配本身不改版本号，于是条件技能激活会在这个
/// 进程里永久失活。一行的保险，比事后从「某些技能就是不亮」倒查便宜得多。
///
/// `bump_skill_version` 的 EventBus emit 在此刻是 no-op（globals 未建，
/// 它自身按 best-effort 写的）。
fn invalidate_catalog_caches() {
    ha_core::skills::bump_skill_version();
}
