//! **DESIGN.md 规范**：设计系统的可移植、人类可读单文件格式。
//!
//! 一个 DESIGN.md = **9 段 canonical schema**（品牌 / 色板 / 排印 / 间距 / 布局 / 组件 /
//! 动效 / 语气 / 反模式）+ 末尾 **Token 表**（`--ds-*` CSS 变量，机器可解析、可回灌渲染）。
//!
//! 这是设计系统的落盘正文格式（`DESIGN.md` 文件），也是**导入 / 导出的互通格式**：
//! - 导入：解析任意 DESIGN.md（表格 / `:root{}` / 内联皆可抽 token；缺则上层用 LLM 合成）。
//! - 导出：设计系统 → 规范 DESIGN.md（附 Token 表，可无损回灌）。

use std::collections::BTreeMap;

/// 设计系统落盘的正文文件名。
pub const DESIGN_MD_FILE: &str = "DESIGN.md";

/// canonical 9 段：`(key, 中文标题, 英文标题)`。导出按此顺序渲染；解析按关键词宽松匹配。
pub const SECTIONS: &[(&str, &str, &str)] = &[
    ("brand", "主题与品牌", "Brand"),
    ("palette", "色彩与角色", "Palette"),
    ("typography", "字体排印", "Typography"),
    ("spacing", "间距与网格", "Spacing"),
    ("layout", "布局与响应式", "Layout"),
    ("components", "组件样式", "Components"),
    ("motion", "动效", "Motion"),
    ("voice", "语气与文案", "Voice"),
    ("antipatterns", "禁忌与反模式", "Anti-patterns"),
];

/// 从任意 DESIGN.md 文本抽取 `--ds-*` token（支持 `:root{ --x: v }` / 表格 `| --x | v |`
/// / 内联 `--x: v`）。value 读到 `;` / `|` / 行尾（保留 rgba() / 字体栈里的逗号）。
pub fn extract_tokens(md: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in md.lines() {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while let Some(rel) = line[i..].find("--ds-") {
            let start = i + rel;
            // key = --ds-[a-z0-9-]+
            let mut e = start + 2;
            while e < bytes.len() {
                let c = bytes[e];
                if c.is_ascii_alphanumeric() || c == b'-' {
                    e += 1;
                } else {
                    break;
                }
            }
            let key = &line[start..e];
            // 分隔符前跳过空白 + 可选反引号（markdown 表格 `--ds-x`）。
            let mut j = e;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'`') {
                j += 1;
            }
            if j < bytes.len()
                && matches!(bytes[j], b':' | b'|' | b'=')
                && key.len() > "--ds-".len()
            {
                let after = &line[j + 1..];
                // value 读到 `;` / `|` / `}` / 行尾；去两端空白与反引号（CSS / 表格通用）。
                let val: String = after
                    .split([';', '|', '}'])
                    .next()
                    .unwrap_or(after)
                    .trim()
                    .trim_matches('`')
                    .trim()
                    .to_string();
                if !val.is_empty() {
                    out.insert(key.to_string(), val);
                }
            }
            i = e.max(start + 5);
        }
    }
    out
}

/// 首个 `# ` 标题 或 `> ` 引言 作为摘要（导入时用）。
pub fn extract_summary(md: &str) -> Option<String> {
    for line in md.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("> ") {
            if !h.trim().is_empty() {
                return Some(h.trim().to_string());
            }
        }
    }
    for line in md.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("# ") {
            if !h.trim().is_empty() {
                return Some(h.trim().to_string());
            }
        }
    }
    None
}

/// Token 表（markdown）——附到 DESIGN.md 末尾，机器可回灌。
pub fn tokens_table(tokens: &BTreeMap<String, String>) -> String {
    if tokens.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n## Tokens\n\n| Token | 值 (value) |\n| --- | --- |\n");
    for (k, v) in tokens {
        s.push_str(&format!("| `{k}` | `{v}` |\n"));
    }
    s
}

/// 是否已含 Token 表（避免导出时重复追加）。
fn has_tokens_table(md: &str) -> bool {
    md.contains("--ds-") && (md.contains("| Token") || md.contains("## Tokens"))
}

/// 设计系统 → 规范 DESIGN.md：保留正文 prose，确保末尾有 Token 表（可无损回灌）。
pub fn to_design_md(system_md: &str, tokens: &BTreeMap<String, String>) -> String {
    let body = system_md.trim_end();
    if has_tokens_table(body) {
        format!("{body}\n")
    } else {
        format!("{body}\n{}", tokens_table(tokens))
    }
}

