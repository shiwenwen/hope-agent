//! `http` hook handler — POSTs the hook input JSON to a URL and treats the
//! JSON response body as the hook's output (design §7.3).
//!
//! The outbound URL is SSRF-gated through `security::ssrf::check_url` (the
//! shared policy + trusted-host allowlist) before any network touch, and
//! redirects are NOT followed (a redirect would escape that DNS-level check) —
//! new outbound entries must never self-validate IPs (AGENTS.md red line).
//!
//! ## Fail-closed on blocking events
//!
//! For events that GATE execution (`PreToolUse` / `UserPromptSubmit` /
//! `PreCompact`), every degraded delivery path is mapped to `exit 2`
//! (`Block`) so the gate fails closed. Specifically: SSRF refusal, transport
//! errors, request timeouts, body-read errors, non-2xx HTTP status, AND
//! 2xx responses whose body isn't valid JSON. Without this, a 401 HTML page
//! from an auth-expired webhook or a 502 from a reverse proxy parses as an
//! inert outcome — a silent fail-open precisely on the security path the
//! hook exists to enforce. Observation-only events keep their non-blocking
//! degraded paths (transport/timeout = inert) since they can't gate anyway
//! and fail-closing them would just hide the real call's outcome.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::super::config::HttpHookConfig;
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};

/// Default http-hook timeout (design §7.3).
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;
/// Response body capture cap (§7.9).
const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1 MiB

/// Whether this hook input fires on a gate-capable event whose degraded
/// delivery paths must fail closed. The list mirrors the events that flow
/// through a blocking decision sink (PreToolUse gate, UserPromptSubmit
/// preflight, PreCompact). Observation-only events are excluded — `Block`
/// from them is downgraded by `is_observation_only` anyway, and fail-closing
/// them would hide real errors without buying any security.
fn is_blocking_hook_input(input: &HookInput) -> bool {
    matches!(
        input,
        HookInput::PreToolUse { .. }
            | HookInput::UserPromptSubmit { .. }
            | HookInput::PreCompact { .. }
    )
}

/// Build a `Block`-mapped `RawHookResult` (`exit 2` → parser produces
/// `HookDecision::Block { reason: stderr }`) for a degraded HTTP delivery on
/// a blocking event. Used by every fail-closed branch so the audit trail
/// stays uniform.
fn fail_closed_block(stderr: String, start: Instant) -> RawHookResult {
    RawHookResult {
        exit_code: Some(2),
        stdout: String::new(),
        stderr,
        duration: start.elapsed(),
        timed_out: false,
    }
}

/// Resolve the value for each name in the `allowed_env_vars` whitelist.
/// Lookup order: synthesized [`HookEnv`] map (HOPE / CLAUDE / PATH) first,
/// host process env second. Names that resolve to nothing are dropped; the
/// caller's placeholder expansion will report them as unresolved. A
/// `BTreeMap` is used so the resulting `X-Hope-Env-*` headers come out in a
/// stable order — useful for tests and signature-based webhooks.
fn resolve_allowed_env(env: &HookEnv, allowed: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for key in allowed {
        let val = env
            .as_vars()
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok());
        if let Some(v) = val {
            out.insert(key.clone(), v);
        }
    }
    out
}

