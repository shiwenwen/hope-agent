//! 「一句话 brief → 任意形态自包含 HTML 设计产物」的一次性生成（GUI/owner prompt→生成）。
//!
//! image 形态走 [`super::image`]（image_generate）；web / deck / dashboard / 文档 / 邮件 /
//! 海报 / 移动 / 动效 等结构化形态在此用一次分析 side-query 生成 body_html / css / js。
//! 让 GUI 的「打字 → 直接生成这个设计」对齐参照品类——此前非 image 形态 GUI 只能建空壳，
//! 真正的生成只发生在 agent 对话里。见 design-space.md §11。
//!
//! 输出用 `<<<BODY>>> / <<<CSS>>> / <<<JS>>>` 分节定界符（比 JSON 抗大段 HTML 的引号 / 换行
//! 转义更稳，模型更不易产出非法 JSON）。

use anyhow::Result;
use std::collections::BTreeMap;

use super::renderer::{ArtifactKind, ArtifactParts};

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        match s.char_indices().nth(max) {
            Some((i, _)) => &s[..i],
            None => s,
        }
    }
}

/// 该 kind 的首个内置 recipe 生成指导（无则空）。
fn kind_guidance(kind: ArtifactKind) -> String {
    let ks = kind.as_str();
    super::recipe::builtin_recipes()
        .into_iter()
        .find(|r| r.kind == ks)
        .map(|r| r.guidance)
        .unwrap_or_default()
}

/// 剥离 markdown 代码围栏：按行删掉首行 ```` ```lang ```` / 末行 ```` ``` ````。
/// 必须按行处理——`trim_matches('`')` 只去反引号、会把语言标签（```html 的 `html`）留在
/// 内容里污染该段（body 顶端多出 `html`、CSS 首规则失效、JS 裸标识符抛错）。
fn strip_fence(s: &str) -> String {
    let t = s.trim();
    let mut lines: Vec<&str> = t.lines().collect();
    if lines
        .first()
        .is_some_and(|f| f.trim_start().starts_with("```"))
    {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

/// 取 `start` 与下一个 `ends` 标记之间的内容（剥两端空白 + 代码围栏）。
fn between(text: &str, start: &str, ends: &[&str]) -> String {
    let Some(s) = text.find(start) else {
        return String::new();
    };
    let rest = &text[s + start.len()..];
    let end = ends
        .iter()
        .filter_map(|e| rest.find(e))
        .min()
        .unwrap_or(rest.len());
    strip_fence(&rest[..end])
}

fn parse_sections(text: &str) -> ArtifactParts {
    ArtifactParts {
        body_html: between(text, "<<<BODY>>>", &["<<<CSS>>>", "<<<JS>>>"]),
        css: between(text, "<<<CSS>>>", &["<<<BODY>>>", "<<<JS>>>"]),
        js: between(text, "<<<JS>>>", &["<<<BODY>>>", "<<<CSS>>>"]),
    }
}

/// 从 brief + kind + 设计系统生成自包含 HTML 产物（body_html / css / js）。
pub async fn generate_design_parts(
    brief: &str,
    kind: ArtifactKind,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<ArtifactParts> {
    if brief.trim().is_empty() {
        anyhow::bail!("design brief is empty");
    }
    let token_list = tokens.keys().cloned().collect::<Vec<_>>().join(", ");
    let system_block = if system_md.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nDESIGN SYSTEM — ground every color / type / spacing choice in it:\n{}\n",
            truncate(system_md, 8000)
        )
    };
    let prompt = format!(
        "You are a senior product designer. Produce a polished, production-grade **{kind}** design \
for the brief. Aim for something a designer would actually ship: strong visual hierarchy, real \
concrete content (never lorem ipsum), tasteful spacing, accessible contrast, thoughtful details.\n\n\
{common}\n\nKIND-SPECIFIC GUIDANCE:\n{guidance}\n\n\
Reference these design tokens as var(--x): {tokens}{system}\n\n\
Output EXACTLY three sections in this order and NOTHING else (no prose, no markdown code fences):\n\
<<<BODY>>>\n(the inner HTML that goes inside <body>)\n<<<CSS>>>\n(all CSS)\n<<<JS>>>\n(optional JS; may be empty)\n\n\
Hard rules: self-contained, ZERO network (no CDN, no remote fonts, no remote images — use inline \
SVG or CSS gradients for any imagery); responsive; accessible.\n\nBRIEF:\n{brief}",
        kind = kind.as_str(),
        common = super::recipe::COMMON_GUIDANCE,
        guidance = kind_guidance(kind),
        tokens = token_list,
        system = system_block,
        brief = truncate(brief, 4000),
    );
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    // 16000：一个完整网页 / 多页 deck / dashboard 的 HTML+CSS 很占 token，预算不足会截断。
    let res = agent.side_query(&prompt, 16000).await?;
    // 截断检测：合规输出必含 `<<<CSS>>>` 标记——缺失 = 在 body 段就被截断（否则 between 会把
    // 半截残余整段当 body、css/js 空，落库一个「成功」的无样式半截产物）。缺则 bail，让
    // 上层 `create_artifact_generating` 走降级空壳分支 + warn，不静默交付损坏产物。
    if !res.text.contains("<<<CSS>>>") {
        anyhow::bail!(
            "generation looks truncated for a {} brief (no CSS section)",
            kind.as_str()
        );
    }
    let parts = parse_sections(&res.text);
    if parts.body_html.trim().is_empty() {
        anyhow::bail!(
            "model returned no design body for a {} brief",
            kind.as_str()
        );
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delimited_sections() {
        let text = "junk\n<<<BODY>>>\n<main>hi</main>\n<<<CSS>>>\nmain{color:red}\n<<<JS>>>\nconsole.log(1)\n";
        let p = parse_sections(text);
        assert_eq!(p.body_html, "<main>hi</main>");
        assert_eq!(p.css, "main{color:red}");
        assert_eq!(p.js, "console.log(1)");
    }

    #[test]
    fn tolerates_missing_js() {
        let text = "<<<BODY>>>\n<div>x</div>\n<<<CSS>>>\ndiv{}";
        let p = parse_sections(text);
        assert_eq!(p.body_html, "<div>x</div>");
        assert_eq!(p.css, "div{}");
        assert_eq!(p.js, "");
    }

    #[test]
    fn strips_labeled_code_fences() {
        // 语言标签行（```html / ```css / ```js）必须整行删除，不能作为字面量残留污染内容。
        let text = "<<<BODY>>>\n```html\n<p>a</p>\n```\n<<<CSS>>>\n```css\np{}\n```\n<<<JS>>>\n```js\nconsole.log(1)\n```";
        let p = parse_sections(text);
        assert_eq!(p.body_html, "<p>a</p>");
        assert_eq!(p.css, "p{}");
        assert_eq!(p.js, "console.log(1)");
    }

    #[test]
    fn strips_bare_code_fences() {
        let text = "<<<BODY>>>\n```\n<p>a</p>\n```\n<<<CSS>>>\np{}";
        let p = parse_sections(text);
        assert_eq!(p.body_html, "<p>a</p>");
        assert_eq!(p.css, "p{}");
    }
}
