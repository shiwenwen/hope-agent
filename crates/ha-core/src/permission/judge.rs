//! Smart-mode judge model — independent `side_query` to a configured model
//! that returns `allow` / `ask` / `deny` per tool call.
//!
//! Skeleton only — the real implementation (5s timeout + LRU cache) lands
//! when Smart mode ships. See [`super::mode::JudgeModelConfig`].

use serde::{Deserialize, Serialize};

use super::mode::JudgeModelConfig;

/// Output schema enforced on the judge model's reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JudgeVerdict {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeResponse {
    pub decision: JudgeVerdict,
    /// One-line rationale shown in approval dialog / audit log.
    #[serde(default)]
    pub reason: String,
}

/// Run the judge model for one tool call. Returns `None` if the judge cannot
/// be reached (timeout, missing config, network error) — caller should
/// fallback per [`super::mode::SmartFallback`]. Stub for now.
pub async fn judge(
    _config: &JudgeModelConfig,
    _tool_name: &str,
    _args: &serde_json::Value,
) -> Option<JudgeResponse> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_verdict_serde_round_trip() {
        for v in [JudgeVerdict::Allow, JudgeVerdict::Ask, JudgeVerdict::Deny] {
            let s = serde_json::to_string(&v).unwrap();
            let v2: JudgeVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, v2);
        }
    }
}
