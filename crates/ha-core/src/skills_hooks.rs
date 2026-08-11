//! 技能机器层的**反向钩子**——kernel → ha-skills 的唯一回调面。
//!
//! # 为什么只有九槽
//!
//! 阶段 5 第七刀把内置技能解包、SKILL.md 扫描与解析、创作写盘、自动复盘
//! 流水线、`@skill` 提及、fork 派发、GUI / HTTP 命令面与 `skill` 工具自
//! ha-core 迁入 ha-skills。analyzer 迁出前对本组报 19 条需切边 + 16 条装配边，
//! 但**契约 + 台账 + 纯谓词留 kernel** 后（[`crate::skills`] 的
//! `types` / `activation` / `requirements` / `prompt` / `slash`）绝大多数
//! 直接消失：
//!
//! - `tools::execution` 的条件技能激活块（5 条）——只有「取目录」那一步是
//!   钩子，`skill_cache_version` / `activate_skills_for_paths` /
//!   `bump_skill_version` 全在 kernel，`HAS_PATHS_SKILLS_CACHE` 快路径原地不动。
//! - `session::cleanup_watcher`（2 条）——`clear_session_activation` 是台账
//!   操作，**一行未改**，零钩子。
//! - `system_prompt`（2 条）——`build_skills_prompt` / `activated_skill_names`
//!   留 kernel，只有 `load_all_skills_with_budget` 走钩子。
//! - `slash_commands` 的 16 条装配边——`SkillEntry` / `resolve_skill_command_names`
//!   / `check_requirements_detail` / `format_requirements_diagnostic` 全是契约与
//!   纯谓词，只剩「取目录」「内联 SKILL.md」「fork」三处真行为。
//!
//! # 台账不在这里
//!
//! [`crate::skills::activation`] 是 `session_skill_activation` 表 + 进程内热
//! 缓存的真相源，**留 kernel、不走钩子**。三个 kernel 调用点读写它（工具执行
//! 写、system prompt 读、会话清理清），`SessionDB` 也在 kernel——把它挪出去再
//! 钩回来，等于给「会话删除必须同时清 DB 行与热缓存」这条不变量
//! （skill-system.md「清理时机」）加一条「未装配即漏清」的旁路。
//!
//! 同理 [`crate::skills::requirements`] / [`prompt`](crate::skills::prompt) /
//! [`slash`](crate::skills::slash)：纯谓词与纯渲染，不碰文件系统 / LLM / 网络，
//! 放钩子后面只会凭空多出「未装配语义」。
//!
//! # 未装配语义（逐槽，刻意不统一）
//!
//! - **装配一槽**（[`spawn_auto_curator_loop`]）：no-op。等价于「这个进程没有
//!   技能机器」——与 acp / mcp / eval 形态压根不跑 `start_background_tasks` 同义。
//! - [`invocable_skills`] / [`load_all_skills_with_budget`]：返回**空目录**。
//!   逐位等价于迁出前「`skills/` 下一个 SKILL.md 都没有」——所有调用点
//!   （slash 菜单、system prompt 段、条件激活）本就按空目录写的。
//! - [`auto_review_post_turn`]：no-op（不复盘，不落 draft）。
//! - [`render_skill_inline`] / [`spawn_skill_fork`] / [`create_managed_skill_draft`]
//!   / [`set_managed_skill_status`]：返回 **`Err`**。四者都是用户或学习循环
//!   显式触发的**激活 / 写**动作，静默成功会让 slash 命令回一段空正文、
//!   让 Coding Improvement 的 promotion 记下一条根本没落盘的 artifact。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::OnceLock;

