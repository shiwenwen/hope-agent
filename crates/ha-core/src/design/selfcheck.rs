//! 反 AI-slop **确定性自查**（无 LLM）——设计空间产物版，移植并适配自 atelier
//! 的 `selfcheck.rs`（原为 JSX 组件卡）。两类信号：
//! - **thin**：剥掉 `<script>` / `<style>` / 注释后，元素开标签数与可见文字都低于下限
//!   （近空壳）。
//! - **placeholder**：命中高置信占位/填充标记（`lorem ipsum` / `your text here` /
//!   `#REPLACE_ME` 等）——真实可交付产物绝不含这些。
//! - **near_identical**（纯函数，供多方向候选去雷同复用）：去标签后可见文字的字符
//!   5-gram shingle 的 Jaccard 相似度阈值判定（CJK 无词边界，故用字符级）。
//!
//! 命中的产物由 [`crate::design::service`] 在创建 / 生成定稿时翻 `needs_review` 并把
//! `selfCheck` 键**合并**进 `metadata`；未命中或关闭 `design.self_check` 时清键回
//! `ready`。**与两个竞品的区别**：它们的质量闸都靠 LLM 自判（成本高、非确定）；本闸
//! LLM-free、确定、可单测，是差异化护城河。设计契约见 `design-space.md` §11.3。

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

/// thin 判定：剥离后元素开标签数下限（HTML 比 JSX 样板多，取保守小值只抓近空壳）。
const THIN_MIN_TAGS: usize = 5;
/// thin 判定：剥离标记后可见非空白文字字符数下限。
const THIN_MIN_TEXT_CHARS: usize = 60;
/// 雷同判定：可见文字最小长度（短产物噪声大，不比）。
const IDENTICAL_MIN_CHARS: usize = 120;
/// 雷同判定：字符 5-gram shingle 集合的 Jaccard 相似度阈值。
const IDENTICAL_JACCARD_THRESHOLD: f64 = 0.90;

/// 高置信占位 / 填充标记（小写匹配）。刻意从严——只列「真实交付物绝不会出现」的词，
/// 避免误伤合法内容（如正文里合理出现的 "todo" 不进此表）。
const PLACEHOLDER_MARKERS: &[&str] = &[
    "lorem ipsum",
    "your text here",
    "your headline here",
    "your title here",
    "your content here",
    "replace me",
    "replace_me",
    "#replace",
    "insert text here",
    "placeholder text",
    "sample text here",
    "[placeholder]",
    "add your text",
    "click to edit",
];

/// 单产物自查结论：`flag ∈ {"thin","placeholder"}` + 人读细节（写进 metadata）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfCheckFlag {
    pub flag: &'static str,
    pub detail: String,
}

/// 剥掉 `<script>…</script>`、`<style>…</style>` 与 `<!-- -->` 注释（大小写不敏感）。
/// 朴素扫描：不感知属性里的 `>`（HTML 标签属性可含 `>` 极罕见，启发式可接受）。
fn strip_noise(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0usize;
    while i < bytes.len() {
        // 尝试匹配需要整段吞掉的区块起点。
        if let Some(end) = skip_block(&lower, i, "<script", "</script>")
            .or_else(|| skip_block(&lower, i, "<style", "</style>"))
            .or_else(|| skip_block(&lower, i, "<!--", "-->"))
        {
            i = end;
            continue;
        }
        // 逐字节推进（按 UTF-8 边界）。
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&body[i..(i + ch_len).min(body.len())]);
        i += ch_len;
    }
    out
}

/// 若 `lower[at..]` 以 `open` 起（`open` 后须是 `>` / 空白 / `/` 或即块注释），返回吞到
/// `close` 之后的下标（未闭合则吞到末尾）；否则 None。
fn skip_block(lower: &str, at: usize, open: &str, close: &str) -> Option<usize> {
    let rest = lower.get(at..)?;
    if !rest.starts_with(open) {
        return None;
    }
    // `<script`/`<style` 后须跟非字母（`>`/空白/`/`），避免误吞 `<styles-foo>` 之类。
    if open.starts_with("<s") {
        let after = rest.as_bytes().get(open.len()).copied();
        if let Some(b) = after {
            if b.is_ascii_alphabetic() {
                return None;
            }
        }
    }
    let close_at = rest.find(close);
    Some(match close_at {
        Some(rel) => at + rel + close.len(),
        None => lower.len(), // 未闭合：吞到末尾
    })
}

fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// 元素开标签计数：`<` 后紧跟 ASCII 字母（`</div` 的 `<` 后是 `/`，不计入 = 只数开标签）。
fn count_element_tags(s: &str) -> usize {
    s.as_bytes()
        .windows(2)
        .filter(|w| w[0] == b'<' && w[1].is_ascii_alphabetic())
        .count()
}

