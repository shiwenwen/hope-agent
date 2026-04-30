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
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::mode::JudgeModelConfig;
use crate::agent::AssistantAgent;
use crate::ttl_cache::TtlCache;

/// Hard timeout for the judge model side query. The chat loop blocks on
/// this — if the judge is slow we'd rather fall back than stall the user.
const JUDGE_TIMEOUT: Duration = Duration::from_secs(5);

const JUDGE_CACHE_TTL: Duration = Duration::from_secs(60);

/// Soft cap. Tool loops retrying with mutated args produce fresh keys, so
/// a small bounded cache (cleared on overflow) is plenty.
const JUDGE_CACHE_CAP: usize = 256;

/// Headroom over the ~50-token expected JSON to accommodate reasoning
/// models that emit hidden scratch text before the answer.
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
    if let Some(cached) = cache().get(&key, JUDGE_CACHE_TTL) {
        return Some(cached);
    }

    let app_cfg = crate::config::cached_config();
    let provider_cfg = crate::provider::find_provider(&app_cfg.providers, &config.provider_id)?;

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
    cache().put(key, parsed.clone());
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
/// Uses the shared bracket-balanced extractor so braces inside string
/// literals (e.g. `"reason": "see {note}"`) don't fool the parser.
fn parse_response(text: &str) -> Option<JudgeResponse> {
    let json_part = crate::extract_json_span(text, Some('{'))?;
    serde_json::from_str(json_part).ok()
}

// ── Cache ───────────────────────────────────────────────────────────

fn cache() -> &'static TtlCache<u64, JudgeResponse> {
    static CACHE: OnceLock<TtlCache<u64, JudgeResponse>> = OnceLock::new();
    CACHE.get_or_init(|| TtlCache::new(JUDGE_CACHE_CAP))
}

fn cache_key(tool_name: &str, args: &Value, provider_id: &str, model: &str) -> u64 {
    let mut h = DefaultHasher::new();
    tool_name.hash(&mut h);
    hash_value_canonical(args, &mut h);
    provider_id.hash(&mut h);
    model.hash(&mut h);
    h.finish()
}

/// Hash a `serde_json::Value` so that semantically equal JSON produces the
/// same hash regardless of object key order. Models often emit the same
/// args with different key ordering across tool calls; without this the
/// cache would miss and we'd burn extra ~5s judge LLM calls.
///
/// Tag bytes (0..=5) per variant prevent cross-variant collisions
/// (e.g. `null` vs the empty string `""`).
fn hash_value_canonical(v: &Value, h: &mut DefaultHasher) {
    match v {
        Value::Null => 0u8.hash(h),
        Value::Bool(b) => {
            1u8.hash(h);
            b.hash(h);
        }
        Value::Number(n) => {
            2u8.hash(h);
            // Number isn't Hash directly; canonical decimal repr is stable
            // for any value reachable from JSON parsing.
            n.to_string().hash(h);
        }
        Value::String(s) => {
            3u8.hash(h);
            s.hash(h);
        }
        Value::Array(arr) => {
            4u8.hash(h);
            (arr.len() as u64).hash(h);
            for item in arr {
                hash_value_canonical(item, h);
            }
        }
        Value::Object(map) => {
            5u8.hash(h);
            (map.len() as u64).hash(h);
            // Sort by key for canonical order. serde_json::Map (BTreeMap-
            // backed only when the `preserve_order` feature is off) may
            // already iterate in sorted order, but we don't rely on that.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                k.hash(h);
                hash_value_canonical(&map[k], h);
            }
        }
    }
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
    fn parse_response_handles_braces_in_string_literal() {
        // Naive find('{')/rfind('}') would mis-extract; the shared
        // bracket-balanced helper tracks string state correctly.
        let raw = r#"{"decision":"deny","reason":"contains } literal"}"#;
        let r = parse_response(raw).expect("parse");
        assert_eq!(r.decision, JudgeVerdict::Deny);
        assert!(r.reason.contains("} literal"));
    }

    #[test]
    fn parse_response_rejects_garbage() {
        assert!(parse_response("nothing json here").is_none());
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
    fn cache_key_canonical_object_key_order() {
        // Two semantically identical objects with different key order
        // must hash to the same key, otherwise the LRU cache misses on
        // every retry where the model emits keys in a new order.
        // Build via raw JSON parsing so insertion order is preserved
        // (json! macro normalizes via BTreeMap regardless of input).
        let a: Value = serde_json::from_str(r#"{"path":"/tmp/x","cwd":"/repo","n":1}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"cwd":"/repo","n":1,"path":"/tmp/x"}"#).unwrap();
        let c: Value = serde_json::from_str(r#"{"n":1,"path":"/tmp/x","cwd":"/repo"}"#).unwrap();
        let ka = cache_key("write", &a, "p1", "m1");
        let kb = cache_key("write", &b, "p1", "m1");
        let kc = cache_key("write", &c, "p1", "m1");
        assert_eq!(ka, kb);
        assert_eq!(ka, kc);
    }

    #[test]
    fn cache_key_canonical_nested_objects() {
        // Recursive: nested objects must also be canonical.
        let a: Value = serde_json::from_str(r#"{"o":{"a":1,"b":2},"k":"v"}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"k":"v","o":{"b":2,"a":1}}"#).unwrap();
        assert_eq!(cache_key("t", &a, "p", "m"), cache_key("t", &b, "p", "m"));
    }

    #[test]
    fn cache_key_canonical_distinguishes_distinct_values() {
        // Tag bytes prevent cross-variant collisions.
        let null = Value::Null;
        let empty_str = Value::String(String::new());
        let empty_arr = Value::Array(vec![]);
        let empty_obj: Value = serde_json::from_str("{}").unwrap();
        let k_null = cache_key("t", &null, "p", "m");
        let k_str = cache_key("t", &empty_str, "p", "m");
        let k_arr = cache_key("t", &empty_arr, "p", "m");
        let k_obj = cache_key("t", &empty_obj, "p", "m");
        assert_ne!(k_null, k_str);
        assert_ne!(k_null, k_arr);
        assert_ne!(k_null, k_obj);
        assert_ne!(k_str, k_arr);
        assert_ne!(k_str, k_obj);
        assert_ne!(k_arr, k_obj);
    }

    #[test]
    fn cache_round_trip_within_ttl() {
        // Use a key unlikely to collide with concurrent tests since the
        // underlying TtlCache is process-global.
        let key = u64::MAX - 12345;
        let resp = JudgeResponse {
            decision: JudgeVerdict::Allow,
            reason: "test".to_string(),
        };
        cache().put(key, resp.clone());
        let got = cache().get(&key, JUDGE_CACHE_TTL).expect("hit");
        assert_eq!(got.decision, resp.decision);
        assert_eq!(got.reason, resp.reason);
    }
}
