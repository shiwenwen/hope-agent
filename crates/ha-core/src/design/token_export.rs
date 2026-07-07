//! 设计系统 Token → 多平台开发者格式导出（CSS / SCSS / TS / Swift / Android / DTCG）。
//!
//! 纯函数：输入 token map（`--ds-*` → 值），输出各目标平台可直接落地的文件。供 owner 平面
//! （GUI 导出对话框）与 `design` 工具 `export_tokens` action 共用；无网络 / 无副作用 / 确定性。
//!
//! **单位约定**：Web 值原样保留在 CSS/SCSS/TS/DTCG；Swift 保留原始数字 + 注释原单位（不臆测
//! 换算）；Android 按业界通行惯例 px→dp(1:1)、rem/em→dp(×16 基准) 并把 CSS `#rrggbbaa` 转成
//! Android 的 `#aarrggbb`（ARGB）。非 hex 颜色 / 无法确定的值降级为注释或字符串资源，绝不产出
//! 编不过的文件。

use std::collections::BTreeMap;

/// Token 语义类型（决定各平台如何落地）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Color,
    Dimension,
    Duration,
    FontFamily,
    FontWeight,
    Number,
    Other,
}

/// 单个导出目标产物。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenExport {
    /// 目标标识（css / scss / ts / swift / android / dtcg）。
    pub format: String,
    /// 展示名。
    pub label: String,
    /// 建议文件名。
    pub filename: String,
    /// 语法高亮语言标识（css / scss / typescript / swift / xml / json）。
    pub language: String,
    /// 文件正文。
    pub content: String,
}

/// 生成全部目标格式（顺序固定）。空 token 也产出骨架文件（不 panic）。
pub fn export_all(tokens: &BTreeMap<String, String>) -> Vec<TokenExport> {
    vec![
        gen_css(tokens),
        gen_scss(tokens),
        gen_ts(tokens),
        gen_swift(tokens),
        gen_android(tokens),
        gen_dtcg(tokens),
    ]
}

// ── 分类 ────────────────────────────────────────────────────────

fn is_color(v: &str) -> bool {
    let s = v.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    const FNS: [&str; 10] = [
        "rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch(", "color(",
    ];
    FNS.iter().any(|p| s.starts_with(p))
}