/// Expand `$VAR` and `${VAR}` placeholders in `value` against `env_map`.
/// Returns the expanded string and the list of placeholder names that didn't
/// have a value (i.e. the name wasn't in the whitelist OR it was but had no
/// value in either env source). Unknown placeholders are left literal so a
/// malformed config doesn't accidentally leak the empty string into an
/// `Authorization` header (which would silently produce a 401 rather than
/// surfacing the misconfig).
fn expand_env_placeholders(
    value: &str,
    env_map: &BTreeMap<String, String>,
) -> (String, Vec<String>) {
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut unresolved: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            // Push the raw byte. Multi-byte UTF-8 is fine because we only
            // branch on the `$` ASCII byte; other bytes pass through verbatim.
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // `$` at the very end → literal.
        if i + 1 >= bytes.len() {
            out.push('$');
            i += 1;
            continue;
        }
        if bytes[i + 1] == b'{' {
            // `${VAR}` form. Find the closing `}` after the `{`.
            if let Some(close_rel) = bytes[i + 2..].iter().position(|b| *b == b'}') {
                let name_start = i + 2;
                let name_end = name_start + close_rel;
                let name = &value[name_start..name_end];
                if name.is_empty() {
                    // `${}` is literal — there's no useful expansion.
                    out.push_str("${}");
                } else if let Some(v) = env_map.get(name) {
                    out.push_str(v);
                } else {
                    // Unknown / not-whitelisted name → leave literal AND record
                    // so the caller can warn.
                    out.push_str(&value[i..=name_end]);
                    unresolved.push(name.to_string());
                }
                i = name_end + 1;
                continue;
            }
            // No closing `}` → treat the rest as literal.
            out.push_str(&value[i..]);
            break;
        }
        // `$VAR` form — name is `[A-Za-z_][A-Za-z0-9_]*` (POSIX-like; restrictive
        // on purpose so we don't gobble valid trailing punctuation in headers).
        let name_start = i + 1;
        let mut name_end = name_start;
        if bytes[name_end].is_ascii_alphabetic() || bytes[name_end] == b'_' {
            name_end += 1;
            while name_end < bytes.len()
                && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
            {
                name_end += 1;
            }
            let name = &value[name_start..name_end];
            if let Some(v) = env_map.get(name) {
                out.push_str(v);
            } else {
                out.push_str(&value[i..name_end]);
                unresolved.push(name.to_string());
            }
            i = name_end;
            continue;
        }
        // `$` followed by something that can't start an identifier → literal.
        out.push('$');
        i += 1;
    }
    (out, unresolved)
}

pub struct HttpHandler {
    config: HttpHookConfig,
}

impl HttpHandler {
    pub fn new(config: HttpHookConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl HookHandler for HttpHandler {
    fn identity(&self) -> String {
        format!("{}|timeout={:?}", self.config.url, self.config.timeout)
    }

    fn handler_type(&self) -> &'static str {
        "http"
    }

    fn default_timeout(&self) -> Duration {
        Duration::from_secs(self.config.timeout.unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS))
    }

