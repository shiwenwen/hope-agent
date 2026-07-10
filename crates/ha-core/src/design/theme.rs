//! 主题派生（决策增量）：从单一 light token 集**确定性**派生 dark / compact 变体——单 seed、
//! 无手写第二套、可单测。取代 Kit 页此前的「4 中性色表面切换」。
//!
//! - **dark**：bg→近黑（保 bg 色相）、fg→近白、muted/border→暗色面；accent 类保色相+饱和、
//!   钳最低亮度让其在暗底可读；非颜色 token 原样。
//! - **compact**：字号 / 间距 / 圆角按因子缩小；颜色 / 字体 / 阴影原样。
//!
//! 纯函数、零外部依赖，色彩走 HSL 保色相调亮度。

use std::collections::BTreeMap;

/// `#rgb` / `#rrggbb` → (r,g,b)。非法返回 None。
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().strip_prefix('#')?;
    let (r, g, b) = match h.len() {
        3 => {
            let c: Vec<u8> = h
                .chars()
                .map(|c| u8::from_str_radix(&c.to_string(), 16))
                .collect::<Result<_, _>>()
                .ok()?;
            (c[0] * 17, c[1] * 17, c[2] * 17)
        }
        6 => (
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ),
        _ => return None,
    };
    Some((r, g, b))
}

fn to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// RGB(0..=255) → HSL（h∈[0,360)、s,l∈[0,1]）。
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if (max - rf).abs() < f64::EPSILON {
        60.0 * (((gf - bf) / d).rem_euclid(6.0))
    } else if (max - gf).abs() < f64::EPSILON {
        60.0 * ((bf - rf) / d + 2.0)
    } else {
        60.0 * ((rf - gf) / d + 4.0)
    };
    (h.rem_euclid(360.0), s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let f = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (f(r1), f(g1), f(b1))
}

/// 保色相与饱和度、把亮度设为 `l`。非法 hex 原样返回。
fn set_lightness(hex: &str, l: f64) -> String {
    match parse_hex(hex) {
        Some((r, g, b)) => {
            let (h, s, _) = rgb_to_hsl(r, g, b);
            let (nr, ng, nb) = hsl_to_rgb(h, s, l.clamp(0.0, 1.0));
            to_hex(nr, ng, nb)
        }
        None => hex.to_string(),
    }
}

/// 亮度低于 `min` 则抬到 `min`（保色相饱和），否则原样。用于让 accent 在暗底可读。
fn ensure_min_lightness(hex: &str, min: f64) -> String {
    match parse_hex(hex) {
        Some((r, g, b)) => {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            if l < min {
                let (nr, ng, nb) = hsl_to_rgb(h, s, min);
                to_hex(nr, ng, nb)
            } else {
                hex.to_string()
            }
        }
        None => hex.to_string(),
    }
}

/// 从 light token 集确定性派生 dark 变体。
pub fn derive_dark(tokens: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in tokens {
        let dv = match k.as_str() {
            "--ds-color-bg" => set_lightness(v, 0.08),
            "--ds-color-fg" => set_lightness(v, 0.93),
            "--ds-color-surface" => set_lightness(v, 0.13),
            "--ds-color-muted" => set_lightness(v, 0.17),
            "--ds-color-border" => set_lightness(v, 0.30),
            "--ds-color-primary"
            | "--ds-color-secondary"
            | "--ds-color-accent"
            | "--ds-color-success"
            | "--ds-color-warning"
            | "--ds-color-danger" => ensure_min_lightness(v, 0.62),
            _ => v.clone(),
        };
        out.insert(k.clone(), dv);
    }
    out
}

/// 数值 token（`16px` / `1.5rem` / `24`）× factor，保单位；非数值原样。
fn scale_size(val: &str, factor: f64) -> String {
    let s = val.trim();
    let num_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    if num_end == 0 {
        return val.to_string();
    }
    match s[..num_end].parse::<f64>() {
        Ok(n) => {
            let unit = &s[num_end..];
            let scaled = n * factor;
            // px（及无单位）取整——子像素无意义、避免噪声；rem/em/% 保两位有效小数。
            if unit.eq_ignore_ascii_case("px") || unit.is_empty() {
                format!("{}{}", scaled.round() as i64, unit)
            } else {
                let r = (scaled * 100.0).round() / 100.0;
                format!("{r}{unit}")
            }
        }
        Err(_) => val.to_string(),
    }
}

