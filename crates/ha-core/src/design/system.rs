//! 设计系统层（品牌契约 + Token 编译）。
//!
//! 一个设计系统 = `DESIGN.md`（**DESIGN.md 规范** 9 段 prose + Token 表，真相源，供 LLM
//! grounding，见 `design_md.rs`）+ `tokens.json`（CSS 变量，渲染器注入产物 `:root`）。
//! 见 docs/architecture/design-space.md §6。
//!
//! 内置系统在此**代码内定义**：6 套原创原型语言 + 一批品牌风格参考（`brands.rs`，对各
//! 品牌公开视觉语言的独立再诠释，渲染附免责声明、非官方），首次访问懒 seed 到 managed
//! 目录 + 注册 `design.db`，用户可 fork / 编辑。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::db::{DesignDb, DesignSystemMeta};
use crate::paths;
use crate::platform::write_atomic;

/// 完整设计系统（含正文 + token）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignSystemFull {
    #[serde(flatten)]
    pub meta: DesignSystemMeta,
    /// DESIGN.md 正文（供 LLM 读取 grounding）。
    pub system_md: String,
    /// CSS 变量 token（有序）。
    pub tokens: BTreeMap<String, String>,
    /// 提取时 harvest 的 logo / 配图资产（data-uri）。B1-4；非提取系统为空。
    #[serde(default)]
    pub assets: DesignAssets,
}

/// 设计系统资产（`assets.json`，B1-4）：logo / 配图均为自包含 data-uri。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignAssets {
    #[serde(default)]
    pub logos: Vec<String>,
    #[serde(default)]
    pub images: Vec<String>,
    /// 从来源页 harvest 的 web 字体：每项是一条内嵌 data-uri src 的 `@font-face` CSS 规则
    /// （自包含）。Kit 套件页据此以**真实字体**渲染排版样张（webfont 提取保真）。
    #[serde(default)]
    pub fonts: Vec<String>,
}

/// 落盘/读取系统资产 `assets.json`（写经原子写；读缺失/损坏回退空）。
pub fn write_assets(id: &str, assets: &DesignAssets) -> Result<()> {
    if assets.logos.is_empty() && assets.images.is_empty() && assets.fonts.is_empty() {
        return Ok(());
    }
    let dir = paths::design_system_dir(id)?;
    std::fs::create_dir_all(&dir).ok();
    crate::platform::write_atomic(
        &dir.join("assets.json"),
        serde_json::to_string(assets)?.as_bytes(),
    )
    .map_err(|e| anyhow::anyhow!("write assets.json: {e}"))
}