    async fn run(&self, input: &HookInput, env: &HookEnv, deadline: Instant) -> RawHookResult {
        let start = Instant::now();
        let is_blocking = is_blocking_hook_input(input);

        // SSRF gate FIRST — before constructing the client or touching the
        // network. Uses the shared `Default` policy + the app's trusted-host
        // allowlist, identical to every other outbound dial-out site. On a
        // blocking event an SSRF refusal means we can't reach the policy
        // endpoint at all → fail closed; on observation events keep the
        // non-blocking error (audit-only).
        let trusted = crate::config::cached_config().ssrf.trusted_hosts.clone();
        if let Err(e) = crate::security::ssrf::check_url(
            &self.config.url,
            crate::security::ssrf::SsrfPolicy::Default,
            &trusted,
        )
        .await
        {
            let msg = format!("hook http SSRF blocked: {e}");
            if is_blocking {
                crate::app_warn!(
                    "hooks",
                    "http",
                    "blocking event fail-closed (SSRF): {} → {}",
                    self.config.url,
                    e
                );
                return fail_closed_block(msg, start);
            }
            return RawHookResult::non_blocking_error(msg);
        }

        let body = match serde_json::to_vec(input) {
            Ok(b) => b,
            Err(e) => {
                return RawHookResult::non_blocking_error(format!("serialize hook input: {e}"))
            }
        };

        // Remaining budget. The SSRF check above did DNS, which can eat the
        // deadline — floor to 1s so a slow lookup doesn't collapse the request
        // to an instant 0-duration timeout that never dials.
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1));
        // Do NOT follow redirects. `check_url` above only SSRF-validated the
        // initial URL with a DNS resolve; a redirect would be followed by
        // reqwest with only the sync host check (which can't resolve a hostname
        // and so lets an unknown name through), letting a public endpoint 3xx
        // to a name that resolves to a metadata/private IP. A hook endpoint is
        // a configured webhook — it should be a stable canonical URL — so the
        // safe posture is no redirects at all (a 3xx body just parses inert).
        let builder = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none());
        // Honor the app proxy policy (matches every other outbound site).
        let client = match crate::provider::apply_proxy(builder).build() {
            Ok(c) => c,
            Err(e) => return RawHookResult::non_blocking_error(format!("build http client: {e}")),
        };

        // Resolve the allow-listed env values once: prefer the synthesized
        // hook env (HOPE_*, CLAUDE_*, PATH) where it overrides, then fall
        // back to the host process env so a user-listed `MY_API_TOKEN` is
        // actually readable. Vars not in the whitelist are never resolved.
        let env_map = resolve_allowed_env(env, &self.config.allowed_env_vars);

        let mut req = client.post(&self.config.url).body(body);
        // Default content-type only when the user didn't configure one (reqwest
        // `.header()` appends, so a configured content-type would otherwise be
        // sent twice).
        if !self
            .config
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("content-type"))
        {
            req = req.header("content-type", "application/json");
        }
        // Configured headers — expand `$VAR` / `${VAR}` placeholders against
        // the whitelist so an `Authorization: Bearer $TOKEN` value (common for
        // PreToolUse webhooks behind auth) reaches the endpoint as the real
        // token, not the literal placeholder. References outside the whitelist
        // remain literal AND are surfaced as a warn so the hook author notices
        // the typo / missing entry rather than the blocking endpoint silently
        // returning 401 → parsed-inert → fail-open.
        for (k, v) in &self.config.headers {
            let (expanded, unresolved) = expand_env_placeholders(v, &env_map);
            if !unresolved.is_empty() {
                crate::app_warn!(
                    "hooks",
                    "http",
                    "HTTP hook header '{}' has unresolved placeholder(s) {:?}; allowedEnvVars whitelist must list each VAR before its value can be substituted",
                    k,
                    unresolved
                );
            }
            req = req.header(k, expanded);
        }
        // Forward whitelisted env vars as `X-Hope-Env-<NAME>` headers so the
        // endpoint can read the same context a command hook gets on its env,
        // without leaking the full set.
        for (key, val) in &env_map {
            req = req.header(format!("X-Hope-Env-{key}"), val);
        }

        let resp = match tokio::time::timeout(timeout, req.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let msg = format!("hook http error: {e}");
                if is_blocking {
                    crate::app_warn!(
                        "hooks",
                        "http",
                        "blocking event fail-closed (transport): {} → {}",
                        self.config.url,
                        e
                    );
                    return fail_closed_block(msg, start);
                }
                return RawHookResult::non_blocking_error(msg);
            }
            Err(_) => {
                let msg = format!("hook http timed out after {}s", timeout.as_secs());
                if is_blocking {
                    crate::app_warn!(
                        "hooks",
                        "http",
                        "blocking event fail-closed (timeout): {} after {}s",
                        self.config.url,
                        timeout.as_secs()
                    );
                    return RawHookResult {
                        // Set BOTH `exit_code=Some(2)` (so the parser produces
                        // `Block`) AND `timed_out=true` (so the audit log
                        // surfaces the cause). `timed_out` short-circuits to
                        // inert in `parse()`, so the explicit exit-2 mapping
                        // happens by branching there — see fail_closed_block
                        // comments; here we don't take that path because the
                        // parser already handles `timed_out=true` as inert.
                        exit_code: Some(2),
                        stdout: String::new(),
                        stderr: msg,
                        duration: start.elapsed(),
                        // Deliberately false: we want the parser to honor the
                        // explicit exit 2 mapping → Block. The audit record
                        // still carries the deadline-exceeded reason via the
                        // stderr string.
                        timed_out: false,
                    };
                }
                return RawHookResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: msg,
                    duration: start.elapsed(),
                    timed_out: true,
                };
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => crate::truncate_utf8(&t, MAX_RESPONSE_BYTES).to_string(),
            Err(e) => {
                let msg = format!("read hook http body: {e}");
                if is_blocking {
                    crate::app_warn!(
                        "hooks",
                        "http",
                        "blocking event fail-closed (body-read): {} → {}",
                        self.config.url,
                        e
                    );
                    return fail_closed_block(msg, start);
                }
                return RawHookResult::non_blocking_error(msg);
            }
        };

        // Blocking events demand a parseable JSON verdict on a 2xx response.
        // A 401 HTML page, a 502 from a reverse proxy, or a 200 with the wrong
        // content-type are all "the policy didn't render a verdict for us" —
        // the safe default for a security gate is to refuse the action, not
        // to silently let it through because the parser falls back to inert.
        // Observation events keep the legacy lenient parse: their decisions
        // are downgraded by `is_observation_only` anyway, so fail-closing them
        // would just hide the real call.
        if is_blocking {
            if !status.is_success() {
                let msg = format!(
                    "hook http returned non-success status {} on blocking event — failing closed",
                    status.as_u16()
                );
                crate::app_warn!(
                    "hooks",
                    "http",
                    "blocking event fail-closed (status {}): {}",
                    status.as_u16(),
                    self.config.url
                );
                return fail_closed_block(msg, start);
            }
            // 2xx — must be valid JSON. An empty body is treated as "no
            // verdict" → fail closed; the parser's plaintext fallback only
            // applies to a couple of observation events and doesn't help a
            // gate decide.
            let trimmed = text.trim();
            if trimmed.is_empty() || serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
                let msg = "hook http returned non-JSON body on blocking event — failing closed"
                    .to_string();
                crate::app_warn!(
                    "hooks",
                    "http",
                    "blocking event fail-closed (non-JSON body): {}",
                    self.config.url
                );
                return fail_closed_block(msg, start);
            }
        }

        // A response was received → exit 0 so the shared parser handles the
        // body. For OBSERVATION events: any status maps to exit 0 (a 5xx HTML
        // page is non-JSON → parsed inert, which is safe — there's no gate to
        // fail closed). For BLOCKING events we've already filtered out
        // non-2xx and non-JSON above, so reaching here means a valid JSON
        // verdict the parser can act on.
        RawHookResult {
            exit_code: Some(0),
            stdout: text,
            stderr: if status.is_success() {
                String::new()
            } else {
                format!("http status {}", status.as_u16())
            },
            duration: start.elapsed(),
            timed_out: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{CommonHookInput, PermissionMode};
    use std::path::PathBuf;

    fn dummy_input() -> HookInput {
        HookInput::PreToolUse {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/tmp"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "PreToolUse".into(),
                agent_id: None,
                agent_type: None,
            },
            tool_name: "exec".into(),
            tool_input: serde_json::json!({}),
            tool_use_id: "c1".into(),
        }
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn expand_bracketed_variable() {
        let map = env(&[("TOKEN", "abc123")]);
        let (out, unresolved) = expand_env_placeholders("Bearer ${TOKEN}", &map);
        assert_eq!(out, "Bearer abc123");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn expand_dollar_variable() {
        let map = env(&[("API_KEY", "xyz")]);
        let (out, unresolved) = expand_env_placeholders("X-Key: $API_KEY!", &map);
        // Trailing `!` is not a name char so the variable terminates cleanly.
        assert_eq!(out, "X-Key: xyz!");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn unknown_variable_stays_literal_and_is_reported() {
        // The whitelist resolves zero values for `MISSING`; the placeholder
        // stays in the output (so the endpoint sees something obviously wrong
        // rather than a silent empty Authorization) and we report it.
        let map = env(&[("OTHER", "v")]);
        let (out, unresolved) =
            expand_env_placeholders("Bearer ${MISSING} suffix $OTHER $ALSO_MISSING", &map);
        assert_eq!(out, "Bearer ${MISSING} suffix v $ALSO_MISSING");
        assert_eq!(unresolved, vec!["MISSING", "ALSO_MISSING"]);
    }

    #[test]
    fn unterminated_brace_stays_literal() {
        let map = env(&[("X", "ok")]);
        let (out, unresolved) = expand_env_placeholders("prefix ${UNCLOSED", &map);
        assert_eq!(out, "prefix ${UNCLOSED");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn lone_dollar_or_invalid_name_passes_through() {
        let map = env(&[("X", "v")]);
        // `$1` isn't a POSIX-style name; treat as literal. `$` at EOL too.
        let (out, _u) = expand_env_placeholders("cost is $5 total: $", &map);
        assert_eq!(out, "cost is $5 total: $");
    }

    #[test]
    fn empty_brace_is_literal() {
        let map = env(&[]);
        let (out, unresolved) = expand_env_placeholders("a${}b", &map);
        assert_eq!(out, "a${}b");
        // No name to report — `${}` collapses to literal without naming a var.
        assert!(unresolved.is_empty());
    }

    #[test]
    fn resolve_prefers_hook_env_then_process_env() {
        // `HOPE_SESSION_ID` lives in the synthesized HookEnv; user-supplied
        // vars (like a real API token) come from the host process env.
        let common = CommonHookInput {
            session_id: "sess-xyz".into(),
            transcript_path: PathBuf::from("/tmp/t.jsonl"),
            cwd: std::env::temp_dir(),
            permission_mode: PermissionMode::Default,
            hook_event_name: "PreToolUse".into(),
            agent_id: None,
            agent_type: None,
        };
        let env = HookEnv::build_for_command(&common);
        // Unique name to avoid colliding with any real env in CI.
        let key = "HA_TEST_HTTP_HOOK_TOKEN_C3";
        std::env::set_var(key, "real-secret");
        let resolved = resolve_allowed_env(
            &env,
            &[
                "HOPE_SESSION_ID".to_string(),
                key.to_string(),
                "DEFINITELY_MISSING_VAR_XYZ".to_string(),
            ],
        );
        std::env::remove_var(key);
        assert_eq!(
            resolved.get("HOPE_SESSION_ID").map(String::as_str),
            Some("sess-xyz")
        );
        assert_eq!(resolved.get(key).map(String::as_str), Some("real-secret"));
        // Missing var is dropped entirely, not stored as empty.
        assert!(!resolved.contains_key("DEFINITELY_MISSING_VAR_XYZ"));
    }

    fn observation_input() -> HookInput {
        // Notification is observation-only; SSRF refusal stays as
        // non-blocking error rather than fail-closed Block.
        HookInput::Notification {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/tmp"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "Notification".into(),
                agent_id: None,
                agent_type: None,
            },
            notification_type: "idle_prompt".into(),
            message: "hi".into(),
            title: None,
        }
    }

    #[test]
    fn blocking_event_classifier_covers_the_gate_set() {
        // PreToolUse / UserPromptSubmit / PreCompact gate execution, so
        // their degraded HTTP paths must fail closed. Everything else is
        // observation and must keep the lenient (inert) behavior.
        let pre_tool = dummy_input();
        assert!(is_blocking_hook_input(&pre_tool));
        let user_prompt = HookInput::UserPromptSubmit {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/tmp"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "UserPromptSubmit".into(),
                agent_id: None,
                agent_type: None,
            },
            prompt: "x".into(),
        };
        assert!(is_blocking_hook_input(&user_prompt));
        let pre_compact = HookInput::PreCompact {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/tmp"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "PreCompact".into(),
                agent_id: None,
                agent_type: None,
            },
            trigger: crate::hooks::types::CompactTrigger::Auto,
            usage_ratio: 0.5,
        };
        assert!(is_blocking_hook_input(&pre_compact));
        // Notification is observation — keep inert path on degradation.
        assert!(!is_blocking_hook_input(&observation_input()));
    }

    /// On a blocking event, an SSRF refusal short-circuits to fail-closed
    /// `exit 2`. The parser maps that to `HookDecision::Block { reason: stderr }`
    /// so the gate stops the tool rather than silently letting it run because
    /// the policy endpoint was unreachable.
    #[tokio::test]
    async fn ssrf_blocks_private_target_fail_closed_on_blocking_event() {
        let h = HttpHandler::new(HttpHookConfig {
            url: "http://10.0.0.1/hook".into(),
            timeout: Some(5),
            headers: Default::default(),
            allowed_env_vars: vec![],
            status_message: None,
            if_rule: None,
            once: None,
        });
        let r = h
            .run(
                &dummy_input(),
                &HookEnv::empty(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert_eq!(r.exit_code, Some(2), "blocking event must fail closed");
        assert!(
            r.stderr.contains("SSRF"),
            "expected SSRF block, got {:?}",
            r.stderr
        );
        assert!(!r.timed_out);
    }

    /// SSRF refusal on an observation event keeps the legacy non-blocking
    /// error — the call would have been observation-only anyway, so there's
    /// no gate to fail closed.
    #[tokio::test]
    async fn ssrf_blocks_private_target_inert_on_observation_event() {
        let h = HttpHandler::new(HttpHookConfig {
            url: "http://10.0.0.1/hook".into(),
            timeout: Some(5),
            headers: Default::default(),
            allowed_env_vars: vec![],
            status_message: None,
            if_rule: None,
            once: None,
        });
        let r = h
            .run(
                &observation_input(),
                &HookEnv::empty(),
                Instant::now() + Duration::from_secs(5),
            )
            .await;
        assert_eq!(r.exit_code, Some(1), "observation event stays non-blocking");
        assert!(r.stderr.contains("SSRF"));
    }
}