use crate::skills::{SkillEntry, SkillStatus};
use ha_config_schema::skills::{SkillPromptBudget, SkillsAutoReviewConfig};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 八槽**原子注册**：行为 7 + 装配 1。
///
/// 与 `knowledge_hooks` / `channel_hooks` / `slash_hooks` 同样一次性注册整组——
/// 部分注册＝一部分技能路径活着、另一部分静默返空目录，是最难查的破法。
pub struct SkillsHooks {
    // ——— 行为 7 ———
    /// `skills::get_invocable_skills`——扫描内置 + 托管 + 额外目录，产出模型 /
    /// slash 可调用的技能目录。四个 kernel 调用点共用（slash 菜单、`/help`、
    /// slash 分发、条件激活快路径）。
    pub invocable_skills: fn(extra_dirs: &[String], disabled: &[String]) -> Vec<SkillEntry>,
    /// `skills::load_all_skills_with_budget`——system prompt 用的目录（含预算
    /// 裁剪）。与 [`Self::invocable_skills`] 是两条不同的过滤链，不能合并。
    pub load_all_skills_with_budget:
        fn(extra_dirs: &[String], budget: &SkillPromptBudget) -> Vec<SkillEntry>,
    /// Typed-composer variant. Names already passed source-anchor validation in
    /// `prompt_context`; the feature layer still enforces its allowlist,
    /// enabled state, requirements, and OS gate.
    pub resolve_named_skill_mentions: fn(
        names: &[String],
        agent_id: Option<&str>,
    ) -> Option<crate::skills::MentionSkillActivation>,
    /// `tools::skill::render_inline`——读 SKILL.md + `$ARGUMENTS` 替换。
    /// slash 命令与 `skill` 工具共用同一份激活正文（两者必须逐字节相同）。
    pub render_skill_inline:
        for<'a> fn(&'a SkillEntry, &'a str) -> BoxFut<'a, anyhow::Result<String>>,
    /// `skills::spawn_skill_fork`——`context: fork` 技能派子 agent，返回 run id。
    pub spawn_skill_fork: for<'a> fn(
        skill: &'a SkillEntry,
        args: &'a str,
        parent_session_id: &'a str,
        agent_id: &'a str,
        skip_parent_injection: bool,
    ) -> BoxFut<'a, anyhow::Result<String>>,
    /// 自动复盘五闸瀑布的**闸 1 之后全部**（`touch_and_maybe_trigger` →
    /// `tokio::spawn(run_review_cycle)` → `sweep_stale`）。kernel 只算四个信号
    /// 标量——其中 `user_correction` 需要 `SessionDB::user_messages_within`，
    /// 而那个 `SessionDB` 是 `ChatEngineParams` 传进来的实例（测试用
    /// ephemeral，不等于 `globals::get_session_db()`），故必须留 kernel 侧算。
    ///
    /// **`cfg` 也由 kernel 传入而非两侧各读一次**：迁出前整块共用同一个
    /// `cached_config().skills.auto_review.clone().sanitize()` 快照，correction
    /// 门与闸 1 阈值必须来自同一份配置。它是 ha-config-schema 的 wire 类型，
    /// 两侧都认识，过边界零成本。
    pub auto_review_post_turn: fn(
        session_id: &str,
        cfg: &SkillsAutoReviewConfig,
        turn_tokens: usize,
        new_messages: usize,
        tool_use_count: usize,
        user_correction: bool,
    ),
    /// `skills::author::create_skill` 的 draft 特化——Coding Improvement 的
    /// `create_managed_skill_draft` 动作专用，故 `CreateOpts` 的三个字段在
    /// 调用点是定值（`status: Draft` / `authored_by: "coding-improvement"`），
    /// 签名收窄后 `CreateOpts` 不必跨 crate。
    pub create_managed_skill_draft: fn(
        skill_id: &str,
        description: &str,
        body_md: &str,
        rationale: Option<String>,
    ) -> anyhow::Result<PathBuf>,
    /// `skills::author::set_skill_status`——Coding Improvement 的
    /// `activate_managed_skill` promotion 动作。
    pub set_managed_skill_status: fn(skill_id: &str, status: SkillStatus) -> anyhow::Result<()>,

    // ——— 装配 1（在 app_init 的原调用位触发，逐位保序）———
    /// `skills::auto_review::curator::spawn_auto_curator_loop`——draft 归并循环。
    /// 迁移前在 primary-only 块内、紧跟外部记忆同步循环，位置由 kernel 侧
    /// 调用点保持。
    pub spawn_auto_curator_loop: fn(),
}

static SKILLS_HOOKS: OnceLock<SkillsHooks> = OnceLock::new();