/// 从 token 集确定性派生 compact 变体（字号 / 间距 / 圆角 × 0.82；颜色 / 字体 / 阴影不变）。
pub fn derive_compact(tokens: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    const COMPACT_FACTOR: f64 = 0.82;
    let mut out = BTreeMap::new();
    for (k, v) in tokens {
        let cv = if k.starts_with("--ds-text-")
            || k.starts_with("--ds-space-")
            || k.starts_with("--ds-radius-")
        {
            scale_size(v, COMPACT_FACTOR)
        } else {
            v.clone()
        };
        out.insert(k.clone(), cv);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks() -> BTreeMap<String, String> {
        [
            ("--ds-color-bg", "#ffffff"),
            ("--ds-color-fg", "#0f172a"),
            ("--ds-color-primary", "#2563eb"),
            ("--ds-color-muted", "#f1f5f9"),
            ("--ds-color-border", "#e2e8f0"),
            ("--ds-text-base", "16px"),
            ("--ds-space-4", "16px"),
            ("--ds-radius-md", "10px"),
            ("--ds-font-sans", "system-ui"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn hsl_roundtrip_stable() {
        for hex in ["#2563eb", "#ffffff", "#000000", "#0ea5e9", "#dc2626"] {
            let (r, g, b) = parse_hex(hex).unwrap();
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);
            assert!(
                (r as i16 - r2 as i16).abs() <= 1
                    && (g as i16 - g2 as i16).abs() <= 1
                    && (b as i16 - b2 as i16).abs() <= 1,
                "roundtrip {hex} -> {r2},{g2},{b2}"
            );
        }
    }

    #[test]
    fn dark_flips_bg_fg_and_keeps_hue() {
        let d = derive_dark(&toks());
        // bg 变暗、fg 变亮。
        let (_, _, bg_l) = {
            let (r, g, b) = parse_hex(&d["--ds-color-bg"]).unwrap();
            rgb_to_hsl(r, g, b)
        };
        let (_, _, fg_l) = {
            let (r, g, b) = parse_hex(&d["--ds-color-fg"]).unwrap();
            rgb_to_hsl(r, g, b)
        };
        assert!(bg_l < 0.15, "dark bg should be near-black, got L={bg_l}");
        assert!(fg_l > 0.85, "dark fg should be near-white, got L={fg_l}");
        // primary 保色相（蓝仍是蓝），亮度被抬到可读。
        let (ph, _, pl) = {
            let (r, g, b) = parse_hex(&d["--ds-color-primary"]).unwrap();
            rgb_to_hsl(r, g, b)
        };
        assert!((ph - 217.0).abs() < 15.0, "primary hue preserved (~blue)");
        assert!(pl >= 0.6, "primary lightened for dark legibility");
        // 非颜色 token 原样。
        assert_eq!(d["--ds-font-sans"], "system-ui");
    }

    #[test]
    fn compact_scales_sizes_only() {
        let c = derive_compact(&toks());
        assert_eq!(c["--ds-text-base"], "13px"); // 16*0.82=13.12 → round 13
        assert_eq!(c["--ds-space-4"], "13px");
        assert_eq!(c["--ds-radius-md"], "8px"); // 10*0.82=8.2 → 8
        // 颜色 / 字体不缩放。
        assert_eq!(c["--ds-color-primary"], "#2563eb");
        assert_eq!(c["--ds-font-sans"], "system-ui");
    }

    #[test]
    fn scale_size_preserves_unit_and_ignores_non_numeric() {
        assert_eq!(scale_size("16px", 0.82), "13px");
        assert_eq!(scale_size("1.5rem", 0.5), "0.75rem");
        assert_eq!(scale_size("auto", 0.82), "auto");
    }
}
