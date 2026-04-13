use std::fmt::Write;

use super::types::{FacetSummary, QuantitativeStats, RecapReport};

const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #0b0d12;
  --surface: #14171f;
  --border: #242833;
  --text: #e8eaf0;
  --muted: #98a0b0;
  --accent: #6366f1;
  --good: #22c55e;
  --warn: #f59e0b;
  --bad: #ef4444;
  --info: #38bdf8;
}
@media (prefers-color-scheme: light) {
  :root {
    --bg: #fafbfd; --surface: #ffffff; --border: #e3e6ec; --text: #14171f;
    --muted: #5b6373; --accent: #4f46e5;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  background: var(--bg); color: var(--text); line-height: 1.55; padding: 32px 24px 64px;
}
.wrap { max-width: 960px; margin: 0 auto; }
header { margin-bottom: 32px; }
h1 { font-size: 28px; margin: 0 0 4px; }
.meta { color: var(--muted); font-size: 14px; }
section.card {
  background: var(--surface); border: 1px solid var(--border); border-radius: 12px;
  padding: 20px 24px; margin: 16px 0;
}
section.card h2 { margin-top: 0; font-size: 18px; }
.kpi-grid {
  display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px;
}
.kpi {
  background: var(--surface); border: 1px solid var(--border); border-radius: 10px;
  padding: 12px 14px;
}
.kpi-label { color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
.kpi-value { font-size: 22px; font-weight: 600; margin-top: 4px; }
.bars { display: flex; flex-direction: column; gap: 6px; }
.bar-row { display: flex; align-items: center; gap: 8px; font-size: 13px; }
.bar-label { width: 40%; color: var(--muted); }
.bar-track { flex: 1; height: 10px; background: var(--border); border-radius: 6px; overflow: hidden; }
.bar-fill { height: 100%; background: var(--accent); }
.bar-value { width: 36px; text-align: right; color: var(--muted); font-variant-numeric: tabular-nums; }
.health-pill { display: inline-block; padding: 2px 10px; border-radius: 999px; font-size: 12px; font-weight: 600; }
.health-excellent { background: rgba(34,197,94,.15); color: var(--good); }
.health-good { background: rgba(56,189,248,.15); color: var(--info); }
.health-warning { background: rgba(245,158,11,.15); color: var(--warn); }
.health-critical { background: rgba(239,68,68,.15); color: var(--bad); }
.section-md p { margin: 8px 0; }
.section-md ul, .section-md ol { padding-left: 20px; margin: 8px 0; }
.section-md code { background: rgba(99,102,241,.12); padding: 1px 6px; border-radius: 4px; font-size: 12.5px; }
.section-md pre { background: rgba(0,0,0,.2); padding: 12px; border-radius: 8px; overflow-x: auto; font-size: 12.5px; }
.section-md strong { color: var(--text); }
.glance { background: linear-gradient(135deg, rgba(99,102,241,.12), rgba(56,189,248,.10)); }
.heatmap {
  display: grid; grid-template-columns: 28px repeat(24, 1fr); gap: 2px; font-size: 10px;
  color: var(--muted); margin-top: 8px;
}
.heatmap .cell { aspect-ratio: 1; border-radius: 2px; background: rgba(99,102,241,.10); }
.heatmap .lbl { text-align: right; padding-right: 4px; line-height: 1; }
"#;

/// Render a complete `RecapReport` as a self-contained HTML string.
pub fn render_html(report: &RecapReport) -> String {
    let mut out = String::with_capacity(8_192);
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{}</style></head><body><div class=\"wrap\">",
        escape(&report.meta.title),
        STYLE,
    );

    // Header
    out.push_str("<header><h1>");
    out.push_str(&escape(&report.meta.title));
    out.push_str("</h1><div class=\"meta\">Generated ");
    out.push_str(&escape(&report.meta.generated_at));
    out.push_str(" · model <code>");
    out.push_str(&escape(&report.meta.analysis_model));
    out.push_str("</code> · ");
    out.push_str(&format!("{} sessions", report.meta.session_count));
    out.push_str("</div></header>");

    // KPI grid
    render_kpis(&mut out, &report.quantitative);
    // Health
    render_health(&mut out, &report.quantitative);
    // Sections (at_a_glance is first, rendered with special class)
    for sec in &report.sections {
        let class = if sec.key == "at_a_glance" {
            "card glance"
        } else {
            "card"
        };
        let _ = write!(
            out,
            "<section class=\"{}\"><h2>{}</h2><div class=\"section-md\">{}</div></section>",
            class,
            escape(&sec.title),
            render_markdown(&sec.markdown),
        );
    }

    // Facet charts
    render_facet_charts(&mut out, &report.facet_summary);
    // Heatmap
    render_heatmap(&mut out, &report.quantitative);

    out.push_str("</div></body></html>");
    out
}

fn render_kpis(out: &mut String, q: &QuantitativeStats) {
    let cur = &q.overview.current;
    out.push_str("<section class=\"card\"><h2>Overview</h2><div class=\"kpi-grid\">");
    push_kpi(out, "Sessions", &cur.total_sessions.to_string());
    push_kpi(out, "Messages", &cur.total_messages.to_string());
    push_kpi(out, "Tool calls", &cur.total_tool_calls.to_string());
    push_kpi(out, "Errors", &cur.total_errors.to_string());
    push_kpi(
        out,
        "Cost",
        &format!("${:.2}", cur.estimated_cost_usd),
    );
    push_kpi(
        out,
        "In-tokens",
        &short_count(cur.total_input_tokens),
    );
    push_kpi(
        out,
        "Out-tokens",
        &short_count(cur.total_output_tokens),
    );
    if let Some(ttft) = cur.avg_ttft_ms {
        push_kpi(out, "Avg TTFT", &format!("{:.0} ms", ttft));
    }
    out.push_str("</div></section>");
}

fn render_health(out: &mut String, q: &QuantitativeStats) {
    let h = &q.health;
    let cls = match h.status.as_str() {
        "excellent" => "health-excellent",
        "good" => "health-good",
        "warning" => "health-warning",
        "critical" => "health-critical",
        _ => "health-good",
    };
    out.push_str("<section class=\"card\"><h2>Health</h2><p>Score: <strong>");
    let _ = write!(out, "{}/100</strong> <span class=\"health-pill {}\">{}</span></p>", h.score, cls, escape(&h.status));
    out.push_str("<div class=\"bars\">");
    push_bar(out, "Log error rate", h.log_error_rate_percent, 100.0);
    push_bar(out, "Tool error rate", h.tool_error_rate_percent, 100.0);
    push_bar(
        out,
        "Cron success rate",
        h.cron_success_rate_percent,
        100.0,
    );
    push_bar(
        out,
        "Subagent success rate",
        h.subagent_success_rate_percent,
        100.0,
    );
    out.push_str("</div></section>");
}

fn render_facet_charts(out: &mut String, f: &FacetSummary) {
    if f.total_facets == 0 {
        return;
    }
    out.push_str("<section class=\"card\"><h2>Facet breakdown</h2>");
    if !f.goal_histogram.is_empty() {
        out.push_str("<h3>Top goals</h3><div class=\"bars\">");
        let max = f.goal_histogram.iter().map(|(_, n)| *n).max().unwrap_or(1) as f64;
        for (k, n) in &f.goal_histogram {
            push_bar(out, k, *n as f64, max);
        }
        out.push_str("</div>");
    }
    if !f.outcome_distribution.is_empty() {
        out.push_str("<h3>Outcomes</h3><div class=\"bars\">");
        let max = f
            .outcome_distribution
            .iter()
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(1) as f64;
        for (k, n) in &f.outcome_distribution {
            push_bar(out, k, *n as f64, max);
        }
        out.push_str("</div>");
    }
    if !f.friction_top.is_empty() {
        out.push_str("<h3>Friction sources</h3><div class=\"bars\">");
        let max = f.friction_top.iter().map(|(_, n)| *n).max().unwrap_or(1) as f64;
        for (k, n) in &f.friction_top {
            push_bar(out, k, *n as f64, max);
        }
        out.push_str("</div>");
    }
    out.push_str("</section>");
}

fn render_heatmap(out: &mut String, q: &QuantitativeStats) {
    if q.heatmap.cells.is_empty() {
        return;
    }
    out.push_str(
        "<section class=\"card\"><h2>Activity heatmap</h2><div class=\"heatmap\">",
    );
    let max = q.heatmap.max_value.max(1) as f64;
    let days = ["S", "M", "T", "W", "T", "F", "S"];
    let mut grid = [[0u64; 24]; 7];
    for cell in &q.heatmap.cells {
        if cell.weekday < 7 && cell.hour < 24 {
            grid[cell.weekday as usize][cell.hour as usize] = cell.message_count;
        }
    }
    for (wi, row) in grid.iter().enumerate() {
        let _ = write!(out, "<div class=\"lbl\">{}</div>", days[wi]);
        for &n in row.iter() {
            let intensity = (n as f64 / max).clamp(0.0, 1.0);
            let alpha = 0.08 + intensity * 0.85;
            let _ = write!(
                out,
                "<div class=\"cell\" style=\"background:rgba(99,102,241,{:.2})\" title=\"{}\"></div>",
                alpha, n
            );
        }
    }
    out.push_str("</div></section>");
}

fn push_kpi(out: &mut String, label: &str, value: &str) {
    let _ = write!(
        out,
        "<div class=\"kpi\"><div class=\"kpi-label\">{}</div><div class=\"kpi-value\">{}</div></div>",
        escape(label),
        escape(value),
    );
}

fn push_bar(out: &mut String, label: &str, value: f64, max: f64) {
    let pct = if max > 0.0 {
        (value / max * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let _ = write!(
        out,
        "<div class=\"bar-row\"><span class=\"bar-label\">{}</span>\
         <span class=\"bar-track\"><span class=\"bar-fill\" style=\"width:{:.1}%\"></span></span>\
         <span class=\"bar-value\">{:.0}</span></div>",
        escape(label),
        pct,
        value,
    );
}

fn short_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Minimal markdown-to-HTML for AI sections. Handles paragraphs, headings,
/// bullet lists, bold, italic, inline code. Keeps the renderer dependency-free.
fn render_markdown(md: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    let mut paragraph = String::new();

    let flush_paragraph = |out: &mut String, p: &mut String| {
        if !p.trim().is_empty() {
            out.push_str("<p>");
            out.push_str(&inline_md(p));
            out.push_str("</p>");
        }
        p.clear();
    };
    let close_list = |out: &mut String, in_list: &mut bool| {
        if *in_list {
            out.push_str("</ul>");
            *in_list = false;
        }
    };

    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush_paragraph(&mut out, &mut paragraph);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            flush_paragraph(&mut out, &mut paragraph);
            close_list(&mut out, &mut in_list);
            out.push_str("<h4>");
            out.push_str(&inline_md(rest));
            out.push_str("</h4>");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            flush_paragraph(&mut out, &mut paragraph);
            close_list(&mut out, &mut in_list);
            out.push_str("<h3>");
            out.push_str(&inline_md(rest));
            out.push_str("</h3>");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            flush_paragraph(&mut out, &mut paragraph);
            if !in_list {
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str("<li>");
            out.push_str(&inline_md(rest));
            out.push_str("</li>");
            continue;
        }
        close_list(&mut out, &mut in_list);
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(trimmed);
    }
    flush_paragraph(&mut out, &mut paragraph);
    if in_list {
        out.push_str("</ul>");
    }
    out
}

fn inline_md(text: &str) -> String {
    // Escape HTML first, then replace markdown tokens. All markup markers
    // (`, *) are single-byte ASCII, so jumping past them preserves UTF-8
    // boundaries; non-marker bytes are emitted via char iteration.
    let escaped = escape(text);
    let bytes = escaped.as_bytes();
    let mut out = String::with_capacity(escaped.len() + 32);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // inline code
        if c == b'`' {
            if let Some(end) = find_single(bytes, i + 1, b'`') {
                out.push_str("<code>");
                out.push_str(&escaped[i + 1..end]);
                out.push_str("</code>");
                i = end + 1;
                continue;
            }
        }
        // **bold**
        if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(end) = find_double_star(bytes, i + 2) {
                out.push_str("<strong>");
                out.push_str(&escaped[i + 2..end]);
                out.push_str("</strong>");
                i = end + 2;
                continue;
            }
        }
        // *italic*
        if c == b'*' {
            if let Some(end) = find_single(bytes, i + 1, b'*') {
                out.push_str("<em>");
                out.push_str(&escaped[i + 1..end]);
                out.push_str("</em>");
                i = end + 1;
                continue;
            }
        }
        // Non-marker byte: walk one full UTF-8 char so we never slice inside
        // a multi-byte sequence.
        let char_len = utf8_char_len(c);
        let end = (i + char_len).min(escaped.len());
        out.push_str(&escaped[i..end]);
        i = end;
    }
    out
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte < 0xC0 {
        // Continuation byte in isolation (should not happen for valid UTF-8).
        1
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

fn find_single(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_double_star(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            return Some(i);
        }
        i += 1;
    }
    None
}