/// 用当前 tokens **重建**末尾 Token 表（剥掉旧 `## Tokens` 段 + 附新表），保留正文 prose。
/// 落盘 chokepoint 用它保证 DESIGN.md ↔ tokens.json ↔ 导出/回灌永远一致——否则编辑 tokens
/// 却留旧表会让「唯一真相源」漂移、导出/再导入静默回退（token 编辑器 review #1）。
pub fn replace_tokens_table(system_md: &str, tokens: &BTreeMap<String, String>) -> String {
    // `## Tokens` 段在文末（`tokens_table` 追加处）；取最后一个 `## Tokens` 之前的正文。
    let body = match system_md.rfind("## Tokens") {
        Some(i) => &system_md[..i],
        None => system_md,
    }
    .trim_end();
    if tokens.is_empty() {
        format!("{body}\n")
    } else {
        format!("{body}\n{}", tokens_table(tokens))
    }
}

/// 空白 9 段 DESIGN.md 模板（供 agent / 用户按规范填写）。
pub fn template(name: &str, summary: &str) -> String {
    let mut s = format!("# {name} 设计系统\n\n> {summary}\n\n");
    for (idx, (_, zh, en)) in SECTIONS.iter().enumerate() {
        s.push_str(&format!("## {}. {zh} / {en}\n\n\n", idx + 1));
    }
    s.push_str("## Tokens\n\n| Token | 值 (value) |\n| --- | --- |\n| `--ds-color-primary` |  |\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tokens_css_table_inline() {
        let md = "\
# Sys\n:root{ --ds-color-primary: #2563eb; --ds-space-4: 16px }\n\
| `--ds-font-sans` | system-ui, sans-serif |\n\
--ds-shadow-md: 0 4px 20px rgba(15,23,42,.08);\n";
        let t = extract_tokens(md);
        assert_eq!(
            t.get("--ds-color-primary").map(String::as_str),
            Some("#2563eb")
        );
        assert_eq!(t.get("--ds-space-4").map(String::as_str), Some("16px"));
        // 字体栈 / rgba 里的逗号保留。
        assert_eq!(
            t.get("--ds-font-sans").map(String::as_str),
            Some("system-ui, sans-serif")
        );
        assert_eq!(
            t.get("--ds-shadow-md").map(String::as_str),
            Some("0 4px 20px rgba(15,23,42,.08)")
        );
    }

    #[test]
    fn summary_prefers_blockquote() {
        assert_eq!(
            extract_summary("# Title\n\n> 干净克制的现代语言\n\n## Palette"),
            Some("干净克制的现代语言".to_string())
        );
        assert_eq!(
            extract_summary("# Only Title\n\ntext"),
            Some("Only Title".to_string())
        );
        assert_eq!(extract_summary("no heading"), None);
    }

    #[test]
    fn round_trip_tokens() {
        let mut tokens = BTreeMap::new();
        tokens.insert("--ds-color-primary".to_string(), "#2563eb".to_string());
        tokens.insert("--ds-radius-md".to_string(), "10px".to_string());
        let md = to_design_md("# Sys\n\n> summary\n\n## Palette\n\nblue.", &tokens);
        // 导出含 Token 表且能被重新解析（无损回灌）。
        let re = extract_tokens(&md);
        assert_eq!(re, tokens);
        // 已有表则不重复追加。
        let again = to_design_md(&md, &tokens);
        assert_eq!(again.matches("## Tokens").count(), 1);
    }

    #[test]
    fn replace_tokens_table_reflects_edits_and_keeps_prose() {
        let mut old = BTreeMap::new();
        old.insert("--ds-color-primary".to_string(), "#2563eb".to_string());
        let md = to_design_md("# S\n\n> prose\n\n## Palette\n\n正文内容", &old);
        // 编辑：改主色 + 加新 token（模拟 token 编辑器保存旧 md + 新 tokens）。
        let mut edited = BTreeMap::new();
        edited.insert("--ds-color-primary".to_string(), "#ff0000".to_string());
        edited.insert("--ds-space-4".to_string(), "1rem".to_string());
        let rebuilt = replace_tokens_table(&md, &edited);
        assert!(rebuilt.contains("prose") && rebuilt.contains("正文内容"), "正文保留");
        assert!(!rebuilt.contains("#2563eb"), "旧 token 值应被剥掉");
        assert_eq!(rebuilt.matches("## Tokens").count(), 1, "不重复表");
        assert_eq!(extract_tokens(&rebuilt), edited, "回读 == 编辑后 tokens（真相源一致）");
    }

    #[test]
    fn template_has_nine_sections() {
        let t = template("Test", "sum");
        for (_, zh, _) in SECTIONS {
            assert!(t.contains(zh), "template missing section {zh}");
        }
    }
}