/// 去掉所有 `<...>` 标签后的可见非空白字符数。
fn visible_text_chars(s: &str) -> usize {
    let mut count = 0usize;
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag && !c.is_whitespace() => count += 1,
            _ => {}
        }
    }
    count
}

/// 「内容近空」判定：剥掉 script/style/注释后，元素开标签 < 5 且可见文字 < 60。
pub fn is_thin(body: &str) -> bool {
    let stripped = strip_noise(body);
    count_element_tags(&stripped) < THIN_MIN_TAGS
        && visible_text_chars(&stripped) < THIN_MIN_TEXT_CHARS
}

/// 命中的高置信占位标记（供细节展示），无则 None。
pub fn find_placeholder(body: &str) -> Option<&'static str> {
    let lower = body.to_ascii_lowercase();
    PLACEHOLDER_MARKERS
        .iter()
        .find(|m| lower.contains(**m))
        .copied()
}

/// AI-slop 表层「tell 色」：Tailwind indigo / violet 500-700 的硬写 hex——被当 accent 直用（不走
/// 设计 token）是最经典的 AI 味信号。仅列这几个高置信值，谨慎防误标。
const SLOP_INDIGO_HEXES: &[&str] = &[
    "#6366f1", "#4f46e5", "#4338ca", "#818cf8", "#a78bfa", "#8b5cf6", "#7c3aed", "#6d28d9",
];

/// 字符是否落在常见 emoji 区（主 emoji + misc symbols + dingbats），用于「emoji 当图标」检测。
fn is_emoji_char(c: char) -> bool {
    matches!(c as u32, 0x1F300..=0x1FAFF | 0x2600..=0x27BF)
}

/// 确定性反 AI-slop 表层检测（无 LLM、可单测）：只抓最可靠的两类 tell，谨慎设阈值防误标。
/// 与既有 thin / placeholder **叠加**（诊断用，不阻断）。返回命中的信号 key。
pub fn find_slop_signals(body: &str, css: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    let hay = format!("{}\n{}", body.to_ascii_lowercase(), css.to_ascii_lowercase());
    // ① indigo/violet 硬写作 accent（AI 味头号 tell 色）。
    if SLOP_INDIGO_HEXES.iter().any(|h| hay.contains(h)) {
        out.push("indigo-accent");
    }
    // ② emoji 当图标 / section marker（≥3 个 emoji = 模式，而非偶发）。
    if body.chars().filter(|&c| is_emoji_char(c)).count() >= 3 {
        out.push("emoji-icons");
    }
    // ③ accent 失控：**CSS 里**硬写的不同 hex 色 > 阈值（无视设计 token 撒一堆颜色）。只数 CSS
    //    （不数 body，避开内联 SVG 插画的正常多色），阈值取高（>16）只抓失控、谨慎防误标。
    if count_distinct_css_hex(css) > 16 {
        out.push("color-sprawl");
    }
    out
}

/// CSS 里不同的 `#rgb` / `#rrggbb` 硬写色数量（小写归一、去重）。仅用于「accent 失控」检测。
fn count_distinct_css_hex(css: &str) -> usize {
    let b = css.as_bytes();
    let mut seen: HashSet<String> = HashSet::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'#' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j] as char).is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - start;
            if len == 3 || len == 6 {
                seen.insert(css[start..j].to_ascii_lowercase());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    seen.len()
}

fn slop_detail(signals: &[&str]) -> String {
    let parts: Vec<&str> = signals
        .iter()
        .map(|s| match *s {
            "indigo-accent" => "indigo/violet 硬写作 accent（未走设计 token）",
            "emoji-icons" => "emoji 当图标 / 段落标记",
            "color-sprawl" => "CSS 硬写过多不同色（未克制 accent / 无视 token）",
            other => other,
        })
        .collect();
    format!("疑似 AI-slop 表层信号：{}", parts.join("、"))
}

/// 单产物确定性自查：placeholder（更具体）> thin（近空壳）> slop（表层 AI 味）。
pub fn evaluate(body: &str, css: &str) -> Option<SelfCheckFlag> {
    if let Some(marker) = find_placeholder(body) {
        return Some(SelfCheckFlag {
            flag: "placeholder",
            detail: format!("含占位/填充文本「{marker}」"),
        });
    }
    if is_thin(body) {
        return Some(SelfCheckFlag {
            flag: "thin",
            detail: "内容近空（元素与文字过少）".to_string(),
        });
    }
    let slop = find_slop_signals(body, css);
    if !slop.is_empty() {
        return Some(SelfCheckFlag {
            flag: "slop",
            detail: slop_detail(&slop),
        });
    }
    None
}

