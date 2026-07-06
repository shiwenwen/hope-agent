//! 「一句话 brief → 任意形态自包含 HTML 设计产物」的一次性生成（GUI/owner prompt→生成）。
//!
//! image 形态走 [`super::image`]（image_generate）；web / deck / dashboard / 文档 / 邮件 /
//! 海报 / 移动 / 动效 等结构化形态在此用一次分析 side-query 生成 body_html / css / js。
//! 让 GUI 的「打字 → 直接生成这个设计」对齐参照品类——此前非 image 形态 GUI 只能建空壳，
//! 真正的生成只发生在 agent 对话里。见 design-space.md §11。
//!
//! 输出用 `<<<CSS>>> / <<<BODY>>> / <<<JS>>>` 分节定界符（比 JSON 抗大段 HTML 的引号 / 换行
//! 转义更稳，模型更不易产出非法 JSON）。**CSS 段在前**：流式预览可先把最终样式注入 iframe
//! head，再流式追加 body，杜绝「先闪一屏无样式内容」的 FOUC。
//!
//! 两个入口共用同一 prompt（`build_generation_prompt`）：
//! - [`generate_design_parts`]：一次性阻塞生成（agent 工具面 / 兜底）。
//! - [`stream_design_parts`]：真流式，走 `side_query_streaming`，逐段把「到目前为止的完整
//!   CSS + 正在增长的 body」回调出去做 live 预览（design 空间 owner/GUI 生成）。

use anyhow::Result;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

