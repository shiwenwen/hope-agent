//! 5 维质量评审门（反 AI-slop）。
//!
//! 对产物做**品牌契合 / 可访问性 / 视觉层次 / 可用性 / 性能**五维评审，返回每维评分
//! （0–10）+ 总分 + 可执行修复。走 [side_query](../../agent/side_query.rs) 降本
//! （复用分析 agent）。见 docs/architecture/design-space.md §11.1。

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CritiqueResult {
    /// 品牌契合。
    #[serde(default)]
    pub brand: f64,
    /// 可访问性（对比度 / 语义 / 可读性）。
    #[serde(default)]
    pub accessibility: f64,
    /// 视觉层次。
    #[serde(default)]
    pub hierarchy: f64,
    /// 可用性。
    #[serde(default)]
    pub usability: f64,
    /// 性能（结构简洁 / 无冗余）。
    #[serde(default)]
    pub performance: f64,
    /// 总分（五维均值，后端计算）。
    #[serde(default)]
    pub overall: f64,
    /// 一句话总评。
    #[serde(default)]
    pub summary: String,
    /// 可执行修复建议。
    #[serde(default)]
    pub fixes: Vec<String>,
}

fn clamp10(v: f64) -> f64 {
    (v.clamp(0.0, 10.0) * 10.0).round() / 10.0
}

/// 对产物 HTML（+ 可选设计系统 grounding）做 5 维评审。
pub async fn critique_html(html: &str, system_md: Option<&str>) -> Result<CritiqueResult> {
    let ground = system_md
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("\n\nThe artifact should follow this design system:\n{s}\n"))
        .unwrap_or_default();

    let prompt = format!(
        "You are a rigorous senior design reviewer. Critique the following self-contained HTML \
design artifact across 5 dimensions, each scored 0–10 (10 = excellent):\n\
- brand: consistency with the design system / cohesive visual language\n\
- accessibility: color contrast, semantic structure, readable type\n\
- hierarchy: clear visual hierarchy, focal point, spacing rhythm\n\
- usability: layout clarity, affordances, information order\n\
- performance: lean structure, no redundant/placeholder cruft (anti AI-slop)\n\
Be honest and specific. Penalize placeholder text (Lorem ipsum), identical repeated blocks, \
low contrast, and cramped layouts.{ground}\n\n\
Return ONLY a JSON object, no prose, no code fence, with keys: \
brand, accessibility, hierarchy, usability, performance (numbers 0–10), \
summary (one sentence), fixes (array of 2–5 concrete, actionable strings).\n\n\
HTML:\n{html}",
        ground = ground,
        html = truncate(html, 24000),
    );

    let config = crate::config::cached_config();
    let agent = build_critique_agent(&config).await?;
    let res = agent.side_query(&prompt, 1200).await?;
    let mut out = parse_critique(&res.text)?;
    out.brand = clamp10(out.brand);
    out.accessibility = clamp10(out.accessibility);
    out.hierarchy = clamp10(out.hierarchy);
    out.usability = clamp10(out.usability);
    out.performance = clamp10(out.performance);
    out.overall = clamp10(
        (out.brand + out.accessibility + out.hierarchy + out.usability + out.performance) / 5.0,
    );
    out.fixes.truncate(6);
    Ok(out)
}

/// Build the agent that runs the critique side-query. Honors the
/// `design.critiqueModel` ("providerId:modelId") override; otherwise reuses the shared
/// analysis agent (same model as `/recap`), which reuses the cache-warm side path.
async fn build_critique_agent(
    config: &crate::config::AppConfig,
) -> Result<crate::agent::AssistantAgent> {
    if let Some(over) = config
        .design
        .critique_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (pid, mid) = over.split_once(':').ok_or_else(|| {
            anyhow::anyhow!("invalid design.critiqueModel '{over}' (want 'providerId:modelId')")
        })?;
        let prov = crate::provider::find_provider(&config.providers, pid)
            .ok_or_else(|| anyhow::anyhow!("design.critiqueModel provider '{pid}' not found"))?;
        let agent = crate::agent::AssistantAgent::try_new_from_provider(prov, mid)
            .await?
            .with_failover_context(prov);
        return Ok(agent);
    }
    let (agent, _model) = crate::recap::report::build_analysis_agent(config).await?;
    Ok(agent)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // 安全按字符边界截断。
        match s.char_indices().nth(max) {
            Some((i, _)) => &s[..i],
            None => s,
        }
    }
}

/// 从模型返回里抽出 JSON（容忍代码围栏 / 前后缀噪声）。
fn parse_critique(text: &str) -> Result<CritiqueResult> {
    let t = text.trim();
    // 直接尝试。
    if let Ok(v) = serde_json::from_str::<CritiqueResult>(t) {
        return Ok(v);
    }
    // 抠出第一个 `{ … }`。
    if let (Some(start), Some(end)) = (t.find('{'), t.rfind('}')) {
        if end > start {
            if let Ok(v) = serde_json::from_str::<CritiqueResult>(&t[start..=end]) {
                return Ok(v);
            }
        }
    }
    anyhow::bail!("could not parse critique JSON from model output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_json() {
        let j = r#"{"brand":8,"accessibility":7,"hierarchy":9,"usability":8,"performance":7,"summary":"ok","fixes":["a","b"]}"#;
        let r = parse_critique(j).unwrap();
        assert_eq!(r.brand, 8.0);
        assert_eq!(r.fixes.len(), 2);
    }

    #[test]
    fn parse_fenced_json() {
        let j = "```json\n{\"brand\":6,\"summary\":\"meh\",\"fixes\":[]}\n```";
        let r = parse_critique(j).unwrap();
        assert_eq!(r.brand, 6.0);
    }

    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp10(12.0), 10.0);
        assert_eq!(clamp10(-3.0), 0.0);
        assert_eq!(clamp10(7.34), 7.3);
    }
}
