//! Deterministic, offline renderer for `AnalysisArtifactV1`.
//!
//! The JSON payload is the evidence contract. This module turns that contract
//! into a decision-oriented reading surface without trusting model-authored
//! HTML or requiring a JavaScript/chart runtime.

use std::fmt::Write as _;

use serde_json::Value;

use super::{escape_html, markdown_to_html, AnalysisArtifactV1, OFFLINE_CSP_STATIC};

pub(super) fn render(analysis: &AnalysisArtifactV1) -> String {
    let labels = Labels::for_analysis(analysis);
    let mut body = String::new();

    render_hero(&mut body, analysis, &labels);

    if let Some(first_block) = analysis.blocks.first() {
        render_answer(&mut body, first_block, &labels);
    }

    render_findings(&mut body, &analysis.findings, &labels);
    render_charts(&mut body, analysis, &labels);
    render_actions(&mut body, analysis, &labels);
    render_tables(&mut body, analysis, &labels);
    render_metrics(&mut body, &analysis.metric_definitions, &labels);
    render_supporting_blocks(&mut body, &analysis.blocks, &labels);
    render_validation(&mut body, analysis, &labels);
    render_sources(&mut body, &analysis.sources, &labels);

    format!(
        "<!DOCTYPE html><html lang=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
         <title>{}</title><style>{}</style></head><body><main class=\"report-shell\">{}</main></body></html>",
        labels.lang,
        OFFLINE_CSP_STATIC,
        escape_html(analysis.question.trim()),
        ANALYSIS_CSS,
        body
    )
}

struct Labels {
    lang: &'static str,
    report: &'static str,
    ready: &'static str,
    partial: &'static str,
    blocked: &'static str,
    audience: &'static str,
    decision: &'static str,
    time_range: &'static str,
    grain: &'static str,
    answer: &'static str,
    findings: &'static str,
    evidence: &'static str,
    recommendations: &'static str,
    caveats: &'static str,
    tables: &'static str,
    metrics: &'static str,
    validation: &'static str,
    claims: &'static str,
    sources: &'static str,
    checks: &'static str,
    passed: &'static str,
    warnings: &'static str,
    failed: &'static str,
    source: &'static str,
    rows: &'static str,
    confidence: &'static str,
    method: &'static str,
    observed: &'static str,
    formula: &'static str,
    numerator: &'static str,
    denominator: &'static str,
    window: &'static str,
    unit: &'static str,
}

impl Labels {
    fn for_analysis(analysis: &AnalysisArtifactV1) -> Self {
        let sample = format!(
            "{} {} {}",
            analysis.question, analysis.audience, analysis.decision
        );
        if sample.chars().any(is_cjk) {
            Self {
                lang: "zh-CN",
                report: "数据分析报告",
                ready: "已验证",
                partial: "部分完成",
                blocked: "已阻塞",
                audience: "分析对象",
                decision: "决策目标",
                time_range: "时间范围",
                grain: "数据粒度",
                answer: "结论摘要",
                findings: "关键发现",
                evidence: "证据与对比",
                recommendations: "建议行动",
                caveats: "风险与限制",
                tables: "明细数据",
                metrics: "指标口径",
                validation: "验证与质量",
                claims: "结论校验",
                sources: "来源与追溯",
                checks: "项检查",
                passed: "通过",
                warnings: "提醒",
                failed: "失败",
                source: "来源",
                rows: "行",
                confidence: "置信度",
                method: "方法",
                observed: "观测",
                formula: "公式",
                numerator: "分子",
                denominator: "分母",
                window: "窗口",
                unit: "单位",
            }
        } else {
            Self {
                lang: "en",
                report: "Data analysis report",
                ready: "Validated",
                partial: "Partial",
                blocked: "Blocked",
                audience: "Audience",
                decision: "Decision",
                time_range: "Time range",
                grain: "Data grain",
                answer: "Executive answer",
                findings: "Key findings",
                evidence: "Evidence & comparison",
                recommendations: "Recommended actions",
                caveats: "Risks & limitations",
                tables: "Supporting data",
                metrics: "Metric definitions",
                validation: "Validation & quality",
                claims: "Claim validation",
                sources: "Sources & lineage",
                checks: " checks",
                passed: "Passed",
                warnings: "Warnings",
                failed: "Failed",
                source: "Source",
                rows: "rows",
                confidence: "Confidence",
                method: "Method",
                observed: "Observed",
                formula: "Formula",
                numerator: "Numerator",
                denominator: "Denominator",
                window: "Window",
                unit: "Unit",
            }
        }
    }

    fn status<'a>(&self, status: &'a str) -> &'a str {
        match status {
            "ready" => self.ready,
            "partial" => self.partial,
            "blocked" => self.blocked,
            other => other,
        }
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff)
}

fn render_hero(out: &mut String, analysis: &AnalysisArtifactV1, labels: &Labels) {
    let status_class = match analysis.status.as_str() {
        "ready" => "ready",
        "blocked" => "blocked",
        _ => "partial",
    };
    let _ = write!(
        out,
        "<header class=\"report-hero\"><div class=\"hero-topline\"><span class=\"eyebrow\">{}</span><span class=\"status-pill {}\"><span class=\"status-dot\"></span>{}</span></div><h1>{}</h1>",
        labels.report,
        status_class,
        labels.status(&analysis.status),
        escape_html(analysis.question.trim())
    );

    let mut meta = Vec::new();
    if !analysis.audience.trim().is_empty() {
        meta.push((labels.audience, analysis.audience.trim().to_string()));
    }
    if !analysis.decision.trim().is_empty() {
        meta.push((labels.decision, analysis.decision.trim().to_string()));
    }
    if let Some(value) = analysis.time_range.as_ref().and_then(format_time_range) {
        meta.push((labels.time_range, value));
    }
    if let Some(grain) = analysis
        .grain
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        meta.push((labels.grain, grain.trim().to_string()));
    }
    if !meta.is_empty() {
        out.push_str("<div class=\"meta-grid\">");
        for (label, value) in meta {
            let _ = write!(
                out,
                "<div class=\"meta-item\"><span>{}</span><strong>{}</strong></div>",
                label,
                escape_html(&value)
            );
        }
        out.push_str("</div>");
    }
    out.push_str("</header>");
}