/// 把自查结论**合并**进现有 metadata JSON：命中写 `selfCheck` 键，未命中清键（保留
/// 其它键）。返回序列化后的 metadata（清空后为空对象则回 None）。**只动 `selfCheck`
/// 键**——不覆盖用户 / agent 写的其它 metadata。
pub fn merge_into_metadata(
    existing: Option<&str>,
    verdict: Option<&SelfCheckFlag>,
) -> Option<String> {
    let mut obj = existing
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    match verdict {
        Some(f) => {
            obj.insert(
                "selfCheck".to_string(),
                serde_json::json!({ "flag": f.flag, "detail": f.detail }),
            );
        }
        None => {
            obj.remove("selfCheck");
        }
    }
    if obj.is_empty() {
        None
    } else {
        serde_json::to_string(&serde_json::Value::Object(obj)).ok()
    }
}

// ── near-identical（多方向候选去雷同，纯函数）────────────────────────────
//
// 用**字符级 n-gram** shingle（非 token 级）——设计空间产物多为中文/CJK，CJK 无词
// 边界，token 切分会把整段折成一个 token。比较对象是**去标签后的可见文字**（比内容不
// 比 HTML 骨架，避免同壳不同文案被误判雷同）。

/// n-gram 字符窗口宽度。
const SHINGLE_N: usize = 5;

/// 去掉所有 `<...>` 标签、把连续空白折叠为单空格后的可见文字。
fn visible_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' '); // 标签边界视作空白，防跨标签粘连成假 shingle
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct ShinglePrep {
    char_count: usize,
    shingles: HashSet<u64>,
}

fn prepare_shingles(source: &str) -> ShinglePrep {
    let text = visible_text(source);
    let chars: Vec<char> = text.chars().collect();
    let shingles = chars
        .windows(SHINGLE_N)
        .map(|w| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for c in w {
                c.hash(&mut h);
            }
            h.finish()
        })
        .collect();
    ShinglePrep {
        char_count: chars.len(),
        shingles,
    }
}

fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let inter = small.iter().filter(|s| large.contains(s)).count();
    let union = a.len() + b.len() - inter;
    inter as f64 / union as f64
}

