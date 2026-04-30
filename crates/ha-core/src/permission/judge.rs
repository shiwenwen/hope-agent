//! Smart-mode judge model — independent `side_query` to a configured model
//! that returns `allow` / `ask` / `deny` per tool call.
//!
//! Triggered from [`super::engine::resolve_async`] when:
//! - `session_mode == Smart`
//! - sync engine returned [`super::Decision::Ask`] with a non-strict reason
//! - `SmartStrategy ∈ { JudgeModel, Both }`
//! - `JudgeModelConfig` is set in `AppConfig.permission.smart.judge_model`
//!
//! Hardened by:
//! - 5 s hard timeout via `tokio::time::timeout`
//! - 60 s TTL cache keyed on `(tool_name, args_hash, provider_id, model)`
//!   to amortize repeat tool calls (model re-trying the same args mid-loop)
//!
//! Cache miss flow: build prompt → bare one-shot LLM call → strip code fences
//! → parse JSON. Any failure (timeout, network, malformed JSON) returns
//! `None`, letting the caller fall back per `SmartFallback`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::mode::JudgeModelConfig;
use crate::agent::AssistantAgent;

/// Hard timeout for the judge model side query. The chat loop blocks on
/// this — if the judge is slow we'd rather fall back than stall the user.
const JUDGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Cache TTL — repeated identical calls within this window reuse the
/// previous verdict instead of paying for another LLM round trip.
const JUDGE_CACHE_TTL: Duration = Duration::from_secs(60);

/// Soft cap for the cache. Tool loops retrying with mutated args produce
/// fresh keys, so a small bounded cache (cleared on overflow) is plenty.
const JUDGE_CACHE_CAP: usize = 256;

/// Token budget for the judge reply. The expected JSON is ~50 tokens; we
/// leave headroom for chain-of-thought reasoning models that emit hidden
/// scratch text before the answer.
const JUDGE_MAX_TOKENS: u32 = 256;

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
/// be reached (timeout, missing config, network error, malformed reply) —
/// caller should fall back per [`super::mode::SmartFallback`].
pub async fn judge(
    config: &JudgeModelConfig,
    tool_name: &str,
    args: &Value,
) -> Option<JudgeResponse> {
    let key = cache_key(tool_name, args, &config.provider_id, &config.model);
    if let Some(cached) = lookup_cache(key) {
        return Some(cached);
    }

    let app_cfg = crate::config::cached_config();
    let provider_cfg = app_cfg.providers.iter().find(|p| p.id == config.provider_id)?;

    let prompt = build_prompt(config, tool_name, args);

    let start = Instant::now();
    let raw = match tokio::time::timeout(
        JUDGE_TIMEOUT,
        AssistantAgent::judge_one_shot(provider_cfg, &config.model, &prompt, JUDGE_MAX_TOKENS),
    )
    .await
    {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            app_warn!(
                "permission",
                "judge",
                "Judge side_query failed: provider={} model={} tool={} err={}",
                config.provider_id,
                config.model,
                tool_name,
                e
            );
            return None;
        }
        Err(_) => {
            app_warn!(
                "permission",
                "judge",
                "Judge side_query timed out after {}s: provider={} model={} tool={}",
                JUDGE_TIMEOUT.as_secs(),
                config.provider_id,
                config.model,
                tool_name
            );
            return None;
        }
    };

    let parsed = parse_response(&raw)?;
    app_info!(
        "permission",
        "judge",
        "Judge verdict: tool={} decision={:?} latency_ms={}",
        tool_name,
        parsed.decision,
        start.elapsed().as_millis()
    );
    insert_cache(key, parsed.clone());
    Some(parsed)
}

// ── Prompt + parsing ────────────────────────────────────────────────

fn build_prompt(config: &JudgeModelConfig, tool_name: &str, args: &Value) -> String {
    let mut prompt = String::with_capacity(1024);
    prompt.push_str(
        "You are a security-conscious permission judge for an AI coding assistant. \
         The assistant is about to call a tool. Decide whether the call is safe \
         to execute, requires explicit user confirmation, or should be blocked.\n\n",
    );
    prompt.push_str(&format!("Tool: {}\n", tool_name));
    prompt.push_str(&format!("Arguments (JSON): {}\n\n", args));

    if let Some(extra) = &config.extra_prompt {
        if !extra.trim().is_empty() {
            prompt.push_str("Additional context from the user:\n");
            prompt.push_str(extra.trim());
            prompt.push_str("\n\n");
        }
    }

    prompt.push_str(
        "Heuristics:\n\
         - 'allow' when the call is clearly low-risk in this context (read-only, \
           inside a known project directory, idempotent, easily reversible).\n\
         - 'ask' when uncertain — the user should confirm.\n\
         - 'deny' only for clearly malicious or destructive intent.\n\n\
         Respond with EXACTLY one JSON object on a single line, no markdown, \
         no commentary:\n\
         {\"decision\":\"allow\"|\"ask\"|\"deny\",\"reason\":\"<one short sentence>\"}\n",
    );

    prompt
}