fn render_answer(out: &mut String, block: &Value, labels: &Labels) {
    let title = text(block, "title").unwrap_or(labels.answer);
    let Some(body) = text(block, "body").or_else(|| text(block, "markdown")) else {
        return;
    };
    let _ = write!(
        out,
        "<section class=\"answer-card\"><div class=\"answer-accent\"></div><div><p class=\"section-kicker\">{}</p><h2>{}</h2><div class=\"prose answer-prose\">{}</div></div></section>",
        labels.answer,
        escape_html(title),
        markdown_to_html(body)
    );
}

fn render_findings(out: &mut String, findings: &[Value], labels: &Labels) {
    if findings.is_empty() {
        return;
    }
    section_heading(out, labels.findings, None);
    out.push_str("<div class=\"finding-grid\">");
    for (index, finding) in findings.iter().enumerate() {
        let summary = value_summary(finding).unwrap_or("—");
        let confidence = finding.get("confidence").and_then(Value::as_f64);
        let _ = write!(
            out,
            "<article class=\"finding-card\"><div class=\"finding-index\">{:02}</div><div class=\"finding-copy\">{}</div>",
            index + 1,
            markdown_to_html(summary)
        );
        if let Some(confidence) = confidence {
            let _ = write!(
                out,
                "<span class=\"confidence\">{} · {:.0}%</span>",
                labels.confidence,
                confidence.clamp(0.0, 1.0) * 100.0
            );
        }
        out.push_str("</article>");
    }
    out.push_str("</div></section>");
}

fn render_charts(out: &mut String, analysis: &AnalysisArtifactV1, labels: &Labels) {
    if analysis.charts.is_empty() {
        return;
    }
    section_heading(out, labels.evidence, None);
    out.push_str("<div class=\"visual-grid\">");
    for chart in &analysis.charts {
        render_chart(out, analysis, chart, labels);
    }
    out.push_str("</div></section>");
}

fn render_chart(out: &mut String, analysis: &AnalysisArtifactV1, chart: &Value, labels: &Labels) {
    let title = text(chart, "title")
        .or_else(|| text(chart, "id"))
        .unwrap_or("Chart");
    let dataset_id = text(chart, "dataset")
        .or_else(|| text(chart, "datasetId"))
        .unwrap_or_default();
    let source_id = text(chart, "sourceId").unwrap_or_default();
    let x = text(chart, "x").unwrap_or_default();
    let y = text(chart, "y").unwrap_or_default();
    let unit = text(chart, "unit").unwrap_or_default();
    let chart_type = text(chart, "type").unwrap_or("bar");
    let dataset = analysis
        .datasets
        .iter()
        .find(|value| text(value, "id") == Some(dataset_id));
    let values = dataset
        .map(|dataset| chart_values(dataset, chart, x, y, unit))
        .unwrap_or_default();

    let _ = write!(
        out,
        "<figure class=\"chart-card\"><figcaption><div><h3>{}</h3><p>{}: <code>{}</code></p></div><span class=\"chart-type\">{}</span></figcaption>",
        escape_html(title),
        labels.source,
        escape_html(source_id),
        escape_html(chart_type)
    );
    if values.is_empty() {
        render_chart_fallback(out, analysis, chart);
    } else if chart_type.eq_ignore_ascii_case("line") {
        render_line_plot(out, &values, unit);
    } else {
        render_bar_plot(out, &values, unit);
    }
    out.push_str("</figure>");
}

fn chart_values(
    dataset: &Value,
    chart: &Value,
    x: &str,
    y: &str,
    unit: &str,
) -> Vec<(String, f64)> {
    let Some(rows) = dataset.get("rows").and_then(Value::as_array) else {
        return Vec::new();
    };
    let allowed = chart
        .get("filter")
        .and_then(Value::as_str)
        .and_then(parse_in_filter);
    let mut values = rows
        .iter()
        .filter_map(|row| {
            let label = row.get(x).map(value_text)?;
            if let Some(allowed) = allowed.as_ref() {
                if !allowed.iter().any(|item| item.eq_ignore_ascii_case(&label)) {
                    return None;
                }
            }
            let value = row.get(y).and_then(Value::as_f64)?;
            value.is_finite().then_some((label, value))
        })
        .collect::<Vec<_>>();
    if unit.eq_ignore_ascii_case("percent")
        && values.iter().map(|(_, value)| *value).fold(0.0, f64::max) <= 1.0
    {
        for (_, value) in &mut values {
            *value *= 100.0;
        }
    }
    values
}