fn read_assets(id: &str) -> DesignAssets {
    let Ok(dir) = paths::design_system_dir(id) else {
        return DesignAssets::default();
    };
    std::fs::read_to_string(dir.join("assets.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// 内置系统定义（代码内）。
struct Builtin {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    tokens: &'static [(&'static str, &'static str)],
    /// DESIGN.md 正文的气质段（品牌 + 反模式，原创措辞）。
    doc: &'static str,
}

fn builtins() -> Vec<Builtin> {
    vec![
        Builtin {
            id: "minimal-modern",
            name: "极简现代",
            summary: "干净克制、留白充足、单一强调色的现代界面语言",
            tokens: &[
                ("--ds-color-bg", "#ffffff"),
                ("--ds-color-fg", "#0f172a"),
                ("--ds-color-primary", "#2563eb"),
                ("--ds-color-secondary", "#475569"),
                ("--ds-color-accent", "#0ea5e9"),
                ("--ds-color-muted", "#f1f5f9"),
                ("--ds-color-border", "#e2e8f0"),
                ("--ds-color-success", "#16a34a"),
                ("--ds-color-warning", "#d97706"),
                ("--ds-color-danger", "#dc2626"),
                ("--ds-font-sans", "system-ui,-apple-system,'Segoe UI',Roboto,'PingFang SC',sans-serif"),
                ("--ds-font-serif", "Georgia,'Songti SC',serif"),
                ("--ds-font-mono", "ui-monospace,'SF Mono',Menlo,monospace"),
                ("--ds-text-base", "16px"),
                ("--ds-text-lg", "20px"),
                ("--ds-text-xl", "28px"),
                ("--ds-text-2xl", "40px"),
                ("--ds-text-3xl", "56px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "24px"),
                ("--ds-space-8", "48px"),
                ("--ds-radius-md", "10px"),
                ("--ds-radius-lg", "16px"),
                ("--ds-shadow-md", "0 4px 20px rgba(15,23,42,.08)"),
            ],
            doc: "克制、精确。大量留白，单一蓝色强调，层次靠字号与间距而非线条与阴影。避免装饰性元素、避免多强调色、避免拥挤。",
        },
        Builtin {
            id: "editorial",
            name: "编辑杂志",
            summary: "衬线大标题、强对比、栅格化的杂志式版面",
            tokens: &[
                ("--ds-color-bg", "#fbfaf7"),
                ("--ds-color-fg", "#1a1a1a"),
                ("--ds-color-primary", "#b91c1c"),
                ("--ds-color-secondary", "#57534e"),
                ("--ds-color-accent", "#b91c1c"),
                ("--ds-color-muted", "#f0ede6"),
                ("--ds-color-border", "#dcd7cc"),
                ("--ds-color-success", "#15803d"),
                ("--ds-color-warning", "#b45309"),
                ("--ds-color-danger", "#b91c1c"),
                ("--ds-font-sans", "'Helvetica Neue',Arial,'PingFang SC',sans-serif"),
                ("--ds-font-serif", "'Playfair Display',Georgia,'Songti SC',serif"),
                ("--ds-font-mono", "ui-monospace,Menlo,monospace"),
                ("--ds-text-base", "17px"),
                ("--ds-text-lg", "22px"),
                ("--ds-text-xl", "34px"),
                ("--ds-text-2xl", "52px"),
                ("--ds-text-3xl", "76px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "28px"),
                ("--ds-space-8", "56px"),
                ("--ds-radius-md", "2px"),
                ("--ds-radius-lg", "4px"),
                ("--ds-shadow-md", "none"),
            ],
            doc: "杂志感：超大衬线标题、粗横线分隔、多栏栅格、红黑强对比。正文用无衬线小字。少圆角、少阴影，靠排版张力。",
        },
        Builtin {
            id: "tech-dark",
            name: "科技暗色",
            summary: "深色背景、霓虹强调、发光边界的科技/开发者语言",
            tokens: &[
                ("--ds-color-bg", "#0b0f17"),
                ("--ds-color-fg", "#e6edf3"),
                ("--ds-color-primary", "#38bdf8"),
                ("--ds-color-secondary", "#94a3b8"),
                ("--ds-color-accent", "#a78bfa"),
                ("--ds-color-muted", "#161b26"),
                ("--ds-color-border", "#232a37"),
                ("--ds-color-success", "#34d399"),
                ("--ds-color-warning", "#fbbf24"),
                ("--ds-color-danger", "#f87171"),
                ("--ds-font-sans", "'Inter',system-ui,'PingFang SC',sans-serif"),
                ("--ds-font-serif", "Georgia,serif"),
                ("--ds-font-mono", "'JetBrains Mono',ui-monospace,Menlo,monospace"),
                ("--ds-text-base", "15px"),
                ("--ds-text-lg", "19px"),
                ("--ds-text-xl", "26px"),
                ("--ds-text-2xl", "38px"),
                ("--ds-text-3xl", "52px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "24px"),
                ("--ds-space-8", "44px"),
                ("--ds-radius-md", "12px"),
                ("--ds-radius-lg", "18px"),
                ("--ds-shadow-md", "0 0 0 1px rgba(56,189,248,.15),0 8px 30px rgba(0,0,0,.5)"),
            ],
            doc: "深色底、青紫霓虹强调、细发光边框、等宽字点缀。适合开发者工具 / SaaS / AI 产品。避免纯黑纯白，用近黑与柔和前景色护眼。",
        },
        Builtin {
            id: "warm-friendly",
            name: "温暖亲和",
            summary: "暖色调、大圆角、柔和阴影的亲切消费级语言",
            tokens: &[
                ("--ds-color-bg", "#fffaf5"),
                ("--ds-color-fg", "#3a2e28"),
                ("--ds-color-primary", "#f97316"),
                ("--ds-color-secondary", "#a8756a"),
                ("--ds-color-accent", "#14b8a6"),
                ("--ds-color-muted", "#fdeee0"),
                ("--ds-color-border", "#f3ddc9"),
                ("--ds-color-success", "#22c55e"),
                ("--ds-color-warning", "#f59e0b"),
                ("--ds-color-danger", "#ef4444"),
                ("--ds-font-sans", "'Nunito','PingFang SC',system-ui,sans-serif"),
                ("--ds-font-serif", "Georgia,serif"),
                ("--ds-font-mono", "ui-monospace,Menlo,monospace"),
                ("--ds-text-base", "16px"),
                ("--ds-text-lg", "20px"),
                ("--ds-text-xl", "28px"),
                ("--ds-text-2xl", "38px"),
                ("--ds-text-3xl", "50px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "24px"),
                ("--ds-space-8", "44px"),
                ("--ds-radius-md", "16px"),
                ("--ds-radius-lg", "28px"),
                ("--ds-shadow-md", "0 6px 24px rgba(249,115,22,.12)"),
            ],
            doc: "温暖橙 + 薄荷绿点缀、大圆角、柔和暖阴影、圆润字体。语气友好鼓励。适合消费级 / 教育 / 健康。避免冷色、避免硬边直角。",
        },
        Builtin {
            id: "corporate",
            name: "专业金融",
            summary: "沉稳藏青、严谨栅格、克制配色的企业级语言",
            tokens: &[
                ("--ds-color-bg", "#ffffff"),
                ("--ds-color-fg", "#1e293b"),
                ("--ds-color-primary", "#1e3a8a"),
                ("--ds-color-secondary", "#475569"),
                ("--ds-color-accent", "#0f766e"),
                ("--ds-color-muted", "#f8fafc"),
                ("--ds-color-border", "#e2e8f0"),
                ("--ds-color-success", "#15803d"),
                ("--ds-color-warning", "#b45309"),
                ("--ds-color-danger", "#b91c1c"),
                ("--ds-font-sans", "'IBM Plex Sans','PingFang SC',system-ui,sans-serif"),
                ("--ds-font-serif", "'IBM Plex Serif',Georgia,serif"),
                ("--ds-font-mono", "'IBM Plex Mono',ui-monospace,monospace"),
                ("--ds-text-base", "15px"),
                ("--ds-text-lg", "18px"),
                ("--ds-text-xl", "24px"),
                ("--ds-text-2xl", "34px"),
                ("--ds-text-3xl", "46px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "24px"),
                ("--ds-space-8", "40px"),
                ("--ds-radius-md", "6px"),
                ("--ds-radius-lg", "10px"),
                ("--ds-shadow-md", "0 2px 8px rgba(30,41,59,.06)"),
            ],
            doc: "沉稳藏青、严谨栅格、信息密度高但层次清晰、克制的强调色。适合金融 / 企业 / 政务。避免鲜艳色、避免俏皮元素。",
        },
        Builtin {
            id: "bold-vibrant",
            name: "大胆活力",
            summary: "高饱和撞色、超大字重、几何块面的活力语言",
            tokens: &[
                ("--ds-color-bg", "#faf5ff"),
                ("--ds-color-fg", "#1e1b2e"),
                ("--ds-color-primary", "#7c3aed"),
                ("--ds-color-secondary", "#db2777"),
                ("--ds-color-accent", "#f59e0b"),
                ("--ds-color-muted", "#f3e8ff"),
                ("--ds-color-border", "#e9d5ff"),
                ("--ds-color-success", "#059669"),
                ("--ds-color-warning", "#ea580c"),
                ("--ds-color-danger", "#e11d48"),
                ("--ds-font-sans", "'Poppins','PingFang SC',system-ui,sans-serif"),
                ("--ds-font-serif", "Georgia,serif"),
                ("--ds-font-mono", "ui-monospace,Menlo,monospace"),
                ("--ds-text-base", "16px"),
                ("--ds-text-lg", "21px"),
                ("--ds-text-xl", "32px"),
                ("--ds-text-2xl", "46px"),
                ("--ds-text-3xl", "68px"),
                ("--ds-space-2", "8px"),
                ("--ds-space-4", "16px"),
                ("--ds-space-6", "26px"),
                ("--ds-space-8", "48px"),
                ("--ds-radius-md", "14px"),
                ("--ds-radius-lg", "24px"),
                ("--ds-shadow-md", "0 10px 40px rgba(124,58,237,.18)"),
            ],
            doc: "紫粉橙撞色、超大字重标题、几何块面、大圆角。适合活动 / 创意 / 年轻品牌。大胆但保持可读，撞色需控制在 2–3 种。",
        },
    ]
}

/// 一份"待渲染"的设计系统：原创内置（`brand_ref=None`）或品牌风格参考
/// （`brand_ref=Some(..)`，渲染时附免责声明）统一走此结构。
struct SystemEntry {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    /// 分组类目（品牌品类 / 原创原型），供 GUI 选择器分组。
    category: &'static str,
    /// 品牌风格参考的官方名（用于免责声明）；原创系统为 `None`。
    brand_ref: Option<&'static str>,
    tokens: BTreeMap<String, String>,
    doc: String,
}

/// 品牌风格参考的"种子"：只声明签名色 / 字体 / 圆角 / 字号密度 / 气质，运行期由
/// [`expand`] 展开为完整 25 token 契约（数据见 `brands.rs`）。
pub(super) struct BrandSeed {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) brand_ref: &'static str,
    pub(super) summary: &'static str,
    /// 分组类目（由 `brands.rs` 的 `cat(..)` 按分节统一赋值）。
    pub(super) category: &'static str,
    pub(super) bg: &'static str,
    pub(super) fg: &'static str,
    pub(super) primary: &'static str,
    /// 次强调色；空串 = 复用 primary。
    pub(super) accent: &'static str,
    pub(super) muted: &'static str,
    pub(super) border: &'static str,
    pub(super) font: &'static str,
    /// 标题 / 展示字体；空串 = 复用 font。
    pub(super) display_font: &'static str,
    pub(super) radius: Radius,
    pub(super) scale: Scale,
    pub(super) doc: &'static str,
}

/// 圆角风格 → `--ds-radius-{md,lg}`。
#[derive(Clone, Copy)]
pub(super) enum Radius {
    Sharp,
    Small,
    Medium,
    Rounded,
    Pill,
}

impl Radius {
    fn radii(self) -> (&'static str, &'static str) {
        match self {
            Radius::Sharp => ("2px", "4px"),
            Radius::Small => ("6px", "10px"),
            Radius::Medium => ("10px", "16px"),
            Radius::Rounded => ("16px", "26px"),
            Radius::Pill => ("22px", "999px"),
        }
    }
}

/// 字号密度 → `--ds-text-{base,lg,xl,2xl,3xl}`。
#[derive(Clone, Copy)]
pub(super) enum Scale {
    Compact,
    Normal,
    Display,
}

impl Scale {
    fn sizes(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match self {
            Scale::Compact => ("14px", "18px", "24px", "34px", "46px"),
            Scale::Normal => ("16px", "20px", "28px", "40px", "56px"),
            Scale::Display => ("16px", "22px", "34px", "52px", "72px"),
        }
    }
}

/// 由背景色感知明暗，决定语义色 / 中性色 / 阴影的深浅取向。
fn is_dark(hex: &str) -> bool {
    let bytes = hex.trim_start_matches('#').as_bytes();
    if bytes.len() < 6 {
        return false;
    }
    let val = |a: u8, b: u8| -> f32 {
        let hi = (a as char).to_digit(16).unwrap_or(0);
        let lo = (b as char).to_digit(16).unwrap_or(0);
        (hi * 16 + lo) as f32
    };
    let r = val(bytes[0], bytes[1]);
    let g = val(bytes[2], bytes[3]);
    let b = val(bytes[4], bytes[5]);
    0.2126 * r + 0.7152 * g + 0.0722 * b < 128.0
}

/// 把品牌种子展开成完整 25 token 契约的设计系统条目（语义色 / 中性色 / 阴影按明暗自适应）。
fn expand(s: &BrandSeed) -> SystemEntry {
    let dark = is_dark(s.bg);
    let (radius_md, radius_lg) = s.radius.radii();
    let (base, lg, xl, x2, x3) = s.scale.sizes();
    let accent = if s.accent.is_empty() {
        s.primary
    } else {
        s.accent
    };
    let serif = if s.display_font.is_empty() {
        s.font
    } else {
        s.display_font
    };
    let (success, warning, danger) = if dark {
        ("#34d399", "#fbbf24", "#f87171")
    } else {
        ("#16a34a", "#d97706", "#dc2626")
    };
    let secondary = if dark { "#94a3b8" } else { "#64748b" };
    let shadow = if dark {
        "0 1px 0 rgba(255,255,255,.04),0 12px 34px rgba(0,0,0,.5)"
    } else {
        "0 4px 20px rgba(15,23,42,.08)"
    };
    let mono = "ui-monospace,'SF Mono','JetBrains Mono',Menlo,Consolas,monospace";
    let pairs: [(&str, &str); 25] = [
        ("--ds-color-bg", s.bg),
        ("--ds-color-fg", s.fg),
        ("--ds-color-primary", s.primary),
        ("--ds-color-secondary", secondary),
        ("--ds-color-accent", accent),
        ("--ds-color-muted", s.muted),
        ("--ds-color-border", s.border),
        ("--ds-color-success", success),
        ("--ds-color-warning", warning),
        ("--ds-color-danger", danger),
        ("--ds-font-sans", s.font),
        ("--ds-font-serif", serif),
        ("--ds-font-mono", mono),
        ("--ds-text-base", base),
        ("--ds-text-lg", lg),
        ("--ds-text-xl", xl),
        ("--ds-text-2xl", x2),
        ("--ds-text-3xl", x3),
        ("--ds-space-2", "8px"),
        ("--ds-space-4", "16px"),
        ("--ds-space-6", "24px"),
        ("--ds-space-8", "44px"),
        ("--ds-radius-md", radius_md),
        ("--ds-radius-lg", radius_lg),
        ("--ds-shadow-md", shadow),
    ];
    SystemEntry {
        id: s.id,
        name: s.name,
        summary: s.summary,
        category: s.category,
        brand_ref: Some(s.brand_ref),
        tokens: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        doc: s.doc.to_string(),
    }
}

/// 全部内置设计系统：6 套原创原型语言 + `brands.rs` 的品牌风格参考。
fn all_systems() -> Vec<SystemEntry> {
    let mut v: Vec<SystemEntry> = builtins()
        .into_iter()
        .map(|b| SystemEntry {
            id: b.id,
            name: b.name,
            summary: b.summary,
            category: "原创原型",
            brand_ref: None,
            tokens: b
                .tokens
                .iter()
                .map(|(k, val)| (k.to_string(), val.to_string()))
                .collect(),
            doc: b.doc.to_string(),
        })
        .collect();
    v.extend(super::brands::seeds().iter().map(expand));
    v
}

/// 内置系统正文：按 **DESIGN.md 规范** 9 段 canonical schema 渲染 + 末尾 Token 表
/// （机器可回灌）。产出的即是一份完整、可移植、可无损导入的 DESIGN.md；品牌风格参考
/// 额外在摘要下附一行免责声明。
fn build_system_md(e: &SystemEntry) -> String {
    let sec = |i: usize| -> String {
        let (_, zh, en) = super::design_md::SECTIONS[i];
        format!("## {}. {zh} / {en}\n\n", i + 1)
    };
    let mut s = format!("# {} 设计系统\n\n> {}\n\n", e.name, e.summary);
    if let Some(brand) = e.brand_ref {
        s.push_str(&format!(
            "> 免责声明：本设计系统是对「{brand}」公开视觉语言的独立再诠释，仅供设计参考；与 {brand} 及其权利人不存在任何隶属、赞助或授权关系，相关名称与商标归各自所有者所有。\n\n"
        ));
    }
    s.push_str(&sec(0)); // brand
    s.push_str(&format!("{}\n\n", e.doc));
    s.push_str(&sec(1)); // palette
    s.push_str("主色 primary、辅助 secondary、强调 accent、中性 muted/border，语义色 success/warning/danger，全部以 `var(--ds-color-*)` 提供（见文末 Token 表）。\n\n");
    s.push_str(&sec(2)); // typography
    s.push_str("无衬线 sans 为主，衬线 serif 用于标题点缀，等宽 mono 用于代码/数据；字号阶 `--ds-text-*`。\n\n");
    s.push_str(&sec(3)); // spacing
    s.push_str(
        "8px 基准间距阶 `--ds-space-*`，留白充足；圆角 `--ds-radius-*`、阴影 `--ds-shadow-*`。\n\n",
    );
    s.push_str(&sec(4)); // layout
    s.push_str("移动优先、内容居中、最大宽度受控；断点自适应。\n\n");
    s.push_str(&sec(5)); // components
    s.push_str("按钮/卡片/输入统一圆角与阴影；层次靠字号与间距而非堆叠边框。\n\n");
    s.push_str(&sec(6)); // motion
    s.push_str("过渡克制自然（120–240ms、ease-out），只用 transform/opacity（60fps）；避免大幅位移与炫技。\n\n");
    s.push_str(&sec(7)); // voice
    s.push_str(&format!("与气质一致：{}。\n\n", e.summary));
    s.push_str(&sec(8)); // anti-patterns
    s.push_str(&format!("{}\n\n", e.doc));
    s.push_str(super::design_md::tokens_table(&e.tokens).trim_start());
    s
}

/// 懒 seed 内置系统到 managed 目录 + 注册 DB（幂等）。
pub fn ensure_builtins(db: &DesignDb) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for e in all_systems() {
        let dir = paths::design_system_dir(e.id)?;
        let md_path = dir.join(super::design_md::DESIGN_MD_FILE);
        let tokens_path = dir.join("tokens.json");
        // 已存在（用户可能已 fork/编辑）则不覆盖正文，仅确保 DB 注册。
        if !md_path.exists() {
            std::fs::create_dir_all(&dir)?;
            write_atomic(&md_path, build_system_md(&e).as_bytes())?;
        }
        if !tokens_path.exists() {
            let json = serde_json::to_string_pretty(&e.tokens)?;
            write_atomic(&tokens_path, json.as_bytes())?;
        }
        match db.get_system(e.id)? {
            None => db.upsert_system(&DesignSystemMeta {
                id: e.id.to_string(),
                name: e.name.to_string(),
                slug: e.id.to_string(),
                source: "builtin".to_string(),
                category: Some(e.category.to_string()),
                summary: Some(e.summary.to_string()),
                thumbnail_path: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })?,
            // 已存在（可能是升级前入库、category 为 NULL）：仅补齐类目，不动用户其它编辑。
            Some(_) => db.backfill_system_category(e.id, e.category)?,
        }
    }
    Ok(())
}

/// 读取设计系统正文 + token。
pub fn read_full(db: &DesignDb, id: &str) -> Result<DesignSystemFull> {
    let meta = db
        .get_system(id)?
        .with_context(|| format!("design system not found: {id}"))?;
    let dir = paths::design_system_dir(id)?;
    let system_md =
        std::fs::read_to_string(dir.join(super::design_md::DESIGN_MD_FILE)).unwrap_or_default();
    let tokens = std::fs::read_to_string(dir.join("tokens.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
        .unwrap_or_default();
    Ok(DesignSystemFull {
        meta,
        system_md,
        tokens,
        assets: read_assets(id),
    })
}

/// 新建 / 更新用户设计系统（正文 + token 一起写）。
#[allow(clippy::too_many_arguments)]
pub fn save_system(
    db: &DesignDb,
    id: &str,
    name: &str,
    summary: Option<&str>,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
    source: &str,
) -> Result<DesignSystemMeta> {
    let dir = paths::design_system_dir(id)?;
    std::fs::create_dir_all(&dir)?;
    // 落盘前用当前 tokens 重建 DESIGN.md 的 Token 表，保证 DESIGN.md ↔ tokens.json 一致
    // （编辑 tokens 不改正文时旧表会漂移；覆盖 editor / import / extract 所有路径）。
    let normalized_md = super::design_md::replace_tokens_table(system_md, tokens);
    write_atomic(
        &dir.join(super::design_md::DESIGN_MD_FILE),
        normalized_md.as_bytes(),
    )?;
    write_atomic(
        &dir.join("tokens.json"),
        serde_json::to_string_pretty(tokens)?.as_bytes(),
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    let existing = db.get_system(id)?;
    let created_at = existing
        .as_ref()
        .map(|m| m.created_at.clone())
        .unwrap_or_else(|| now.clone());
    // 保留既有分组（原地编辑内置品牌系统不丢类目）；纯用户新建系统为 None（归「我的」）。
    let category = existing.and_then(|m| m.category);
    let meta = DesignSystemMeta {
        id: id.to_string(),
        name: name.to_string(),
        slug: id.to_string(),
        source: source.to_string(),
        category,
        summary: summary.map(str::to_string),
        thumbnail_path: None,
        created_at,
        updated_at: now,
    };
    db.upsert_system(&meta)?;
    Ok(meta)
}

/// 删除设计系统（DB + 磁盘目录）。内置系统删除后 `ensure_builtins` 会重建。
pub fn delete_system(db: &DesignDb, id: &str) -> Result<()> {
    db.delete_system(id)?;
    if let Ok(dir) = paths::design_system_dir(id) {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_systems_ids_unique_and_slug_safe() {
        let sys = all_systems();
        // 6 原创 + 一批品牌风格参考。
        assert!(
            sys.len() >= 6 + 100,
            "expected 6 originals + brand refs, got {}",
            sys.len()
        );
        let mut seen = HashSet::new();
        for e in &sys {
            assert!(seen.insert(e.id), "duplicate system id: {}", e.id);
            assert!(
                e.id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "id not slug-safe: {}",
                e.id
            );
        }
    }

    #[test]
    fn brand_seeds_expand_to_full_contract() {
        for seed in crate::design::brands::seeds() {
            let e = expand(&seed);
            assert_eq!(e.tokens.len(), 25, "brand {} token count", e.id);
            assert!(e.brand_ref.is_some(), "brand {} missing ref", e.id);
            assert!(
                !e.category.is_empty(),
                "brand {} missing category (cat() wrapping?)",
                e.id
            );
            for k in [
                "--ds-color-primary",
                "--ds-color-bg",
                "--ds-font-sans",
                "--ds-radius-md",
                "--ds-shadow-md",
            ] {
                assert!(e.tokens.contains_key(k), "brand {} missing {k}", e.id);
            }
        }
    }

    #[test]
    fn brand_md_carries_disclaimer_and_round_trips_tokens() {
        let seed = &crate::design::brands::seeds()[0];
        let e = expand(seed);
        let md = build_system_md(&e);
        assert!(md.contains("免责声明"), "brand md must carry disclaimer");
        assert!(
            md.contains(e.brand_ref.unwrap()),
            "disclaimer must name brand"
        );
        // 末尾 Token 表可被无损回灌（导入回读一致）。
        let re = super::super::design_md::extract_tokens(&md);
        assert_eq!(re.len(), 25, "round-trip token count for {}", e.id);
    }

    #[test]
    fn original_builtins_have_no_disclaimer() {
        let sys = all_systems();
        let minimal = sys
            .iter()
            .find(|e| e.id == "minimal-modern")
            .expect("minimal-modern builtin present");
        assert!(minimal.brand_ref.is_none());
        let md = build_system_md(minimal);
        assert!(
            !md.contains("免责声明"),
            "original systems carry no disclaimer"
        );
    }

    #[test]
    fn dark_brand_uses_light_semantic_palette() {
        // 深色背景品牌应拿到更亮的语义色，浅色背景拿深色语义色。
        let dark = expand(&super::BrandSeed {
            id: "t-dark",
            name: "T",
            brand_ref: "T",
            summary: "s",
            category: "test",
            bg: "#0b0f17",
            fg: "#fff",
            primary: "#38bdf8",
            accent: "",
            muted: "#161b26",
            border: "#232a37",
            font: "sans-serif",
            display_font: "",
            radius: Radius::Medium,
            scale: Scale::Normal,
            doc: "d",
        });
        assert_eq!(
            dark.tokens.get("--ds-color-success").map(String::as_str),
            Some("#34d399")
        );
    }
}
