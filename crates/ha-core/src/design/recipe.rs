//! 设计模板（Recipe）：某产物形态的生成指引，供 agent `list_recipes` / `get_recipe`
//! 参考后产出结构良好的产物。
//!
//! Phase 3 为**内置 in-code 目录**（覆盖 8 种 kind 的常见场景）；用户自建 `RECIPE.md`
//! 目录在后续迭代接入（managed 目录）。命名 / 内容均原创，不引用任何外部实现。

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
    ]
}

pub fn get_recipe(id: &str) -> Option<Recipe> {
    builtin_recipes().into_iter().find(|r| r.id == id)
}