fn parse_in_filter(filter: &str) -> Option<Vec<String>> {
    let start = filter.find('[')? + 1;
    let end = filter[start..].find(']')? + start;
    let values = filter[start..end]
        .split(',')
        .map(|value| value.trim().trim_matches(['\'', '"']).to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn render_bar_plot(out: &mut String, values: &[(String, f64)], unit: &str) {
    let domain = chart_domain(values, unit);
    let zero = domain.position(0.0);
    out.push_str("<div class=\"bar-plot\">");
    for (index, (label, value)) in values.iter().enumerate() {
        let position = domain.position(*value);
        let left = position.min(zero);
        let width = (position - zero).abs();
        let direction = if *value < 0.0 { "negative" } else { "positive" };
        let _ = write!(
            out,
            "<div class=\"bar-row\" aria-label=\"{} {}\"><span class=\"bar-label\">{}</span><span class=\"bar-track\"><span class=\"bar-zero\" style=\"left:{zero:.2}%\"></span><span class=\"bar-fill {direction} color-{}\" style=\"left:{left:.2}%;width:{width:.2}%\"></span></span><strong class=\"bar-value\">{}</strong></div>",
            escape_html(label),
            escape_html(&format_number(*value, unit)),
            escape_html(label),
            index % 6,
            escape_html(&format_number(*value, unit))
        );
    }
    out.push_str("</div>");
}

fn render_line_plot(out: &mut String, values: &[(String, f64)], unit: &str) {
    let width = 760.0;
    let height = 300.0;
    let left = 46.0;
    let right = 24.0;
    let top = 24.0;
    let bottom = 52.0;
    let domain = chart_domain(values, unit);
    let plot_width = width - left - right;
    let plot_height = height - top - bottom;
    let denominator = values.len().saturating_sub(1).max(1) as f64;
    let points = values
        .iter()
        .enumerate()
        .map(|(index, (_, value))| {
            let x = left + plot_width * index as f64 / denominator;
            let y = top + plot_height * (1.0 - domain.position(*value) / 100.0);
            (x, y)
        })
        .collect::<Vec<_>>();
    let point_list = points
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let _ = write!(
        out,
        "<div class=\"line-plot\"><svg viewBox=\"0 0 {width} {height}\" role=\"img\" preserveAspectRatio=\"xMidYMid meet\">"
    );
    for step in 0..=4 {
        let y = top + plot_height * step as f64 / 4.0;
        let value = domain.max - domain.span() * step as f64 / 4.0;
        let _ = write!(
            out,
            "<line class=\"grid-line\" x1=\"{left}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/><text class=\"axis-label\" x=\"{}\" y=\"{:.1}\">{}</text>",
            width - right,
            left - 8.0,
            y + 4.0,
            escape_html(&format_number(value, unit))
        );
    }
    let _ = write!(
        out,
        "<polyline class=\"line-series\" points=\"{point_list}\"/>"
    );
    for ((label, value), (x, y)) in values.iter().zip(points.iter()) {
        let _ = write!(
            out,
            "<circle class=\"line-point\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"5\"><title>{}: {}</title></circle><text class=\"x-label\" x=\"{x:.1}\" y=\"{:.1}\">{}</text>",
            escape_html(label),
            escape_html(&format_number(*value, unit)),
            height - 18.0,
            escape_html(label)
        );
    }
    out.push_str("</svg></div>");
}

#[derive(Clone, Copy, Debug)]
struct ChartDomain {
    min: f64,
    max: f64,
}

impl ChartDomain {
    fn span(self) -> f64 {
        (self.max - self.min).max(f64::EPSILON)
    }

    fn position(self, value: f64) -> f64 {
        ((value - self.min) / self.span() * 100.0).clamp(0.0, 100.0)
    }
}

fn chart_domain(values: &[(String, f64)], unit: &str) -> ChartDomain {
    let raw_min = values
        .iter()
        .map(|(_, value)| *value)
        .fold(f64::INFINITY, f64::min);
    let raw_max = values
        .iter()
        .map(|(_, value)| *value)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut min = raw_min.min(0.0);
    let mut max = raw_max.max(0.0);

    if unit.eq_ignore_ascii_case("percent") && raw_min >= 0.0 && raw_max <= 100.0 {
        max = 100.0;
    }
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON {
        min = 0.0;
        max = if unit.eq_ignore_ascii_case("percent") {
            100.0
        } else {
            1.0
        };
    }
    ChartDomain { min, max }
}

fn render_chart_fallback(out: &mut String, analysis: &AnalysisArtifactV1, chart: &Value) {
    let fallback_id = text(chart, "fallbackId").unwrap_or_default();
    let fallback = analysis
        .static_fallbacks
        .iter()
        .find(|value| text(value, "id") == Some(fallback_id));
    let message = fallback
        .and_then(value_summary)
        .unwrap_or("No plottable values were embedded; use the supporting table.");
    let _ = write!(
        out,
        "<div class=\"chart-fallback\">{}</div>",
        markdown_to_html(message)
    );
}

fn render_actions(out: &mut String, analysis: &AnalysisArtifactV1, labels: &Labels) {
    if analysis.recommendations.is_empty() && analysis.caveats.is_empty() {
        return;
    }
    out.push_str("<section class=\"report-section action-grid\">");
    if !analysis.recommendations.is_empty() {
        let _ = write!(
            out,
            "<div class=\"action-panel recommendations\"><p class=\"section-kicker\">{}</p><h2>{}</h2><ol>",
            labels.decision, labels.recommendations
        );
        let mut recommendations = analysis.recommendations.iter().collect::<Vec<_>>();
        recommendations.sort_by_key(|value| {
            value
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX)
        });
        for recommendation in recommendations {
            let summary = value_summary(recommendation).unwrap_or("—");
            let _ = write!(out, "<li><div>{}</div></li>", markdown_to_html(summary));
        }
        out.push_str("</ol></div>");
    }
    if !analysis.caveats.is_empty() {
        let _ = write!(
            out,
            "<div class=\"action-panel caveats\"><p class=\"section-kicker\">{}</p><h2>{}</h2><ul>",
            labels.validation, labels.caveats
        );
        for caveat in &analysis.caveats {
            let summary = value_summary(caveat).unwrap_or("—");
            let _ = write!(out, "<li>{}</li>", markdown_to_html(summary));
        }
        out.push_str("</ul></div>");
    }
    out.push_str("</section>");
}

fn render_tables(out: &mut String, analysis: &AnalysisArtifactV1, labels: &Labels) {
    if analysis.tables.is_empty() {
        return;
    }
    section_heading(out, labels.tables, None);
    for table in &analysis.tables {
        let title = text(table, "title")
            .or_else(|| text(table, "id"))
            .unwrap_or("Table");
        let (columns, rows, row_count) = table_data(analysis, table);
        let _ = write!(
            out,
            "<article class=\"table-card\"><h3>{}</h3>",
            escape_html(title)
        );
        render_html_table(out, analysis, table, &columns, &rows);
        let _ = write!(
            out,
            "<p class=\"table-note\">{} {}</p></article>",
            row_count, labels.rows
        );
    }
    out.push_str("</section>");
}

fn table_data(analysis: &AnalysisArtifactV1, table: &Value) -> (Vec<String>, Vec<Value>, u64) {
    let dataset_id = text(table, "datasetId").or_else(|| text(table, "dataset"));
    let dataset = dataset_id.and_then(|id| {
        analysis
            .datasets
            .iter()
            .find(|value| text(value, "id") == Some(id))
    });
    let columns = string_array(table.get("columns"))
        .or_else(|| dataset.and_then(|value| string_array(value.get("columns"))))
        .unwrap_or_default();
    let table_rows = table.get("rows").and_then(Value::as_array).cloned();
    let has_table_rows = table_rows.is_some();
    let rows = table_rows
        .or_else(|| dataset.and_then(|value| value.get("rows")?.as_array().cloned()))
        .unwrap_or_default();
    let row_count = table
        .get("rowCount")
        .and_then(Value::as_u64)
        .or_else(|| has_table_rows.then_some(rows.len() as u64))
        .or_else(|| dataset.and_then(|value| value.get("rowCount")?.as_u64()))
        .unwrap_or(rows.len() as u64);
    (columns, rows, row_count)
}

fn render_html_table(
    out: &mut String,
    analysis: &AnalysisArtifactV1,
    table: &Value,
    columns: &[String],
    rows: &[Value],
) {
    if columns.is_empty() {
        out.push_str("<p class=\"empty-state\">No display columns were provided.</p>");
        return;
    }
    let formats = columns
        .iter()
        .map(|column| resolve_column_format(analysis, table, column))
        .collect::<Vec<_>>();
    out.push_str("<div class=\"table-scroll\" tabindex=\"0\"><table><thead><tr>");
    for column in columns {
        let _ = write!(out, "<th>{}</th>", escape_html(&humanize(column)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for (column, format) in columns.iter().zip(formats.iter()) {
            let value = row.get(column).unwrap_or(&Value::Null);
            let _ = write!(
                out,
                "<td>{}</td>",
                escape_html(&format_table_value(value, format.as_ref()))
            );
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
}

fn render_metrics(out: &mut String, metrics: &[Value], labels: &Labels) {
    if metrics.is_empty() {
        return;
    }
    section_heading(out, labels.metrics, None);
    out.push_str("<div class=\"metric-grid\">");
    for metric in metrics {
        let label = text(metric, "label")
            .or_else(|| text(metric, "id"))
            .unwrap_or("Metric");
        let _ = write!(
            out,
            "<article class=\"metric-card\"><h3>{}</h3>",
            escape_html(label)
        );
        metric_line(out, labels.formula, text(metric, "formula"), true);
        metric_line(out, labels.numerator, text(metric, "numerator"), false);
        metric_line(out, labels.denominator, text(metric, "denominator"), false);
        metric_line(out, labels.window, text(metric, "window"), false);
        metric_line(out, labels.unit, text(metric, "unit"), false);
        out.push_str("</article>");
    }
    out.push_str("</div></section>");
}

fn metric_line(out: &mut String, label: &str, value: Option<&str>, code: bool) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    let tag = if code { "code" } else { "span" };
    let _ = write!(
        out,
        "<div class=\"metric-line\"><small>{}</small><{}>{}</{}></div>",
        label,
        tag,
        escape_html(value),
        tag
    );
}

fn render_supporting_blocks(out: &mut String, blocks: &[Value], labels: &Labels) {
    if blocks.len() <= 1 {
        return;
    }
    section_heading(out, labels.method, None);
    out.push_str("<div class=\"supporting-blocks\">");
    for block in blocks.iter().skip(1) {
        let title = text(block, "title").unwrap_or(labels.method);
        let body = text(block, "body").or_else(|| text(block, "markdown"));
        let Some(body) = body else {
            continue;
        };
        let _ = write!(
            out,
            "<article class=\"supporting-block\"><h3>{}</h3><div class=\"prose\">{}</div></article>",
            escape_html(title),
            markdown_to_html(body)
        );
    }
    out.push_str("</div></section>");
}

fn render_validation(out: &mut String, analysis: &AnalysisArtifactV1, labels: &Labels) {
    if analysis.data_quality.is_empty() && analysis.claim_validation.is_empty() {
        return;
    }
    section_heading(out, labels.validation, None);
    if !analysis.data_quality.is_empty() {
        let passed = status_count(&analysis.data_quality, "passed");
        let warnings = status_count(&analysis.data_quality, "warning")
            + status_count(&analysis.data_quality, "partial")
            + status_count(&analysis.data_quality, "inconclusive")
            + status_count(&analysis.data_quality, "not_run");
        let failed = status_count(&analysis.data_quality, "failed");
        let _ = write!(
            out,
            "<div class=\"quality-summary\"><div><strong>{}</strong><span>{}</span></div><div class=\"quality-count passed\"><strong>{passed}</strong><span>{}</span></div><div class=\"quality-count warning\"><strong>{warnings}</strong><span>{}</span></div><div class=\"quality-count failed\"><strong>{failed}</strong><span>{}</span></div></div>",
            analysis.data_quality.len(), labels.checks, labels.passed, labels.warnings, labels.failed
        );
        let _ = write!(
            out,
            "<details class=\"audit-details\"><summary>{} · {} {}</summary><div class=\"audit-list\">",
            labels.validation,
            analysis.data_quality.len(),
            labels.checks
        );
        for check in &analysis.data_quality {
            let name = text(check, "check").unwrap_or("check");
            let status = text(check, "status").unwrap_or("unknown");
            let method = text(check, "method").unwrap_or("—");
            let observed = check
                .get("observed")
                .map(compact_display)
                .unwrap_or_else(|| "—".to_string());
            let _ = write!(
                out,
                "<article class=\"audit-row\"><div class=\"audit-title\"><strong>{}</strong><span class=\"quality-badge {}\">{}</span></div><div class=\"audit-copy\"><p><b>{}:</b> {}</p><p><b>{}:</b> <code>{}</code></p></div></article>",
                escape_html(&humanize(name)),
                status_class(status),
                escape_html(status),
                labels.method,
                escape_html(method),
                labels.observed,
                escape_html(&observed)
            );
        }
        out.push_str("</div></details>");
    }
    if !analysis.claim_validation.is_empty() {
        let _ = write!(
            out,
            "<div class=\"claims-panel\"><h3>{}</h3>",
            labels.claims
        );
        for claim in &analysis.claim_validation {
            let summary = text(claim, "claim").unwrap_or("—");
            let verdict = text(claim, "verdict").unwrap_or("unknown");
            let method = text(claim, "method").unwrap_or("—");
            let mark = if verdict == "supported" { "✓" } else { "!" };
            let _ = write!(
                out,
                "<article class=\"claim-row\"><span class=\"claim-mark {}\">{}</span><div><strong>{}</strong><p>{}: {}</p></div><span class=\"quality-badge {}\">{}</span></article>",
                status_class(verdict),
                mark,
                escape_html(summary),
                labels.method,
                escape_html(method),
                status_class(verdict),
                escape_html(verdict)
            );
        }
        out.push_str("</div>");
    }
    out.push_str("</section>");
}

fn render_sources(out: &mut String, sources: &[Value], labels: &Labels) {
    if sources.is_empty() {
        return;
    }
    section_heading(out, labels.sources, None);
    out.push_str("<div class=\"source-list\">");
    for source in sources {
        let label = text(source, "label")
            .or_else(|| text(source, "title"))
            .or_else(|| text(source, "id"))
            .unwrap_or("Source");
        let source_type = text(source, "type").unwrap_or("data");
        let scope = text(source, "accessScope")
            .or_else(|| text(source, "access_scope"))
            .unwrap_or("unspecified");
        let hash = text(source, "sha256").unwrap_or("unhashed");
        let retrieved = text(source, "retrievedAt").unwrap_or_default();
        let _ = write!(
            out,
            "<article class=\"source-row\"><span class=\"source-icon\">{}</span><div><strong>{}</strong><p>{} · {}{}</p><code>{}</code></div></article>",
            escape_html(&source_type.chars().take(3).collect::<String>().to_uppercase()),
            escape_html(label),
            escape_html(source_type),
            escape_html(scope),
            if retrieved.is_empty() {
                String::new()
            } else {
                format!(" · {}", escape_html(retrieved))
            },
            escape_html(hash)
        );
    }
    out.push_str("</div></section>");
}

fn section_heading(out: &mut String, title: &str, description: Option<&str>) {
    let _ = write!(
        out,
        "<section class=\"report-section\"><div class=\"section-heading\"><h2>{}</h2>",
        title
    );
    if let Some(description) = description {
        let _ = write!(out, "<p>{}</p>", escape_html(description));
    }
    out.push_str("</div>");
}

fn status_count(values: &[Value], status: &str) -> usize {
    values
        .iter()
        .filter(|value| text(value, "status") == Some(status))
        .count()
}

fn status_class(status: &str) -> &'static str {
    match status {
        "passed" | "supported" | "ready" => "passed",
        "failed" | "unsupported" | "conflict" | "blocked" => "failed",
        _ => "warning",
    }
}

fn format_time_range(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.trim().is_empty()).then(|| text.trim().to_string());
    }
    let start = text(value, "start");
    let end = text(value, "end");
    match (start, end) {
        (Some(start), Some(end)) => Some(format!("{start} — {end}")),
        (Some(start), None) => Some(start.to_string()),
        (None, Some(end)) => Some(end.to_string()),
        _ => None,
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn value_summary(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| text(value, "summary"))
        .or_else(|| text(value, "text"))
        .or_else(|| text(value, "description"))
        .or_else(|| text(value, "claim"))
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array().map(|values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "—".to_string(),
        other => other.to_string(),
    }
}

fn compact_display(value: &Value) -> String {
    let raw = value_text(value);
    const LIMIT: usize = 320;
    if raw.chars().count() <= LIMIT {
        raw
    } else {
        format!("{}…", raw.chars().take(LIMIT).collect::<String>())
    }
}

fn humanize(value: &str) -> String {
    let value = value.replace(['_', '-'], " ");
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => value,
    }
}

#[derive(Debug)]
struct ColumnFormat {
    unit: String,
    scale: Option<String>,
}

fn resolve_column_format(
    analysis: &AnalysisArtifactV1,
    table: &Value,
    column: &str,
) -> Option<ColumnFormat> {
    if let Some(value) = table
        .get("columnFormats")
        .and_then(|formats| formats.get(column))
    {
        if let Some(unit) = value.as_str() {
            return Some(ColumnFormat {
                unit: unit.to_string(),
                scale: None,
            });
        }
        if let Some(unit) = text(value, "unit") {
            return Some(ColumnFormat {
                unit: unit.to_string(),
                scale: text(value, "scale").map(str::to_string),
            });
        }
    }

    let dataset_id = text(table, "datasetId").or_else(|| text(table, "dataset"));
    if let Some(chart) = analysis.charts.iter().find(|chart| {
        text(chart, "y") == Some(column)
            && text(chart, "dataset").or_else(|| text(chart, "datasetId")) == dataset_id
    }) {
        if let Some(unit) = text(chart, "unit") {
            return Some(ColumnFormat {
                unit: unit.to_string(),
                scale: text(chart, "valueScale")
                    .or_else(|| text(chart, "scale"))
                    .map(str::to_string)
                    .or_else(|| {
                        unit.eq_ignore_ascii_case("percent")
                            .then(|| "fraction".to_string())
                    }),
            });
        }
    }

    analysis
        .metric_definitions
        .iter()
        .find(|metric| text(metric, "id") == Some(column))
        .and_then(|metric| {
            text(metric, "unit").map(|unit| ColumnFormat {
                unit: unit.to_string(),
                scale: text(metric, "valueScale")
                    .or_else(|| text(metric, "scale"))
                    .map(str::to_string)
                    .or_else(|| {
                        unit.eq_ignore_ascii_case("percent")
                            .then(|| "fraction".to_string())
                    }),
            })
        })
}

fn format_table_value(value: &Value, format: Option<&ColumnFormat>) -> String {
    match value {
        Value::Number(number) => {
            let Some(value) = number.as_f64() else {
                return number.to_string();
            };
            if format.is_some_and(|format| format.unit.eq_ignore_ascii_case("percent")) {
                let percent = if format
                    .and_then(|format| format.scale.as_deref())
                    .is_some_and(|scale| scale.eq_ignore_ascii_case("fraction"))
                {
                    value * 100.0
                } else {
                    value
                };
                return format_percentage(percent);
            }
            if value.fract().abs() < f64::EPSILON {
                format!("{value:.0}")
            } else {
                trim_float(value, 3)
            }
        }
        Value::Bool(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Null => "—".to_string(),
        other => compact_display(other),
    }
}

fn format_number(value: f64, unit: &str) -> String {
    if unit.eq_ignore_ascii_case("percent") {
        format_percentage(value)
    } else {
        trim_float(value, 2)
    }
}

fn format_percentage(value: f64) -> String {
    format!("{}%", trim_float(value, 2))
}

fn trim_float(value: f64, precision: usize) -> String {
    let mut value = format!("{value:.precision$}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    value
}

const ANALYSIS_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#eef2f7;--paper:#fff;--surface:#f7f9fc;--surface-strong:#eef3fa;--ink:#152033;--muted:#667085;--line:#dfe6ef;--accent:#6d5ce7;--accent-2:#4f8cff;--accent-soft:#f0edff;--good:#0d8a60;--good-soft:#e8f7f0;--warn:#b86a00;--warn-soft:#fff4dc;--bad:#c53c4f;--bad-soft:#ffebee;--shadow:0 18px 55px rgba(24,39,75,.10);font-family:Inter,-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Noto Sans CJK SC",sans-serif}
*{box-sizing:border-box}html,body{width:100%;max-width:100%;min-width:0;overflow-x:hidden}body{margin:0;background:var(--bg);color:var(--ink);line-height:1.62}.report-shell{width:min(100%,1120px);min-width:0;margin:0 auto;padding:clamp(20px,4vw,56px)}.report-hero{overflow:hidden;border:1px solid rgba(109,92,231,.18);border-radius:28px;background:radial-gradient(circle at 88% 0%,rgba(79,140,255,.16),transparent 33%),linear-gradient(145deg,#fff 0%,#f8f7ff 58%,#f2f7ff 100%);padding:clamp(24px,5vw,58px);box-shadow:var(--shadow)}.hero-topline{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:22px}.eyebrow,.section-kicker{margin:0;color:var(--accent);font-size:.72rem;font-weight:800;letter-spacing:.15em;text-transform:uppercase}.status-pill{display:inline-flex;align-items:center;gap:8px;border:1px solid transparent;border-radius:999px;padding:7px 12px;font-size:.78rem;font-weight:750}.status-dot{width:7px;height:7px;border-radius:50%;background:currentColor;box-shadow:0 0 0 4px color-mix(in srgb,currentColor 15%,transparent)}.status-pill.ready{color:var(--good);background:var(--good-soft);border-color:color-mix(in srgb,var(--good) 20%,transparent)}.status-pill.partial{color:var(--warn);background:var(--warn-soft)}.status-pill.blocked{color:var(--bad);background:var(--bad-soft)}h1{max-width:900px;margin:0;font-size:clamp(2rem,4.8vw,4.2rem);font-weight:780;letter-spacing:-.045em;line-height:1.08;text-wrap:balance}.meta-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1px;margin-top:36px;overflow:hidden;border:1px solid var(--line);border-radius:16px;background:var(--line)}.meta-item{min-width:0;background:rgba(255,255,255,.74);padding:16px}.meta-item span{display:block;margin-bottom:6px;color:var(--muted);font-size:.72rem;font-weight:700;text-transform:uppercase;letter-spacing:.06em}.meta-item strong{display:block;font-size:.9rem;line-height:1.4;overflow-wrap:anywhere}.answer-card{position:relative;display:grid;grid-template-columns:6px minmax(0,1fr);gap:24px;margin:26px 0 0;padding:clamp(22px,3.2vw,36px);overflow:hidden;border:1px solid var(--line);border-radius:22px;background:var(--paper);box-shadow:0 12px 30px rgba(24,39,75,.06)}.answer-accent{border-radius:999px;background:linear-gradient(180deg,var(--accent),var(--accent-2))}.answer-card h2{margin:5px 0 8px;font-size:clamp(1.35rem,2.5vw,1.9rem)}.prose>:first-child{margin-top:0}.prose>:last-child{margin-bottom:0}.answer-prose{font-size:clamp(1.03rem,1.8vw,1.22rem);color:#344054}.answer-prose strong{color:var(--accent);font-weight:780}.report-section{min-width:0;margin-top:clamp(36px,6vw,68px)}.section-heading{display:flex;align-items:end;justify-content:space-between;gap:20px;margin-bottom:20px}.section-heading h2,.action-panel h2{margin:0;font-size:clamp(1.45rem,2.6vw,2rem);letter-spacing:-.025em}.section-heading p{margin:0;color:var(--muted)}.finding-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px}.finding-card{position:relative;min-width:0;min-height:190px;padding:22px;border:1px solid var(--line);border-radius:20px;background:var(--paper);box-shadow:0 8px 24px rgba(24,39,75,.045)}.finding-index{margin-bottom:26px;color:var(--accent);font-size:.75rem;font-weight:800;letter-spacing:.12em}.finding-copy{font-size:1rem;font-weight:590;line-height:1.55}.finding-copy p{margin:0}.confidence{position:absolute;right:18px;bottom:16px;color:var(--muted);font-size:.72rem}.visual-grid{display:grid;grid-template-columns:minmax(0,1fr);gap:18px}.chart-card{min-width:0;margin:0;padding:clamp(18px,3vw,28px);border:1px solid var(--line);border-radius:22px;background:var(--paper);box-shadow:0 10px 30px rgba(24,39,75,.05)}.chart-card figcaption{display:flex;align-items:start;justify-content:space-between;gap:18px;margin-bottom:28px}.chart-card h3{margin:0 0 5px;font-size:1.08rem}.chart-card figcaption p{margin:0;color:var(--muted);font-size:.78rem}.chart-card code,.source-row code{overflow-wrap:anywhere}.chart-type{border-radius:999px;background:var(--accent-soft);padding:5px 10px;color:var(--accent);font-size:.7rem;font-weight:750;text-transform:uppercase}.bar-plot{display:grid;gap:16px}.bar-row{display:grid;grid-template-columns:minmax(70px,130px) minmax(80px,1fr) minmax(58px,auto);align-items:center;gap:14px;min-width:0}.bar-label{overflow:hidden;color:#344054;font-size:.86rem;font-weight:650;text-overflow:ellipsis;white-space:nowrap}.bar-track{display:block;height:28px;overflow:hidden;border-radius:8px;background:var(--surface-strong)}.bar-fill{display:block;height:100%;min-width:2px;border-radius:8px;background:linear-gradient(90deg,var(--accent),var(--accent-2));animation:bar-in .55s cubic-bezier(.22,1,.36,1) both}.bar-fill.color-1{background:linear-gradient(90deg,#2787d9,#57b4e8)}.bar-fill.color-2{background:linear-gradient(90deg,#f08b45,#f4bc5d)}.bar-fill.color-3{background:linear-gradient(90deg,#18a278,#59c29f)}.bar-fill.color-4{background:linear-gradient(90deg,#d0589a,#e48bbb)}.bar-fill.color-5{background:linear-gradient(90deg,#7c6fe8,#a29af1)}.bar-value{text-align:right;font-size:.88rem;font-variant-numeric:tabular-nums}@keyframes bar-in{from{transform:scaleX(0);transform-origin:left}}.line-plot{width:100%;overflow:hidden}.line-plot svg{display:block;width:100%;height:auto}.grid-line{stroke:var(--line);stroke-width:1}.axis-label,.x-label{fill:var(--muted);font-size:11px}.axis-label{text-anchor:end}.x-label{text-anchor:middle}.line-series{fill:none;stroke:var(--accent);stroke-width:4;stroke-linecap:round;stroke-linejoin:round}.line-point{fill:var(--paper);stroke:var(--accent);stroke-width:3}.chart-fallback{border-radius:14px;background:var(--surface);padding:18px;color:var(--muted)}.chart-fallback p{margin:0}.action-grid{display:grid;grid-template-columns:minmax(0,1.25fr) minmax(0,.75fr);gap:18px}.action-panel{min-width:0;padding:clamp(22px,3vw,32px);border-radius:22px}.action-panel h2{margin:5px 0 22px}.recommendations{border:1px solid rgba(109,92,231,.18);background:linear-gradient(145deg,var(--accent-soft),#f8f7ff)}.recommendations ol{display:grid;gap:14px;margin:0;padding:0;counter-reset:recommendation}.recommendations li{display:grid;grid-template-columns:30px minmax(0,1fr);gap:12px;list-style:none;counter-increment:recommendation}.recommendations li:before{content:counter(recommendation);display:grid;width:28px;height:28px;place-items:center;border-radius:9px;background:var(--accent);color:#fff;font-size:.78rem;font-weight:800}.recommendations li p{margin:1px 0}.caveats{border:1px solid #f0dfbd;background:var(--warn-soft)}.caveats ul{display:grid;gap:12px;margin:0;padding-left:20px}.caveats li::marker{color:var(--warn)}.caveats p{margin:0}.table-card{min-width:0;margin-bottom:16px;padding:20px;border:1px solid var(--line);border-radius:20px;background:var(--paper)}.table-card h3{margin:0 0 14px}.table-scroll{width:100%;max-width:100%;overflow-x:auto;overscroll-behavior-x:contain;border:1px solid var(--line);border-radius:13px;background:var(--paper)}table{width:100%;min-width:560px;border-collapse:collapse;font-size:.86rem}th,td{padding:12px 14px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top;overflow-wrap:anywhere;word-break:break-word}th{position:sticky;top:0;background:var(--surface);color:#475467;font-size:.72rem;letter-spacing:.035em;text-transform:uppercase}tbody tr:last-child td{border-bottom:0}tbody tr:hover{background:var(--surface)}td{font-variant-numeric:tabular-nums}.table-note{margin:10px 2px 0;color:var(--muted);font-size:.76rem}.metric-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px}.metric-card{min-width:0;padding:22px;border:1px solid var(--line);border-radius:20px;background:var(--paper)}.metric-card h3{margin:0 0 18px}.metric-line{display:grid;grid-template-columns:90px minmax(0,1fr);gap:12px;padding:9px 0;border-top:1px solid var(--line)}.metric-line small{color:var(--muted);font-weight:650}.metric-line span,.metric-line code{min-width:0;overflow-wrap:anywhere}.quality-summary{display:grid;grid-template-columns:1.3fr repeat(3,1fr);overflow:hidden;border:1px solid var(--line);border-radius:18px;background:var(--paper)}.quality-summary>div{padding:18px;border-right:1px solid var(--line)}.quality-summary>div:last-child{border:0}.quality-summary strong,.quality-summary span{display:block}.quality-summary strong{font-size:1.5rem}.quality-summary span{color:var(--muted);font-size:.76rem}.quality-count.passed strong{color:var(--good)}.quality-count.warning strong{color:var(--warn)}.quality-count.failed strong{color:var(--bad)}.audit-details{margin-top:14px;border:1px solid var(--line);border-radius:18px;background:var(--paper)}.audit-details summary{cursor:pointer;padding:16px 18px;font-size:.88rem;font-weight:700}.audit-list{border-top:1px solid var(--line)}.audit-row{padding:17px 18px;border-bottom:1px solid var(--line)}.audit-row:last-child{border-bottom:0}.audit-title{display:flex;align-items:center;justify-content:space-between;gap:14px}.quality-badge{flex:none;border-radius:999px;padding:4px 9px;font-size:.68rem;font-weight:750;text-transform:uppercase}.quality-badge.passed,.claim-mark.passed{color:var(--good);background:var(--good-soft)}.quality-badge.warning,.claim-mark.warning{color:var(--warn);background:var(--warn-soft)}.quality-badge.failed,.claim-mark.failed{color:var(--bad);background:var(--bad-soft)}.audit-copy{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:8px;color:var(--muted);font-size:.78rem}.audit-copy p{margin:0;min-width:0;overflow-wrap:anywhere}.claims-panel{margin-top:18px;padding:20px;border:1px solid var(--line);border-radius:18px;background:var(--paper)}.claims-panel h3{margin:0 0 12px}.claim-row{display:grid;grid-template-columns:30px minmax(0,1fr) auto;align-items:start;gap:12px;padding:14px 0;border-top:1px solid var(--line)}.claim-mark{display:grid;width:28px;height:28px;place-items:center;border-radius:50%;font-weight:800}.claim-row strong{display:block}.claim-row p{margin:4px 0 0;color:var(--muted);font-size:.78rem}.source-list{display:grid;gap:10px}.source-row{display:grid;grid-template-columns:46px minmax(0,1fr);gap:14px;align-items:start;padding:16px;border:1px solid var(--line);border-radius:16px;background:var(--paper)}.source-icon{display:grid;width:44px;height:44px;place-items:center;border-radius:12px;background:var(--accent-soft);color:var(--accent);font-size:.7rem;font-weight:800}.source-row strong{display:block}.source-row p{margin:3px 0;color:var(--muted);font-size:.78rem}.source-row code{display:block;color:#475467;font-size:.7rem}.empty-state{color:var(--muted)}code{font-family:"SFMono-Regular",Consolas,"Liberation Mono",monospace}
/* Keep chart geometry deterministic for immediate PDF/screenshot capture. */
.bar-fill{min-width:0;animation:none}.supporting-blocks{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px}.supporting-block{min-width:0;padding:22px;border:1px solid var(--line);border-radius:20px;background:var(--paper)}.supporting-block h3{margin:0 0 12px}.supporting-block .prose{color:var(--muted)}
@media(max-width:820px){.meta-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.finding-grid{grid-template-columns:1fr}.finding-card{min-height:0}.action-grid{grid-template-columns:1fr}.metric-grid,.supporting-blocks{grid-template-columns:1fr}.quality-summary{grid-template-columns:repeat(3,1fr)}.quality-summary>div:first-child{grid-column:1/-1;border-bottom:1px solid var(--line)}.audit-copy{grid-template-columns:1fr}}
@media(max-width:560px){.report-shell{padding:12px}.report-hero{border-radius:20px;padding:22px}.hero-topline{align-items:flex-start}.eyebrow{max-width:55%}h1{font-size:1.85rem}.meta-grid{grid-template-columns:1fr}.answer-card{grid-template-columns:4px minmax(0,1fr);gap:16px;padding:20px}.report-section{margin-top:34px}.bar-row{grid-template-columns:72px minmax(60px,1fr) 54px;gap:8px}.bar-label,.bar-value{font-size:.75rem}.bar-track{height:23px}.chart-card{padding:16px}.chart-card figcaption{margin-bottom:20px}.quality-summary>div{padding:14px}.quality-summary strong{font-size:1.2rem}.claim-row{grid-template-columns:28px minmax(0,1fr)}.claim-row>.quality-badge{grid-column:2;justify-self:start}.metric-line{grid-template-columns:1fr;gap:3px}table{min-width:520px}}
@media(prefers-color-scheme:dark){:root{--bg:#0d1118;--paper:#151b25;--surface:#1a2230;--surface-strong:#212b3b;--ink:#f0f3f8;--muted:#9da9ba;--line:#2a3546;--accent:#a69cff;--accent-2:#63a5ff;--accent-soft:#252044;--good:#4fd0a1;--good-soft:#163a30;--warn:#f1b75d;--warn-soft:#3a2c17;--bad:#ff7b8d;--bad-soft:#3b2027;--shadow:0 20px 60px rgba(0,0,0,.32)}.report-hero{background:radial-gradient(circle at 88% 0%,rgba(79,140,255,.18),transparent 35%),linear-gradient(145deg,#171d28,#17172b)}.meta-item{background:rgba(21,27,37,.82)}.answer-prose,.bar-label{color:#d1d8e3}.source-row code{color:#aeb9c8}}
@media print{:root{--bg:#fff;--paper:#fff;--surface:#f7f7f8;--surface-strong:#eee;--ink:#111;--muted:#555;--line:#ddd;--shadow:none}body{background:#fff}.report-shell{width:100%;max-width:none;padding:0}.report-hero,.answer-card,.finding-card,.chart-card,.table-card,.metric-card,.action-panel,.quality-summary,.audit-details,.claims-panel,.source-row{box-shadow:none;break-inside:avoid}.audit-details>div{display:block}.bar-fill{print-color-adjust:exact;-webkit-print-color-adjust:exact}}
:root{--accent-panel-end:#f8f7ff;--prose:#344054;--heading-muted:#475467;--warn-border:#f0dfbd}
.answer-prose,.bar-label{color:var(--prose)}.bar-track{position:relative}.bar-zero{position:absolute;z-index:2;top:0;bottom:0;width:1px;background:color-mix(in srgb,var(--muted) 55%,transparent)}.bar-fill{position:absolute;display:block;top:0;bottom:0;height:auto;min-width:0;transform-origin:left}.bar-fill.negative{transform-origin:right}.recommendations{background:linear-gradient(145deg,var(--accent-soft),var(--accent-panel-end))}.caveats{border-color:var(--warn-border)}th{color:var(--heading-muted)}.source-row code{color:var(--muted)}
@keyframes bar-in{from{transform:scaleX(0)}}
@media(prefers-color-scheme:dark){:root{--accent-panel-end:#1c2340;--prose:#d1d8e3;--heading-muted:#aeb9c8;--warn-border:#5b4522}}
@media print{:root{--accent-panel-end:#f8f7ff;--prose:#344054;--heading-muted:#475467;--warn-border:#f0dfbd}.report-hero{background:#fff}.meta-item{background:#fff}.answer-prose,.bar-label{color:var(--prose)}}
@media(prefers-reduced-motion:reduce){.bar-fill{animation:none}}
"#;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn analysis_with_dataset() -> AnalysisArtifactV1 {
        AnalysisArtifactV1 {
            schema_version: "hope.analysis-artifact.v1".to_string(),
            question: "Test".to_string(),
            audience: String::new(),
            decision: String::new(),
            status: "ready".to_string(),
            metric_definitions: Vec::new(),
            time_range: None,
            filters: Vec::new(),
            grain: None,
            datasets: vec![json!({
                "id": "metrics",
                "columns": ["segment", "exchange_rate", "activation_rate"],
                "rowCount": 2,
                "rows": [
                    {"segment": "a", "exchange_rate": 7.2, "activation_rate": 0.64},
                    {"segment": "b", "exchange_rate": 7.3, "activation_rate": 0.50}
                ]
            })],
            findings: Vec::new(),
            recommendations: Vec::new(),
            caveats: Vec::new(),
            blocks: Vec::new(),
            charts: Vec::new(),
            tables: Vec::new(),
            static_fallbacks: Vec::new(),
            sources: Vec::new(),
            data_quality: Vec::new(),
            claim_validation: Vec::new(),
        }
    }

    #[test]
    fn signed_chart_domain_keeps_negative_values_distinct_from_zero() {
        let values = vec![("loss".to_string(), -20.0), ("gain".to_string(), 10.0)];
        let domain = chart_domain(&values, "number");

        assert_eq!(domain.min, -20.0);
        assert_eq!(domain.max, 10.0);
        assert!(domain.position(-20.0) < domain.position(0.0));
        assert!(domain.position(10.0) > domain.position(0.0));
    }

    #[test]
    fn explicit_empty_table_projection_never_falls_back_to_dataset() {
        let analysis = analysis_with_dataset();
        let table = json!({
            "datasetId": "metrics",
            "columns": [],
            "rows": []
        });
        let (columns, rows, row_count) = table_data(&analysis, &table);

        assert!(columns.is_empty());
        assert!(rows.is_empty());
        assert_eq!(row_count, 0);
    }

    #[test]
    fn curated_table_rows_use_their_own_count() {
        let analysis = analysis_with_dataset();
        let table = json!({
            "datasetId": "metrics",
            "columns": ["segment"],
            "rows": [{"segment": "a"}]
        });
        let (_, rows, row_count) = table_data(&analysis, &table);

        assert_eq!(rows.len(), 1);
        assert_eq!(row_count, 1);
    }

    #[test]
    fn table_numbers_are_only_percent_formatted_with_semantic_metadata() {
        let analysis = analysis_with_dataset();
        let table = json!({
            "datasetId": "metrics",
            "columnFormats": {
                "activation_rate": {"unit": "percent", "scale": "fraction"}
            }
        });
        let exchange_format = resolve_column_format(&analysis, &table, "exchange_rate");
        let activation_format = resolve_column_format(&analysis, &table, "activation_rate");

        assert_eq!(
            format_table_value(&json!(7.2), exchange_format.as_ref()),
            "7.2"
        );
        assert_eq!(
            format_table_value(&json!(0.64), activation_format.as_ref()),
            "64%"
        );
    }
}
