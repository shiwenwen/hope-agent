//! 设计模板（Recipe）：某产物形态的生成指引，供 agent `list_recipes` / `get_recipe`
//! 参考后产出结构良好的产物。
//!
//! **内置 in-code 目录**（覆盖 9 种 kind 的常见场景 + 域文档 / deck 模板广度）；用户自建
//! `RECIPE.md` 目录在后续迭代接入（managed 目录）。命名 / 内容均原创，不引用任何外部实现。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub scenario: String,
    /// 一句话描述。
    pub summary: String,
    /// 面向 agent 的生成指引（结构 / 要点 / 反 slop）。
    pub guidance: String,
}

fn r(id: &str, name: &str, kind: &str, scenario: &str, summary: &str, guidance: &str) -> Recipe {
    Recipe {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        scenario: scenario.to_string(),
        summary: summary.to_string(),
        guidance: guidance.to_string(),
    }
}

/// 通用生成约束（拼进每个 recipe guidance 头部时用）。
pub const COMMON_GUIDANCE: &str = "\
产出**自包含 HTML**：结构写进 body_html，样式写进 css（**引用设计系统变量** var(--ds-color-primary) 等，未提供则用合理默认），可选交互写进 js。\
**禁止引用任何外部 CDN / 网络资源**（沙箱零网络）；图片用内联 SVG 或 CSS 渐变占位。\
真实、具体、克制：不要占位文案（Lorem ipsum）、不要雷同区块、保证对比度与层次。";

