//! 设计系统层（品牌契约 + Token 编译）。
//!
//! 一个设计系统 = `SYSTEM.md`（9 段 prose，真相源，供 LLM grounding）+ `tokens.json`
//! （CSS 变量，渲染器注入产物 `:root`）。见 docs/architecture/design-space.md §6。
//!
//! 内置系统在此**代码内定义**（原创原型化设计语言，非品牌克隆），首次访问懒 seed
//! 到 managed 目录 + 注册 `design.db`，用户可 fork / 编辑。

use anyhow::{Context, Result};
use serde::Serialize;
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
    /// SYSTEM.md 正文（供 LLM 读取 grounding）。
    pub system_md: String,
    /// CSS 变量 token（有序）。
    pub tokens: BTreeMap<String, String>,
}

/// 内置系统定义（代码内）。
struct Builtin {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    tokens: &'static [(&'static str, &'static str)],
    /// SYSTEM.md 正文（气质 + 用法要点，原创措辞）。
    doc: &'static str,
}

/// Token 词汇表（渲染器与产物共用的 CSS 变量名约定）。
/// 每个内置系统按此表提供值；产物 CSS 一律引用 `var(--ds-*)`。
const TOKEN_KEYS_DOC: &str = "\
色彩：--ds-color-bg / --ds-color-fg / --ds-color-primary / --ds-color-secondary / --ds-color-accent / \
--ds-color-muted / --ds-color-border / --ds-color-success / --ds-color-warning / --ds-color-danger。\
排印：--ds-font-sans / --ds-font-serif / --ds-font-mono / --ds-text-sm / --ds-text-base / --ds-text-lg / \
--ds-text-xl / --ds-text-2xl / --ds-text-3xl。\
间距与圆角：--ds-space-1..8 / --ds-radius-sm / --ds-radius-md / --ds-radius-lg / --ds-shadow-sm / --ds-shadow-md。";

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

fn build_system_md(b: &Builtin) -> String {
    format!(
        "# {name} 设计系统\n\n\
> {summary}\n\n\
## 1. 主题与气质\n\n{doc}\n\n\
## 2. 色彩与角色\n\n\
主色 primary、辅助 secondary、强调 accent、中性 muted/border，以及语义色 success/warning/danger。\
所有颜色以 CSS 变量提供（见文末 Token 清单），产物 CSS 引用 `var(--ds-color-*)`。\n\n\
## 3. 字体排印\n\n无衬线 sans 为主，衬线 serif 用于标题点缀，等宽 mono 用于代码/数据。字号阶见 Token。\n\n\
## 4. 间距与网格\n\n8px 基准间距阶（`--ds-space-*`），栅格与容器留白充足。\n\n\
## 5. 布局与响应式\n\n移动优先、内容居中、最大宽度受控；断点自适应。\n\n\
## 6. 组件样式\n\n按钮/卡片/输入统一圆角 `--ds-radius-*`、统一阴影 `--ds-shadow-*`。\n\n\
## 7. 层次与投影\n\n层次靠字号、间距、克制的阴影表达，避免堆叠边框。\n\n\
## 8. 语气与文案\n\n与气质一致：{summary}。\n\n\
## 9. 禁忌\n\n{doc}\n\n\
---\n\n**Token 词汇表**：{keys}\n",
        name = b.name,
        summary = b.summary,
        doc = b.doc,
        keys = TOKEN_KEYS_DOC,
    )
}

fn tokens_map(b: &Builtin) -> BTreeMap<String, String> {
    b.tokens
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// 懒 seed 内置系统到 managed 目录 + 注册 DB（幂等）。
pub fn ensure_builtins(db: &DesignDb) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for b in builtins() {
        let dir = paths::design_system_dir(b.id)?;
        let md_path = dir.join("SYSTEM.md");
        let tokens_path = dir.join("tokens.json");
        // 已存在（用户可能已 fork/编辑）则不覆盖正文，仅确保 DB 注册。
        if !md_path.exists() {
            std::fs::create_dir_all(&dir)?;
            write_atomic(&md_path, build_system_md(&b).as_bytes())?;
        }
        if !tokens_path.exists() {
            let json = serde_json::to_string_pretty(&tokens_map(&b))?;
            write_atomic(&tokens_path, json.as_bytes())?;
        }
        if db.get_system(b.id)?.is_none() {
            db.upsert_system(&DesignSystemMeta {
                id: b.id.to_string(),
                name: b.name.to_string(),
                slug: b.id.to_string(),
                source: "builtin".to_string(),
                summary: Some(b.summary.to_string()),
                thumbnail_path: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })?;
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
    let system_md = std::fs::read_to_string(dir.join("SYSTEM.md")).unwrap_or_default();
    let tokens = std::fs::read_to_string(dir.join("tokens.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
        .unwrap_or_default();
    Ok(DesignSystemFull {
        meta,
        system_md,
        tokens,
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
    write_atomic(&dir.join("SYSTEM.md"), system_md.as_bytes())?;
    write_atomic(
        &dir.join("tokens.json"),
        serde_json::to_string_pretty(tokens)?.as_bytes(),
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    let created_at = db
        .get_system(id)?
        .map(|m| m.created_at)
        .unwrap_or_else(|| now.clone());
    let meta = DesignSystemMeta {
        id: id.to_string(),
        name: name.to_string(),
        slug: id.to_string(),
        source: source.to_string(),
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