/// CSS-first 生成 prompt（两入口共用）。CSS 段在 body 前，让流式预览先有样式。
fn build_generation_prompt(
    brief: &str,
    kind: ArtifactKind,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<String> {
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
    Ok(format!(
        "You are a senior product designer. Produce a polished, production-grade **{kind}** design \
for the brief. Aim for something a designer would actually ship: strong visual hierarchy, real \
concrete content (never lorem ipsum), tasteful spacing, accessible contrast, thoughtful details.\n\n\
{common}\n\nKIND-SPECIFIC GUIDANCE:\n{guidance}\n\n\
Reference these design tokens as var(--x): {tokens}{system}\n\n\
Output EXACTLY three sections in this order and NOTHING else (no prose, no markdown code fences). \
Emit CSS FIRST so a live preview can apply styles before the body paints:\n\
<<<CSS>>>\n(all CSS)\n<<<BODY>>>\n(the inner HTML that goes inside <body>)\n<<<JS>>>\n(optional JS; may be empty)\n\n\
Hard rules: self-contained, ZERO network (no CDN, no remote fonts, no remote images — use inline \
SVG or CSS gradients for any imagery); responsive; accessible.\n\nBRIEF:\n{brief}",
        kind = kind.as_str(),
        common = super::recipe::COMMON_GUIDANCE,
        guidance = kind_guidance(kind),
        tokens = token_list,
        system = system_block,
        brief = truncate(brief, 4000),
    ))
}

/// 截断检测：CSS-first 下合规输出必含 `<<<BODY>>>`（CSS 段在前，`<<<BODY>>>` 出现即证明 CSS
/// 段完整收束）。缺失 = 在 CSS 段就被截断——`between` 会把半截 CSS 当 body 之外的残余、body/js
/// 空，落库一个「成功」的损坏半截产物。缺则 bail，让上层走降级空壳 + warn。
fn validate_not_truncated(text: &str, kind: ArtifactKind) -> Result<ArtifactParts> {
    if !text.contains("<<<BODY>>>") {
        anyhow::bail!(
            "generation looks truncated for a {} brief (no BODY section)",
            kind.as_str()
        );
    }
    let parts = parse_sections(text);
    if parts.body_html.trim().is_empty() {
        anyhow::bail!(
            "model returned no design body for a {} brief",
            kind.as_str()
        );
    }
    Ok(parts)
}

/// 三个分节定界符（截断检测 / 增量剥离共用真相源）。
const SECTION_MARKERS: [&str; 3] = ["<<<CSS>>>", "<<<BODY>>>", "<<<JS>>>"];

/// 剥离缓冲区尾部**未闭合的真 marker 前缀**（如 `<<<`, `<<<BOD`, `<<<JS>`）——流式增量解析时
/// 防半截 marker 泄漏进 body/css 预览。**只在 buf 的结尾后缀恰是某个完整 marker 的严格前缀时
/// 才截**：正文里合法出现的 `<<<`（git 冲突标记 `<<<<<<< HEAD` / `content:"<<<"` / ASCII art）
/// 其后跟的字符不构成 marker 前缀，原样保留、绝不冻结预览（旧版裸 `rfind("<<<")` 会把这类
/// 合法 `<<<` 当未闭合 marker 反复截到同一 pos、把节流基线钉死 = 预览多秒冻结）。
fn strip_trailing_partial_marker(buf: &str) -> &str {
    let max = SECTION_MARKERS.iter().map(|m| m.len()).max().unwrap_or(0);
    let lo = buf.len().saturating_sub(max);
    // 从最长后缀往短找，取最长的「是某完整 marker 严格前缀」的尾部截掉。
    for cut in lo..buf.len() {
        if !buf.is_char_boundary(cut) {
            continue;
        }
        let tail = &buf[cut..];
        if SECTION_MARKERS
            .iter()
            .any(|m| m.len() > tail.len() && m.starts_with(tail))
        {
            return &buf[..cut];
        }
    }
    buf
}

/// 从 brief + kind + 设计系统生成自包含 HTML 产物（body_html / css / js）。
pub async fn generate_design_parts(
    brief: &str,
    kind: ArtifactKind,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<ArtifactParts> {
    let prompt = build_generation_prompt(brief, kind, system_md, tokens)?;
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    // 16000：一个完整网页 / 多页 deck / dashboard 的 HTML+CSS 很占 token，预算不足会截断。
    let res = agent.side_query(&prompt, 16000).await?;
    validate_not_truncated(&res.text, kind)
}

/// 真流式生成：走 `side_query_streaming`，把「到目前为止的完整 CSS + 正在增长的 body」经
/// `on_snapshot` 逐段回调（按字节增长节流），供上层 live 预览。返回定稿完整 parts（权威真相，
/// 落盘用）。失败（截断 / 空 body / 无后端）返回 `Err`，由上层降级空壳。
pub async fn stream_design_parts(
    brief: &str,
    kind: ArtifactKind,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
    cancel: &Arc<AtomicBool>,
    on_snapshot: &(dyn Fn(&ArtifactParts) + Send + Sync),
) -> Result<ArtifactParts> {
    let prompt = build_generation_prompt(brief, kind, system_md, tokens)?;
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;

    // 按字节增长节流（≥ STEP 才发一帧）：帧小、纯文本、频率有界，稳过 WS broadcast，避免
    // per-token 洪泛。首帧在 CSS 段完整（`<<<BODY>>>` 一现）即触发，让样式尽早落地。
    const STEP: usize = 1200;
    let last_len = std::sync::Mutex::new(0usize);
    let on_text = |cumulative: &str| {
        let cleaned = strip_trailing_partial_marker(cumulative);
        {
            let mut g = last_len.lock().unwrap_or_else(|e| e.into_inner());
            // failover 重试：累积文本从头重启（变短）→ 复位高水位，让新尝试的首帧重新触发
            // （否则 STEP 节流会把新尝试的完整快照压制到超过旧尝试峰值才发帧、甚至永不发）。
            // 当前接线（agent 无 session_id → 恒单尝试直连）不可达，作前瞻防御。
            if cleaned.len() < *g {
                *g = 0;
            }
            let grew_enough = cleaned.len() >= *g + STEP;
            let css_just_completed = *g == 0 && cleaned.contains("<<<BODY>>>");
            if !grew_enough && !css_just_completed {
                return;
            }
            *g = cleaned.len();
        }
        let parts = parse_sections(cleaned);
        on_snapshot(&parts);
    };

    let res = agent
        .side_query_streaming(&prompt, 16000, cancel, &on_text)
        .await?;
    validate_not_truncated(&res.text, kind)
}

/// 生成交互式 `Component` 产物的 React 组件源（JSX/TSX，classic runtime、全局 React、无 import/
/// export）。返回原始源码字符串，由 `service::render` 走后端 oxc 编译。失败 `Err` → 上层降级。
pub async fn generate_component_source(
    brief: &str,
    system_md: &str,
    tokens: &BTreeMap<String, String>,
) -> Result<String> {
    if brief.trim().is_empty() {
        anyhow::bail!("component brief is empty");
    }
    let token_list = tokens.keys().cloned().collect::<Vec<_>>().join(", ");
    let system_block = if system_md.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nDESIGN SYSTEM — ground colors / type / spacing in it (reference tokens as CSS \
var(--x) in inline styles):\n{}\n",
            truncate(system_md, 6000)
        )
    };
    let prompt = format!(
        "You are a senior frontend engineer. Write a **single self-contained React component** for the brief.\n\n\
CRITICAL RULES:\n\
- Define a component named EXACTLY `App`: `function App() {{ ... }}`.\n\
- Use the GLOBAL `React` (already loaded on the page): `React.useState`, `React.useEffect`, \
`React.useRef`, `React.useMemo`, etc.\n\
- **Do NOT write any import or export statements** — no `import React from 'react'`, no \
`export default`. The runtime provides `React` and `ReactDOM` as globals.\n\
- Return JSX. Inline styles are objects: `style={{{{ color: 'red', padding: 16 }}}}`.\n\
- Self-contained, ZERO network: no CDN, no remote fonts/images — use inline SVG or CSS gradients.\n\
- Make it genuinely interactive, polished, production-grade (state, events, transitions).\n\
- Reference these design tokens as CSS variables where you style: {tokens}.{system}\n\n\
Output ONLY the component source code (JSX/TSX). No markdown code fences, no prose, no explanation.\n\n\
BRIEF:\n{brief}",
        tokens = token_list,
        system = system_block,
        brief = truncate(brief, 4000),
    );
    let config = crate::config::cached_config();
    let (agent, _model) = crate::recap::report::build_analysis_agent(&config).await?;
    let res = agent.side_query(&prompt, 16000).await?;
    let src = strip_fence(&res.text);
    if src.trim().is_empty() {
        anyhow::bail!("model returned no component source");
    }
    // 早筛：必须含 `App`（否则 bootstrap 找不到组件、编译/运行必失败），早 bail 走降级。
    if !src.contains("App") {
        anyhow::bail!("generated component source has no `App` component");
    }
    Ok(src)
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

    // ── CSS-first truncation detection ───────────────────────────────
    #[test]
    fn validate_bails_when_body_section_missing() {
        // CSS-first: truncated mid-CSS → no <<<BODY>>> marker → must bail so the
        // caller degrades to a shell instead of shipping a broken half-artifact.
        let truncated = "<<<CSS>>>\nbody{color:red;font-";
        assert!(validate_not_truncated(truncated, ArtifactKind::Web).is_err());
    }

    #[test]
    fn validate_accepts_complete_css_first_output() {
        let ok = "<<<CSS>>>\nbody{color:red}\n<<<BODY>>>\n<main>Hi</main>\n<<<JS>>>\n";
        let parts = validate_not_truncated(ok, ArtifactKind::Web).expect("complete");
        assert_eq!(parts.css, "body{color:red}");
        assert_eq!(parts.body_html, "<main>Hi</main>");
    }

    #[test]
    fn validate_bails_on_empty_body() {
        let empty_body = "<<<CSS>>>\nbody{}\n<<<BODY>>>\n\n<<<JS>>>\n";
        assert!(validate_not_truncated(empty_body, ArtifactKind::Web).is_err());
    }

    // ── incremental streaming guards ─────────────────────────────────
    #[test]
    fn strip_trailing_partial_marker_cuts_incomplete() {
        // A marker being streamed (`<<<`, `<<<BOD`, `<<<JS>`) is cut off so it
        // never leaks into the previewed body.
        assert_eq!(
            strip_trailing_partial_marker("<<<CSS>>>\n.x{}\n<<<BODY>>>\n<div>a</div><<<"),
            "<<<CSS>>>\n.x{}\n<<<BODY>>>\n<div>a</div>"
        );
        assert_eq!(
            strip_trailing_partial_marker("<<<CSS>>>\n.x{}\n<<<BOD"),
            "<<<CSS>>>\n.x{}\n"
        );
        assert_eq!(
            strip_trailing_partial_marker("<<<CSS>>>\n.x{}\n<<<BODY>>>\n<p>x</p>\n<<<JS>"),
            "<<<CSS>>>\n.x{}\n<<<BODY>>>\n<p>x</p>\n"
        );
    }

    #[test]
    fn strip_trailing_partial_marker_keeps_complete() {
        // All markers closed → nothing to strip.
        let complete = "<<<CSS>>>\n.x{}\n<<<BODY>>>\n<p>x</p>\n<<<JS>>>\ncode()";
        assert_eq!(strip_trailing_partial_marker(complete), complete);
    }

    #[test]
    fn strip_trailing_partial_marker_keeps_literal_triple_angle() {
        // Legit `<<<` in content (git conflict marker, ASCII art) is NOT a marker
        // prefix once followed by non-marker chars → left intact, no freeze.
        let conflict = "<<<CSS>>>\n.x{}\n<<<BODY>>>\n<pre><<<<<<< HEAD\nmore";
        assert_eq!(strip_trailing_partial_marker(conflict), conflict);
        // `content:"<<<x"` — the `<<<x` tail isn't any marker's prefix.
        let css_literal = "<<<CSS>>>\n.a::before{content:\"<<<x\"";
        assert_eq!(strip_trailing_partial_marker(css_literal), css_literal);
    }

    #[test]
    fn incremental_snapshot_has_complete_css_and_growing_body() {
        // Mid-stream: CSS section closed, body still growing. The cleaned buffer
        // parses to a complete CSS + partial body — exactly what the preview
        // needs to style-then-fill.
        let mid =
            strip_trailing_partial_marker("<<<CSS>>>\nbody{color:blue}\n<<<BODY>>>\n<main><h1>Ti");
        let parts = parse_sections(mid);
        assert_eq!(parts.css, "body{color:blue}");
        assert_eq!(parts.body_html, "<main><h1>Ti");
        assert_eq!(parts.js, "");
    }
}