/// 两产物「雷同」判定：双方可见文字都 ≥ 120 字符才比较；字符 5-gram shingle 的
/// Jaccard ≥ 0.90 视为雷同。供多方向候选生成去掉近重复候选（模型偷懒产雷同稿）。
pub fn near_identical(a: &str, b: &str) -> bool {
    let pa = prepare_shingles(a);
    let pb = prepare_shingles(b);
    pa.char_count >= IDENTICAL_MIN_CHARS
        && pb.char_count >= IDENTICAL_MIN_CHARS
        && jaccard(&pa.shingles, &pb.shingles) >= IDENTICAL_JACCARD_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_flags_near_empty_shell() {
        assert!(is_thin("<div></div>"));
        assert!(is_thin("<section><h1>Hi</h1></section>"));
    }

    #[test]
    fn slop_flags_indigo_accent_hex() {
        // CSS 里硬写 indigo tell 色 → slop 命中。
        let sig = find_slop_signals("<div>real content here</div>", ".btn{background:#6366F1}");
        assert!(sig.contains(&"indigo-accent"));
    }

    #[test]
    fn slop_flags_emoji_icons_over_threshold() {
        assert!(find_slop_signals("<h2>🚀 Fast</h2><h2>🎨 Design</h2><h2>📊 Data</h2>", "")
            .contains(&"emoji-icons"));
        // 偶发单个 emoji 不算。
        assert!(!find_slop_signals("<p>Nice 🚀</p>", "").contains(&"emoji-icons"));
    }

    #[test]
    fn slop_flags_color_sprawl_css_only() {
        // 17 个不同 hex 硬写在 CSS → color-sprawl（小值保证都是 6 位）。
        let css: String = (0..17u32).map(|n| format!(".c{n}{{color:#{:06x}}}", n * 0x100 + 0x10)).collect();
        assert!(find_slop_signals("<div>x</div>", &css).contains(&"color-sprawl"));
        // body 里的内联 SVG 多色不计入（只数 CSS）→ 不误标。
        let svg_body: String = format!(
            "<svg>{}</svg>",
            (0..30u32).map(|n| format!("<rect fill=\"#{:06x}\"/>", n * 0x100 + 0x20)).collect::<String>()
        );
        assert!(!find_slop_signals(&svg_body, ".a{color:#111}.b{color:#222}").contains(&"color-sprawl"));
    }

    #[test]
    fn slop_clean_design_no_false_positive() {
        // 走 token 的正常配色 + 无 emoji → 不误标。
        let sig = find_slop_signals(
            "<section class=\"hero\"><h1>产品标题</h1><p>真实文案</p></section>",
            ".hero{background:var(--ds-color-primary);color:var(--ds-color-fg)}",
        );
        assert!(sig.is_empty());
    }

    #[test]
    fn evaluate_slop_after_thin_and_placeholder() {
        // 内容充实但 indigo 硬写 → slop（非 thin / placeholder）。
        let body = "<section><h1>Real Product Headline</h1><p>Substantive marketing copy that is clearly not a placeholder and has enough text.</p></section>";
        let v = evaluate(body, ".x{color:#4f46e5}").unwrap();
        assert_eq!(v.flag, "slop");
    }

    #[test]
    fn thin_ignores_script_and_style_bulk() {
        // 大量 script/style 不算内容——剥离后仍近空 = thin。
        let body = "<div><style>.a{color:red}.b{margin:0}.c{padding:2rem}</style>\
                    <script>const x=1;function f(){return 42}</script></div>";
        assert!(is_thin(body), "script/style 体积不应抵消 thin 判定");
    }

    #[test]
    fn thin_passes_real_content() {
        let body = "<header><h1>秋季发布会</h1><p>加入我们，见证下一代产品的诞生，\
                    现场演示、限量周边、深度问答，尽在十月盛典。</p></header>\
                    <main><section><h2>议程</h2><ul><li>开场</li><li>主题演讲</li>\
                    <li>产品揭示</li></ul></section></main>";
        assert!(!is_thin(body));
        assert!(evaluate(body, "").is_none());
    }

    #[test]
    fn placeholder_flags_lorem_and_markers() {
        assert_eq!(
            find_placeholder("<p>Lorem ipsum dolor sit amet</p>"),
            Some("lorem ipsum")
        );
        assert_eq!(
            find_placeholder("<h1>YOUR HEADLINE HERE</h1>"),
            Some("your headline here")
        );
        // "#REPLACE_ME" 含更早的 marker "replace_me"（.find 返回首个命中）。
        assert!(find_placeholder("<span>#REPLACE_ME</span>").is_some());
        assert!(find_placeholder("<p>真实的产品文案，没有占位。</p>").is_none());
    }

    #[test]
    fn evaluate_prioritizes_placeholder_over_thin() {
        // 近空 + 占位 → 报 placeholder（更具体）。
        let v = evaluate("<div>Lorem ipsum</div>", "").expect("flagged");
        assert_eq!(v.flag, "placeholder");
    }

    #[test]
    fn merge_metadata_sets_and_clears_only_self_check_key() {
        let existing = r#"{"author":"model","selfCheck":{"flag":"old"}}"#;
        let flag = SelfCheckFlag {
            flag: "thin",
            detail: "x".into(),
        };
        let merged = merge_into_metadata(Some(existing), Some(&flag)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["author"], "model", "其它键保留");
        assert_eq!(v["selfCheck"]["flag"], "thin", "selfCheck 键更新");
        // 清键：selfCheck 移除、author 保留。
        let cleared = merge_into_metadata(Some(&merged), None).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&cleared).unwrap();
        assert!(v2.get("selfCheck").is_none());
        assert_eq!(v2["author"], "model");
        // 清键后只剩空对象 → None。
        assert!(merge_into_metadata(Some(r#"{"selfCheck":{"flag":"x"}}"#), None).is_none());
    }

    #[test]
    fn near_identical_catches_duplicates_ignores_distinct() {
        // 可见文字须 ≥ 120 字，故用足够长的段落。
        let a = "<section><h1>定价方案</h1><p>我们提供三档灵活套餐，按团队规模与实际使用量按需选择，\
                 年付立省两成，支持随时升级或降级，无需长期锁定合约，也没有任何隐藏费用。企业档另配专属\
                 客户成功经理，从接入、培训到正式上线全程护航，确保团队顺利落地并持续获得价值，售后响应\
                 通常不超过一个工作日。</p></section>";
        let b = "<section><h1>定价方案</h1><p>我们提供三档灵活套餐，按团队规模与实际使用量按需选择，\
                 年付立省两成，支持随时升级或降级，无需长期锁定合约，也没有任何隐藏费用。企业档另配专属\
                 客户成功经理，从接入、培训到正式上线全程护航，确保团队顺利落地并持续获得价值，售后响应\
                 通常不超过一个工作日哦。</p></section>";
        assert!(near_identical(a, b), "仅尾字之差应判雷同");
        let c = "<section><h1>关于我们</h1><p>这是一支专注于工业设计的独立团队，深耕消费电子领域已逾十年，\
                 作品屡次斩获国际设计大奖，始终坚持以人为本、克制而温暖的设计哲学。我们相信好的设计源于对\
                 真实生活的观察，也源于对细节近乎苛刻的打磨，愿与每一位伙伴长期同行，共同创造经得起时间\
                 检验的作品。</p></section>";
        assert!(!near_identical(a, c), "主题不同不应判雷同");
    }

    #[test]
    fn near_identical_skips_too_short() {
        assert!(!near_identical("<p>hi</p>", "<p>hi</p>"), "过短样本不比较");
    }
}