/// 装配期注册技能机器钩子（八槽原子）。重复注册返回 `Err`。
pub fn register_skills_hooks(hooks: SkillsHooks) -> Result<(), crate::AlreadyRegistered> {
    SKILLS_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("skills machinery hooks"))
}

fn hooks() -> Option<&'static SkillsHooks> {
    SKILLS_HOOKS.get()
}

const NOT_WIRED: &str = "skills machinery not wired (ha_skills::wire() was not called)";

/// 模型 / slash 可调用的技能目录。未装配即**空目录**。
pub fn invocable_skills(extra_dirs: &[String], disabled: &[String]) -> Vec<SkillEntry> {
    match hooks() {
        Some(h) => (h.invocable_skills)(extra_dirs, disabled),
        None => Vec::new(),
    }
}

/// system prompt 用的技能目录（含预算裁剪）。未装配即**空目录**。
pub fn load_all_skills_with_budget(
    extra_dirs: &[String],
    budget: &SkillPromptBudget,
) -> Vec<SkillEntry> {
    match hooks() {
        Some(h) => (h.load_all_skills_with_budget)(extra_dirs, budget),
        None => Vec::new(),
    }
}

/// Resolve already-validated typed skill ids. Unwired machinery yields no
/// activation (fail closed).
pub fn resolve_named_skill_mentions(
    names: &[String],
    agent_id: Option<&str>,
) -> Option<crate::skills::MentionSkillActivation> {
    match hooks() {
        Some(h) => (h.resolve_named_skill_mentions)(names, agent_id),
        None => None,
    }
}

/// 读 SKILL.md + `$ARGUMENTS` 替换。**未装配返回 `Err`**——用户显式激活了技能，
/// 静默返空正文会让 LLM 以为技能是空的。
pub async fn render_skill_inline(skill: &SkillEntry, args: &str) -> anyhow::Result<String> {
    match hooks() {
        Some(h) => (h.render_skill_inline)(skill, args).await,
        None => Err(anyhow::anyhow!(NOT_WIRED)),
    }
}

/// `context: fork` 技能派子 agent。**未装配返回 `Err`**（同上，用户显式触发）。
pub async fn spawn_skill_fork(
    skill: &SkillEntry,
    args: &str,
    parent_session_id: &str,
    agent_id: &str,
    skip_parent_injection: bool,
) -> anyhow::Result<String> {
    match hooks() {
        Some(h) => {
            (h.spawn_skill_fork)(
                skill,
                args,
                parent_session_id,
                agent_id,
                skip_parent_injection,
            )
            .await
        }
        None => Err(anyhow::anyhow!(NOT_WIRED)),
    }
}

/// 轮末自动复盘触发（五闸瀑布闸 1 起）。未装配即 no-op。
pub fn auto_review_post_turn(
    session_id: &str,
    cfg: &SkillsAutoReviewConfig,
    turn_tokens: usize,
    new_messages: usize,
    tool_use_count: usize,
    user_correction: bool,
) {
    if let Some(h) = hooks() {
        (h.auto_review_post_turn)(
            session_id,
            cfg,
            turn_tokens,
            new_messages,
            tool_use_count,
            user_correction,
        );
    }
}

/// 落一份 draft 托管技能（Coding Improvement）。**未装配返回 `Err`**——
/// 静默成功会让 promotion 记下一条根本没落盘的 artifact。
pub fn create_managed_skill_draft(
    skill_id: &str,
    description: &str,
    body_md: &str,
    rationale: Option<String>,
) -> anyhow::Result<PathBuf> {
    match hooks() {
        Some(h) => (h.create_managed_skill_draft)(skill_id, description, body_md, rationale),
        None => Err(anyhow::anyhow!(NOT_WIRED)),
    }
}

/// 改托管技能状态（Coding Improvement 的 promotion）。**未装配返回 `Err`**。
pub fn set_managed_skill_status(skill_id: &str, status: SkillStatus) -> anyhow::Result<()> {
    match hooks() {
        Some(h) => (h.set_managed_skill_status)(skill_id, status),
        None => Err(anyhow::anyhow!(NOT_WIRED)),
    }
}

/// draft 归并循环。未装配即 no-op。
pub fn spawn_auto_curator_loop() {
    if let Some(h) = hooks() {
        (h.spawn_auto_curator_loop)();
    }
}