fn split_unit(v: &str, units: &[&'static str]) -> Option<(f64, &'static str)> {
    let s = v.trim();
    for unit in units {
        if let Some(num) = s.strip_suffix(unit) {
            if let Ok(n) = num.trim().parse::<f64>() {
                return Some((n, unit));
            }
        }
    }
    None
}

const DIMENSION_UNITS: [&str; 9] = ["px", "rem", "em", "vh", "vw", "vmin", "vmax", "pt", "ch"];

/// 判定 token 语义类型（值优先，名称提示兜底）。
pub fn classify(name: &str, value: &str) -> TokenType {
    let n = name.to_ascii_lowercase();
    let v = value.trim();
    if is_color(v) {
        return TokenType::Color;
    }
    if n.contains("font-family") || n.contains("fontfamily") || (n.contains("font") && v.contains(','))
    {
        return TokenType::FontFamily;
    }
    if n.contains("weight") {
        return TokenType::FontWeight;
    }
    // ms 必须先于 s 匹配。
    if split_unit(v, &["ms", "s"]).is_some() {
        return TokenType::Duration;
    }
    if v.ends_with('%') && v.trim_end_matches('%').trim().parse::<f64>().is_ok() {
        return TokenType::Dimension;
    }
    if split_unit(v, &DIMENSION_UNITS).is_some() {
        return TokenType::Dimension;
    }
    if v.parse::<f64>().is_ok() {
        return TokenType::Number;
    }
    TokenType::Other
}

// ── 命名转换 ────────────────────────────────────────────────────

/// `--ds-color-primary` → `color-primary`（无 ds 前缀则剥 `--`）。
fn core_name(k: &str) -> &str {
    k.strip_prefix("--ds-").unwrap_or_else(|| k.trim_start_matches("--"))
}

/// `--ds-color-primary` → `colorPrimary`。
fn to_camel(k: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in core_name(k).chars() {
        if c == '-' || c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `--ds-color-primary` → `ds_color_primary`（Android 资源名：小写 + 下划线）。
fn to_snake(k: &str) -> String {
    let base = k.trim_start_matches("--").to_ascii_lowercase();
    base.chars()
        .map(|c| if c == '-' { '_' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// 值的前导数字（"16px" → 16.0，"1.5rem" → 1.5，"600" → 600.0）。
fn leading_number(v: &str) -> Option<f64> {
    let s = v.trim();
    let end = s
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[..end].parse::<f64>().ok()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// JS/TS 字符串字面量（双引号 + 转义）。
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── 各格式生成 ──────────────────────────────────────────────────

fn gen_css(tokens: &BTreeMap<String, String>) -> TokenExport {
    let mut body = String::from(":root {\n");
    for (k, v) in tokens {
        body.push_str(&format!("  {k}: {v};\n"));
    }
    body.push_str("}\n");
    TokenExport {
        format: "css".into(),
        label: "CSS Variables".into(),
        filename: "tokens.css".into(),
        language: "css".into(),
        content: body,
    }
}

fn gen_scss(tokens: &BTreeMap<String, String>) -> TokenExport {
    let mut body = String::new();
    for (k, v) in tokens {
        // --ds-color-primary → $ds-color-primary
        let name = k.trim_start_matches("--");
        body.push_str(&format!("${name}: {v};\n"));
    }
    TokenExport {
        format: "scss".into(),
        label: "SCSS".into(),
        filename: "_tokens.scss".into(),
        language: "scss".into(),
        content: body,
    }
}

fn gen_ts(tokens: &BTreeMap<String, String>) -> TokenExport {
    let mut body = String::from("export const tokens = {\n");
    for (k, v) in tokens {
        body.push_str(&format!("  {}: {},\n", to_camel(k), js_string(v)));
    }
    body.push_str("} as const\n\nexport type DesignTokens = typeof tokens\n");
    TokenExport {
        format: "ts".into(),
        label: "TypeScript".into(),
        filename: "tokens.ts".into(),
        language: "typescript".into(),
        content: body,
    }
}

const SWIFT_UICOLOR_EXT: &str = r##"
private extension UIColor {
    /// 从 `#RGB` / `#RRGGBB` / `#RRGGBBAA` 十六进制字符串构造；无法解析时回退 `.clear`。
    convenience init(ds hex: String) {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")).lowercased()
        var value: UInt64 = 0
        guard Scanner(string: s).scanHexInt64(&value) else {
            self.init(white: 0, alpha: 0); return
        }
        let r, g, b, a: CGFloat
        switch s.count {
        case 3:
            r = CGFloat((value >> 8) & 0xF) / 15; g = CGFloat((value >> 4) & 0xF) / 15
            b = CGFloat(value & 0xF) / 15; a = 1
        case 6:
            r = CGFloat((value >> 16) & 0xFF) / 255; g = CGFloat((value >> 8) & 0xFF) / 255
            b = CGFloat(value & 0xFF) / 255; a = 1
        case 8:
            r = CGFloat((value >> 24) & 0xFF) / 255; g = CGFloat((value >> 16) & 0xFF) / 255
            b = CGFloat((value >> 8) & 0xFF) / 255; a = CGFloat(value & 0xFF) / 255
        default:
            r = 0; g = 0; b = 0; a = 0
        }
        self.init(red: r, green: g, blue: b, alpha: a)
    }
}
"##;

/// 内置 `UIColor(ds:)` init 只解析 `#RGB` / `#RRGGBB` / `#RRGGBBAA`；其余（rgba/hsl/oklch/
/// 4 位 hex）走它会静默变透明，故 Swift 侧只把这三种真·hex 路由给它。
fn is_plain_hex(v: &str) -> bool {
    match v.trim().strip_prefix('#') {
        Some(h) => matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

fn gen_swift(tokens: &BTreeMap<String, String>) -> TokenExport {
    let mut body =
        String::from("import UIKit\n\n/// 设计系统 Token（自动生成，请勿手改）。\npublic enum DesignTokens {\n");
    let mut has_color = false;
    for (k, v) in tokens {
        let name = to_camel(k);
        match classify(k, v) {
            // 仅 3/6/8 位 hex 交给 UIColor(ds:)（init 能解析）；其余颜色保留原值字符串 + 提示
            // 手工转换，绝不让 rgba/oklch 静默变透明（review #1/#4）。
            TokenType::Color if is_plain_hex(v) => {
                has_color = true;
                body.push_str(&format!(
                    "    public static let {name} = UIColor(ds: {})\n",
                    swift_string(v)
                ));
            }
            TokenType::Color => body.push_str(&format!(
                "    public static let {name} = {} // non-hex color, convert manually\n",
                swift_string(v)
            )),
            TokenType::Dimension | TokenType::Number | TokenType::FontWeight => {
                if let Some(n) = leading_number(v) {
                    body.push_str(&format!("    public static let {name}: CGFloat = {n} // {v}\n"));
                } else {
                    body.push_str(&format!("    public static let {name} = {}\n", swift_string(v)));
                }
            }
            _ => body.push_str(&format!("    public static let {name} = {}\n", swift_string(v))),
        }
    }
    body.push_str("}\n");
    if has_color {
        body.push_str(SWIFT_UICOLOR_EXT);
    }
    TokenExport {
        format: "swift".into(),
        label: "Swift (iOS)".into(),
        filename: "DesignTokens.swift".into(),
        language: "swift".into(),
        content: body,
    }
}

fn swift_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// CSS 颜色 → Android hex（`#AARRGGBB` / `#RRGGBB`）；非 hex 返回 None。
fn to_android_hex(v: &str) -> Option<String> {
    let s = v.trim().strip_prefix('#')?.to_ascii_lowercase();
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let expand = |c: char| format!("{c}{c}");
    match s.len() {
        3 => {
            let mut it = s.chars();
            let (r, g, b) = (it.next()?, it.next()?, it.next()?);
            Some(format!("#{}{}{}", expand(r), expand(g), expand(b)))
        }
        6 => Some(format!("#{s}")),
        // CSS #rrggbbaa → Android #aarrggbb
        8 => Some(format!("#{}{}", &s[6..8], &s[0..6])),
        _ => None,
    }
}

/// CSS 尺寸 → Android dp 数值（px→1:1，rem/em→×16）；非确定单位返回 None。
fn to_android_dimen(v: &str) -> Option<String> {
    let (n, unit) = split_unit(v, &DIMENSION_UNITS)?;
    let dp = match unit {
        "px" | "pt" => n,
        "rem" | "em" => n * 16.0,
        _ => return None, // vh/vw/ch 等视口相对单位无 Android 等价
    };
    // 去掉多余小数（16.0 → 16）。
    let s = if dp.fract() == 0.0 {
        format!("{}", dp as i64)
    } else {
        format!("{dp}")
    };
    Some(format!("{s}dp"))
}

fn gen_android(tokens: &BTreeMap<String, String>) -> TokenExport {
    let mut body = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
    for (k, v) in tokens {
        let name = to_snake(k);
        match classify(k, v) {
            TokenType::Color => match to_android_hex(v) {
                Some(hex) => body.push_str(&format!("    <color name=\"{name}\">{hex}</color>\n")),
                None => body.push_str(&format!(
                    "    <!-- {name}: {} （非 hex 颜色，需手工转换） -->\n",
                    xml_escape(v)
                )),
            },
            TokenType::Dimension => match to_android_dimen(v) {
                Some(dp) => body.push_str(&format!("    <dimen name=\"{name}\">{dp}</dimen>\n")),
                None => body.push_str(&format!(
                    "    <item name=\"{name}\" type=\"string\">{}</item>\n",
                    xml_escape(v)
                )),
            },
            _ => body.push_str(&format!(
                "    <item name=\"{name}\" type=\"string\">{}</item>\n",
                xml_escape(v)
            )),
        }
    }
    body.push_str("</resources>\n");
    TokenExport {
        format: "android".into(),
        label: "Android XML".into(),
        filename: "design_tokens.xml".into(),
        language: "xml".into(),
        content: body,
    }
}

fn dtcg_type(t: TokenType) -> Option<&'static str> {
    match t {
        TokenType::Color => Some("color"),
        TokenType::Dimension => Some("dimension"),
        TokenType::Duration => Some("duration"),
        TokenType::FontFamily => Some("fontFamily"),
        TokenType::FontWeight => Some("fontWeight"),
        TokenType::Number => Some("number"),
        TokenType::Other => None,
    }
}

fn gen_dtcg(tokens: &BTreeMap<String, String>) -> TokenExport {
    use serde_json::{Map, Value};
    let mut root = Map::new();
    for (k, v) in tokens {
        let mut leaf = Map::new();
        leaf.insert("$value".into(), Value::String(v.clone()));
        if let Some(ty) = dtcg_type(classify(k, v)) {
            leaf.insert("$type".into(), Value::String(ty.into()));
        }
        let leaf = Value::Object(leaf);

        let segments: Vec<&str> = core_name(k).split('-').filter(|s| !s.is_empty()).collect();
        if !insert_nested(&mut root, &segments, leaf.clone()) {
            // 路径冲突（一个 token 名是另一个的段前缀，如 --ds-radius vs --ds-radius-md）→ 退化为
            // 扁平 key。用**完整 CSS 变量名**（含 `--ds-` 前缀）作 key：全局唯一、绝不撞任何裸段名
            // （如 "radius"），两个 token 都保留、且不产出「既是 token 又是 group」的非法 DTCG。
            root.insert(k.clone(), leaf);
        }
    }
    let content = serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default() + "\n";
    TokenExport {
        format: "dtcg".into(),
        label: "Design Tokens (DTCG)".into(),
        filename: "tokens.dtcg.json".into(),
        language: "json".into(),
        content,
    }
}

/// 把叶子插入嵌套树；遇到分支/叶子撞名返回 false（由调用方退化处理）。
fn insert_nested(node: &mut serde_json::Map<String, serde_json::Value>, path: &[&str], leaf: serde_json::Value) -> bool {
    use serde_json::Value;
    match path {
        [] => false,
        [last] => {
            if node.contains_key(*last) {
                return false;
            }
            node.insert((*last).to_string(), leaf);
            true
        }
        [head, rest @ ..] => {
            let child = node
                .entry((*head).to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            match child.as_object_mut() {
                // 该段已是叶子 token（叶子也是 Object，故必须查 `$value` 而非 as_object）→ 冲突：
                // 不把子 token 塞进叶子里造出「既是 token 又是 group」的非法 DTCG，退化扁平（review #2/#3）。
                Some(obj) if obj.contains_key("$value") => false,
                Some(obj) => insert_nested(obj, rest, leaf),
                None => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn classify_covers_types() {
        assert_eq!(classify("--ds-color-primary", "#2563eb"), TokenType::Color);
        assert_eq!(classify("--ds-color-overlay", "rgba(0,0,0,.5)"), TokenType::Color);
        assert_eq!(classify("--ds-space-4", "16px"), TokenType::Dimension);
        assert_eq!(classify("--ds-radius-md", "0.5rem"), TokenType::Dimension);
        assert_eq!(classify("--ds-motion-fast", "200ms"), TokenType::Duration);
        assert_eq!(classify("--ds-font-body", "Inter, sans-serif"), TokenType::FontFamily);
        assert_eq!(classify("--ds-font-weight-bold", "700"), TokenType::FontWeight);
        assert_eq!(classify("--ds-z-modal", "1000"), TokenType::Number);
        assert_eq!(classify("--ds-shadow-sm", "0 1px 2px rgba(0,0,0,.1)"), TokenType::Other);
    }

    #[test]
    fn css_and_scss_names() {
        let t = toks(&[("--ds-color-primary", "#2563eb")]);
        assert!(gen_css(&t).content.contains("--ds-color-primary: #2563eb;"));
        assert!(gen_scss(&t).content.contains("$ds-color-primary: #2563eb;"));
    }

    #[test]
    fn ts_camelcases_and_quotes() {
        let t = toks(&[("--ds-color-primary", "#2563eb"), ("--ds-space-4", "16px")]);
        let c = gen_ts(&t).content;
        assert!(c.contains("colorPrimary: \"#2563eb\","));
        assert!(c.contains("space4: \"16px\","));
        assert!(c.contains("as const"));
    }

    #[test]
    fn swift_colors_dims_and_helper() {
        let t = toks(&[("--ds-color-primary", "#2563eb"), ("--ds-space-4", "16px")]);
        let c = gen_swift(&t).content;
        assert!(c.contains("public static let colorPrimary = UIColor(ds: \"#2563eb\")"));
        assert!(c.contains("public static let space4: CGFloat = 16 // 16px"));
        assert!(c.contains("convenience init(ds hex: String)"), "含 hex 时应附 UIColor 扩展");
    }

    #[test]
    fn swift_omits_helper_without_color() {
        let t = toks(&[("--ds-space-4", "16px")]);
        assert!(!gen_swift(&t).content.contains("convenience init(ds hex"));
    }

    #[test]
    fn android_hex_alpha_and_dimen() {
        // #rrggbbaa → #aarrggbb
        assert_eq!(to_android_hex("#11223380").as_deref(), Some("#80112233"));
        assert_eq!(to_android_hex("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(to_android_hex("rgb(0,0,0)"), None);
        assert_eq!(to_android_dimen("16px").as_deref(), Some("16dp"));
        assert_eq!(to_android_dimen("1rem").as_deref(), Some("16dp"));
        assert_eq!(to_android_dimen("50vw"), None);
        let t = toks(&[("--ds-color-primary", "#2563eb"), ("--ds-space-4", "16px")]);
        let c = gen_android(&t);
        assert!(c.content.contains("<color name=\"ds_color_primary\">#2563eb</color>"));
        assert!(c.content.contains("<dimen name=\"ds_space_4\">16dp</dimen>"));
    }

    #[test]
    fn dtcg_nests_with_type() {
        let t = toks(&[("--ds-color-primary", "#2563eb"), ("--ds-space-4", "16px")]);
        let c = gen_dtcg(&t).content;
        let v: serde_json::Value = serde_json::from_str(&c).unwrap();
        assert_eq!(v["color"]["primary"]["$value"], "#2563eb");
        assert_eq!(v["color"]["primary"]["$type"], "color");
        assert_eq!(v["space"]["4"]["$value"], "16px");
        assert_eq!(v["space"]["4"]["$type"], "dimension");
    }

    #[test]
    fn swift_non_hex_color_stays_visible() {
        // rgba / 4 位 hex 颜色不走 UIColor(ds:)（内置 init 只解 3/6/8 位，否则运行时透明）。
        let t = toks(&[("--ds-color-overlay", "rgba(0,0,0,.5)"), ("--ds-color-tint", "#f00c")]);
        let c = gen_swift(&t).content;
        assert!(!c.contains("UIColor(ds:"), "非 hex 颜色不应路由给 UIColor(ds:)");
        assert!(c.contains("rgba(0,0,0,.5)") && c.contains("non-hex color"));
        assert!(!c.contains("convenience init(ds hex"), "无真 hex 颜色不应附 UIColor 扩展");
        // 真 hex 仍走 UIColor + 附扩展。
        let t2 = toks(&[("--ds-color-primary", "#2563eb")]);
        let c2 = gen_swift(&t2).content;
        assert!(c2.contains("UIColor(ds: \"#2563eb\")") && c2.contains("convenience init(ds hex"));
    }

    #[test]
    fn dtcg_prefix_collision_stays_valid() {
        fn collect_values(v: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(obj) = v.as_object() {
                if let Some(val) = obj.get("$value").and_then(|x| x.as_str()) {
                    out.push(val.to_string());
                }
                for cv in obj.values() {
                    collect_values(cv, out);
                }
            }
        }
        // 任一节点既含 $value 又有 group 子节点 = 非法 DTCG（既是 token 又是 group）。
        fn has_conflict(v: &serde_json::Value) -> bool {
            if let Some(obj) = v.as_object() {
                let leaf = obj.contains_key("$value");
                let group_child = obj.iter().any(|(k, cv)| !k.starts_with('$') && cv.is_object());
                if leaf && group_child {
                    return true;
                }
                return obj.values().any(has_conflict);
            }
            false
        }
        // 两种插入顺序都覆盖（BTreeMap 排序：短前缀先插）。
        for pairs in [
            &[("--ds-radius", "8px"), ("--ds-radius-md", "12px")][..],
            &[("--ds-color", "#2563eb"), ("--ds-color-primary", "#1d4ed8")][..],
        ] {
            let t = toks(pairs);
            let c = gen_dtcg(&t).content;
            let v: serde_json::Value = serde_json::from_str(&c).unwrap();
            assert!(!has_conflict(&v), "DTCG 不应有既 token 又 group 的节点: {c}");
            let mut vals = Vec::new();
            collect_values(&v, &mut vals);
            for (_, expect) in pairs {
                assert!(vals.iter().any(|x| x == expect), "缺 token 值 {expect}: {c}");
            }
        }
    }

    #[test]
    fn export_all_produces_six_targets() {
        let t = toks(&[("--ds-color-primary", "#2563eb")]);
        let all = export_all(&t);
        assert_eq!(all.len(), 6);
        let formats: Vec<_> = all.iter().map(|e| e.format.as_str()).collect();
        assert_eq!(formats, ["css", "scss", "ts", "swift", "android", "dtcg"]);
        // 每个都非空、有文件名。
        assert!(all.iter().all(|e| !e.content.is_empty() && !e.filename.is_empty()));
    }

    #[test]
    fn empty_tokens_still_valid() {
        let all = export_all(&BTreeMap::new());
        assert!(all[0].content.contains(":root {"));
        // DTCG 空对象仍是合法 JSON。
        assert!(serde_json::from_str::<serde_json::Value>(&all[5].content).is_ok());
    }
}