/// 内置目录。
pub fn builtin_recipes() -> Vec<Recipe> {
    vec![
        r(
            "web-landing",
            "落地页",
            "web",
            "marketing",
            "含 hero、特性、行动号召的单页落地页",
            "结构：顶部导航 + hero（主标题/副标题/主按钮）+ 3–4 个特性卡 + 社会证明 + 页脚 CTA。视觉有节奏、留白充足。",
        ),
        r(
            "web-saas",
            "SaaS 首页",
            "web",
            "product",
            "SaaS 产品首页：hero + 功能 + 定价",
            "结构：hero + 关键指标 + 功能分区（图文交替）+ 定价三档卡 + FAQ + 页脚。定价卡突出推荐档。",
        ),
        r(
            "mobile-onboarding",
            "移动引导流",
            "mobile",
            "product",
            "移动 App 启动 + 引导 + 登录",
            "结构：390×844 内多屏（可用多个 section 叠加/切换）。启动页 → 3 屏价值介绍 → 登录/注册。底部主按钮，尊重安全区。",
        ),
        r(
            "mobile-app",
            "移动应用界面",
            "mobile",
            "product",
            "带底部导航的移动应用主界面",
            "结构：顶部标题栏 + 内容列表/卡片 + 底部 tab 栏（4–5 项）。触控目标 ≥44px，圆角友好。",
        ),
        r(
            "deck-pitch",
            "路演演示",
            "deck",
            "product",
            "融资/产品路演演示文稿",
            "每页一个 <section class=\"ds-slide\">。顺序：封面 → 问题 → 方案 → 演示 → 市场 → 商业模式 → 团队 → 结语。每页一个核心观点，大字少字。",
        ),
        r(
            "deck-report",
            "汇报演示",
            "deck",
            "operation",
            "工作/数据汇报演示文稿",
            "每页 <section class=\"ds-slide\">：封面 → 概览 → 分主题（每题结论先行 + 图表/要点）→ 下一步。图表用内联 SVG。",
        ),
        r(
            "dashboard-admin",
            "管理后台仪表盘",
            "dashboard",
            "operation",
            "带侧边栏的数据仪表盘",
            "结构：左侧导航 + 顶部筛选 + KPI 卡行 + 图表网格（内联 SVG 折线/柱状/饼）+ 明细表。信息密度高但有层次。",
        ),
        r(
            "poster-social",
            "社交海报",
            "poster",
            "marketing",
            "1080×1080 社交媒体图文",
            "定尺容器。大标题 + 视觉主体（内联 SVG / 渐变）+ 品牌角标。构图有焦点，文字可读。",
        ),
        r(
            "document-spec",
            "产品规格文档",
            "document",
            "product",
            "带目录的产品规格/PRD",
            "结构：标题 + 元信息 + 目录 + 分章节（背景/目标/方案/边界/验收）。排版专业，标题层级清晰。",
        ),
        r(
            "email-marketing",
            "营销邮件",
            "email",
            "marketing",
            "table 布局的营销邮件",
            "用 table 布局（邮件客户端兼容）。600 宽。头图 + 标题 + 正文 + 主按钮 + 页脚。内联样式，避免复杂 CSS。",
        ),
        r(
            "email-transactional",
            "事务邮件",
            "email",
            "operation",
            "通知/回执类事务邮件（table 布局）",
            "600 宽 table 布局。品牌头 + 标题（如「订单已确认」）+ 关键信息块（订单号/金额/时间，用 table 行）+ 主按钮（查看详情）+ 帮助页脚。克制、可信、无营销噪声。内联样式。",
        ),
        // ── 域文档（追齐 PM spec / OKR / runbook / finance / HR / RFC 广度）──
        r(
            "document-okr",
            "OKR 记分卡",
            "document",
            "operation",
            "季度 OKR 目标与关键结果记分卡",
            "结构：标题 + 周期/负责人元信息 + 每个 Objective 一个卡片（目标句 + 3–5 个 Key Result 带进度条 + 当前值/目标值 + 信心色标 绿/黄/红）+ 总体进度摘要。进度条用纯 CSS，色标语义清晰。",
        ),
        r(
            "document-runbook",
            "工程 Runbook",
            "document",
            "operation",
            "运维/事故处置 Runbook",
            "结构：标题 + 适用范围/严重级 + 前置检查清单 + 编号处置步骤（每步：动作 + 预期结果 + 命令块 用等宽样式）+ 回滚步骤 + 升级联系人表。步骤可勾选感、命令块可读、危险步骤高亮。",
        ),
        r(
            "document-report",
            "数据/财务报告",
            "document",
            "operation",
            "带图表的分析/财务报告",
            "结构：封面标题 + 执行摘要（要点先行）+ 关键指标卡行 + 分析章节（每节：结论 + 内联 SVG 图表 + 简短解读）+ 附录/口径说明。图表用内联 SVG（折线/柱状/瀑布），数字对齐、单位清晰。",
        ),
        r(
            "document-onboarding",
            "入职计划",
            "document",
            "operation",
            "新人入职 30/60/90 天计划",
            "结构：欢迎语 + 角色/导师信息 + 三阶段时间线（30/60/90 天，每阶段目标 + 任务清单 + 里程碑）+ 关键联系人 + 资源链接清单。阶段用色带区分，任务可勾选感、节奏清晰。",
        ),
        r(
            "document-rfc",
            "决策记录 / RFC",
            "document",
            "product",
            "技术决策记录（RFC / ADR）",
            "结构：标题 + 状态徽标（草案/已批准/已废弃）+ 背景与问题 + 方案对比（表格：选项 × 优劣/成本）+ 决策与理由 + 影响与迁移 + 决策日志（时间线）。表格对齐、状态徽标醒目、理由充分。",
        ),
        r(
            "web-editorial",
            "编辑长文",
            "web",
            "content",
            "杂志/编辑风格长文阅读页",
            "结构：大标题 + 作者/日期/阅读时长 + 首字下沉引导段 + 正文（大行高、舒适测量宽度 60–75 字符）+ 图注/引文块 + 章节小标 + 结尾。排版为王：层次、留白、引文强调，阅读体验优先。",
        ),
        // ── deck 模板广度（模板/布局；视觉主题走设计系统）──
        r(
            "deck-keynote",
            "主题演讲",
            "deck",
            "marketing",
            "大字主题演讲式演示",
            "每页 <section class=\"ds-slide\">，极简大字风：封面大标题 → 每页一个观点（超大字 + 一行支撑 + 可选大图/SVG）→ 金句页 → 收尾行动号召。每页信息极少、视觉冲击强、对比度高。",
        ),
        r(
            "deck-timeline",
            "路线图演示",
            "deck",
            "operation",
            "时间线 / 路线图演示",
            "每页 <section class=\"ds-slide\">：封面 → 总览时间线（横向阶段条）→ 每阶段一页（目标 + 交付物 + 时间）→ 里程碑页 → 风险与依赖。时间线用纯 CSS/SVG，阶段进度与顺序一目了然。",
        ),
        r(
            "deck-comparison",
            "对比演示",
            "deck",
            "product",
            "方案/竞品对比演示",
            "每页 <section class=\"ds-slide\">：封面 → 评估维度说明 → 对比表页（选项 × 维度，用色标/勾叉）→ 逐项深入页 → 推荐结论页。对比表清晰对齐、优势项高亮、结论有据。",
        ),
        r(
            "deck-datastory",
            "数据故事",
            "deck",
            "operation",
            "以图表驱动的数据故事演示",
            "每页 <section class=\"ds-slide\">：封面 → 背景问题 → 每页一个图表 + 一句洞察（内联 SVG 折线/柱状/散点）→ 转折/对比页 → 结论与建议。一页一图一结论，图表诚实、洞察先行。",
        ),
        r(
            "dashboard-analytics",
            "分析仪表盘",
            "dashboard",
            "operation",
            "指标分析型仪表盘",
            "结构：顶部时间/维度筛选 + 核心指标卡行（含环比箭头）+ 主图表区（趋势大图）+ 次级图表网格 + 维度明细表。内联 SVG 图表，环比涨跌用色，信息密度高但主次分明。",
        ),
        r(
            "poster-event",
            "活动海报",
            "poster",
            "marketing",
            "活动/发布会海报",
            "定尺容器（默认 1080×1080，竖版可 1080×1350）。主视觉（渐变/几何 SVG）+ 活动名大标题 + 时间地点信息块 + 嘉宾/亮点 + 二维码占位 + 品牌角标。构图有焦点、信息层级清晰、可读性强。",
        ),
        r(
            "motion-kinetic",
            "动态标题",
            "motion",
            "marketing",
            "1280×720 动态排版短片（纯 CSS/JS 动画）",
            "在 .ds-stage（1280×720）内用**纯 CSS/JS 动画**做动态排版：文字/形状依次入场、位移、淡入淡出，形成 5–10 秒短片。用 CSS @keyframes / transform / opacity（60fps），可选 requestAnimationFrame 编排时间线。零外部依赖。**在 .ds-stage 上加 `data-ds-duration=\"8000\"`（毫秒）声明总时长**——设计空间支持一键导出为 **MP4**（客户端逐帧编码，无需 ffmpeg），声明时长后导出更精确。",
        ),
        r(
            "motion-reveal",
            "产品揭示动画",
            "motion",
            "product",
            "1280×720 产品/功能揭示动画",
            "在 .ds-stage 内做产品揭示：背景渐变缓动 + 主体元素缩放/滑入 + 标语逐字打出 + 收尾定格。全程 transform/opacity 动画（60fps），时间线 6–12 秒。零外部依赖、自包含。在 .ds-stage 加 `data-ds-duration`（毫秒）声明总时长，便于一键导出 MP4。",
        ),
    ]
}

pub fn get_recipe(id: &str) -> Option<Recipe> {
    builtin_recipes().into_iter().find(|r| r.id == id)
}