/// Tolerates models that wrap the JSON in markdown fences or trailing text.
fn parse_response(text: &str) -> Option<JudgeResponse> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end < start {
        return None;
    }
    let json_part = &trimmed[start..=end];
    serde_json::from_str(json_part).ok()
}

// ── Cache ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct CachedVerdict {
    response: JudgeResponse,
    expires_at: Instant,
}

fn cache() -> &'static Mutex<HashMap<u64, CachedVerdict>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, CachedVerdict>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(tool_name: &str, args: &Value, provider_id: &str, model: &str) -> u64 {
    let mut h = DefaultHasher::new();
    tool_name.hash(&mut h);
    args.to_string().hash(&mut h);
    provider_id.hash(&mut h);
    model.hash(&mut h);
    h.finish()
}

fn lookup_cache(key: u64) -> Option<JudgeResponse> {
    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if let Some(entry) = map.get(&key) {
        if entry.expires_at > now {
            return Some(entry.response.clone());
        }
    }
    map.remove(&key);
    None
}

fn insert_cache(key: u64, response: JudgeResponse) {
    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= JUDGE_CACHE_CAP {
        let now = Instant::now();
        map.retain(|_, v| v.expires_at > now);
        if map.len() >= JUDGE_CACHE_CAP {
            map.clear();
        }
    }
    map.insert(
        key,
        CachedVerdict {
            response,
            expires_at: Instant::now() + JUDGE_CACHE_TTL,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn judge_verdict_serde_round_trip() {
        for v in [JudgeVerdict::Allow, JudgeVerdict::Ask, JudgeVerdict::Deny] {
            let s = serde_json::to_string(&v).unwrap();
            let v2: JudgeVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(v, v2);
        }
    }

    #[test]
    fn parse_response_strips_code_fence() {
        let raw = "```json\n{\"decision\":\"allow\",\"reason\":\"ok\"}\n```";
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.decision, JudgeVerdict::Allow);
        assert_eq!(r.reason, "ok");
    }

    #[test]
    fn parse_response_tolerates_trailing_text() {
        let raw = "Sure: {\"decision\":\"deny\",\"reason\":\"x\"} that's it";
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.decision, JudgeVerdict::Deny);
    }

    #[test]
    fn parse_response_rejects_garbage() {
        assert!(parse_response("nothing json here").is_none());
        assert!(parse_response("} { reversed").is_none());
    }

    #[test]
    fn cache_key_stable_across_calls() {
        let args = json!({"path": "/tmp/x", "n": 1});
        let k1 = cache_key("write", &args, "p1", "m1");
        let k2 = cache_key("write", &args, "p1", "m1");
        assert_eq!(k1, k2);

        // Different args → different key.
        let args2 = json!({"path": "/tmp/y", "n": 1});
        assert_ne!(k1, cache_key("write", &args2, "p1", "m1"));

        // Different model → different key.
        assert_ne!(k1, cache_key("write", &args, "p1", "m2"));
    }

    #[test]
    fn cache_round_trip_within_ttl() {
        let key = u64::MAX - 12345; // unique marker for this test
        let resp = JudgeResponse {
            decision: JudgeVerdict::Allow,
            reason: "test".to_string(),
        };
        insert_cache(key, resp.clone());
        let got = lookup_cache(key).expect("hit");
        assert_eq!(got.decision, resp.decision);
        assert_eq!(got.reason, resp.reason);
    }

    #[test]
    fn cache_evicts_expired_entries_on_overflow() {
        // Fill to cap with already-expired entries; next insert should
        // sweep them rather than blow past the cap.
        let map = cache();
        {
            let mut m = map.lock().unwrap_or_else(|e| e.into_inner());
            m.clear();
            for i in 0..JUDGE_CACHE_CAP {
                m.insert(
                    i as u64,
                    CachedVerdict {
                        response: JudgeResponse {
                            decision: JudgeVerdict::Ask,
                            reason: String::new(),
                        },
                        expires_at: Instant::now() - Duration::from_secs(1),
                    },
                );
            }
        }
        insert_cache(
            999_999,
            JudgeResponse {
                decision: JudgeVerdict::Allow,
                reason: String::new(),
            },
        );
        let m = map.lock().unwrap_or_else(|e| e.into_inner());
        assert!(m.len() < JUDGE_CACHE_CAP);
    }
}
