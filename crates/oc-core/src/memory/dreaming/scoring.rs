//! Scoring / parsing for LLM-nominated promotion decisions.
//!
//! The Light-phase pipeline delegates ranking to the LLM: we hand it the
//! candidate list and ask it to return a JSON array of `{id, score,
//! title, rationale}` records, then apply `min_score` and `max_promote`
//! cutoffs on our side. This module parses the LLM output defensively.

use serde::Deserialize;

use super::types::PromotionRecord;

#[derive(Debug, Deserialize)]
struct RawNomination {
    id: serde_json::Value,
    #[serde(default)]
    score: Option<f32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
}

/// Parse the LLM response text into a list of `PromotionRecord`.
/// Accepts a JSON array at the top level, or a JSON object with a
/// "promotions" key holding the array. Silently skips malformed items.
pub fn parse_nominations(text: &str) -> Vec<PromotionRecord> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Tolerate fenced code blocks by extracting the first `[ ... ]` or
    // `{ ... }` span, so an LLM that wrapped its output in ```json ...
    // ``` still parses.
    let json_slice = extract_json_span(trimmed).unwrap_or(trimmed);

    // Try array first, then object with "promotions" field.
    let raw_list: Vec<RawNomination> =
        if let Ok(list) = serde_json::from_str::<Vec<RawNomination>>(json_slice) {
            list
        } else if let Ok(obj) = serde_json::from_str::<serde_json::Value>(json_slice) {
            obj.get("promotions")
                .and_then(|v| serde_json::from_value::<Vec<RawNomination>>(v.clone()).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

    raw_list
        .into_iter()
        .filter_map(|raw| {
            let id = coerce_id(&raw.id)?;
            let score = raw.score.unwrap_or(0.0).clamp(0.0, 1.0);
            let title = raw
                .title
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Memory #{}", id));
            let rationale = raw
                .rationale
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            Some(PromotionRecord {
                memory_id: id,
                score,
                title,
                rationale,
            })
        })
        .collect()
}

/// Apply `min_score` and `max_promote` cutoffs, sort descending by score.
pub fn filter_and_rank(
    mut records: Vec<PromotionRecord>,
    min_score: f32,
    max_promote: usize,
) -> Vec<PromotionRecord> {
    records.retain(|r| r.score >= min_score);
    records.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    records.truncate(max_promote);
    records
}

/// Accept numeric or stringified IDs ("42" / 42 / 42.0) since LLMs
/// occasionally quote them.
fn coerce_id(v: &serde_json::Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    if let Some(f) = v.as_f64() {
        return Some(f as i64);
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse::<i64>().ok();
    }
    None
}

/// Grab the first balanced JSON array or object from a string. Handles
/// markdown code fences by letting the scan start wherever the first `[`
/// or `{` appears.
fn extract_json_span(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes
        .iter()
        .position(|&b| b == b'[' || b == b'{')?;
    let open = bytes[start];
    let close = if open == b'[' { b']' } else { b'}' };

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_array() {
        let out = parse_nominations(r#"[{"id":42,"score":0.9,"title":"T","rationale":"R"}]"#);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].memory_id, 42);
        assert!((out[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn parses_fenced_code_block() {
        let out = parse_nominations(
            "Here's my answer:\n```json\n[{\"id\":\"7\",\"score\":0.8}]\n```",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].memory_id, 7);
    }

    #[test]
    fn parses_object_with_promotions_key() {
        let out = parse_nominations(r#"{"promotions":[{"id":3,"score":0.5}]}"#);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].memory_id, 3);
    }

    #[test]
    fn filters_below_threshold() {
        let records = vec![
            PromotionRecord {
                memory_id: 1,
                score: 0.5,
                title: "a".into(),
                rationale: String::new(),
            },
            PromotionRecord {
                memory_id: 2,
                score: 0.9,
                title: "b".into(),
                rationale: String::new(),
            },
        ];
        let out = filter_and_rank(records, 0.8, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].memory_id, 2);
    }

    #[test]
    fn caps_at_max_promote() {
        let records = (0..10)
            .map(|i| PromotionRecord {
                memory_id: i,
                score: 0.9,
                title: format!("t{}", i),
                rationale: String::new(),
            })
            .collect();
        let out = filter_and_rank(records, 0.0, 3);
        assert_eq!(out.len(), 3);
    }
}
