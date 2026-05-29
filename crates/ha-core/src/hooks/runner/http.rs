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

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::super::config::HttpHookConfig;
use super::super::env::HookEnv;
use super::super::types::HookInput;
use super::{HookHandler, RawHookResult};

/// Default http-hook timeout (design §7.3).
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;
/// Response body capture cap (§7.9). Enforced INCREMENTALLY by
/// [`read_body_bounded`] — the cap is a memory ceiling, not a post-hoc
/// truncation. An endpoint that streams a multi-GB body is hung up at 1 MiB.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1 MiB

/// Top-level keys that count as recognized hook-protocol fields on a 2xx body.
/// An object whose keys are ALL outside this set (`{"error":"unauthorized"}`,
/// `{"message":"ok"}` from a generic JSON endpoint, etc.) is rejected on a
/// blocking event — silent-fall-through to `Allow` would defeat the gate.
/// Camel-case to match the wire form; `HookOutput` field renames decide what
/// the parser actually consumes downstream.
const RECOGNIZED_PROTOCOL_KEYS: &[&str] = &[
    "continue",
    "stopReason",
    "suppressOutput",
    "systemMessage",
    "decision",
    "reason",
    "hookSpecificOutput",
];

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

/// A stable fingerprint of the per-request semantics that aren't already in
/// the URL/timeout. Hashing headers + the env-var whitelist disambiguates two
/// HTTP hooks pointing at the same URL but carrying different
/// Authorization/tenant headers or sourcing different env keys — without it
/// the `(handler_type, identity)` dedup in `dispatch_with` silently folds
/// them and only the first config actually runs (a UNION-scope user / managed
/// double-policy footgun). Identity-only call site for now, so SipHash via
/// `DefaultHasher` is fine — collisions require two semantically distinct
/// configs to coincidentally produce the same 64-bit digest, vanishingly
/// rare for short JSON-ish material.
fn http_identity_hash(headers: &HashMap<String, String>, allowed_env_vars: &[String]) -> String {
    let mut h = DefaultHasher::new();
    // HashMap iteration order is non-deterministic — sort by key for a stable
    // identity. Mixing in both key + value catches `Authorization: X` vs
    // `Authorization: Y` on the same URL.
    let mut sorted_headers: Vec<(&String, &String)> = headers.iter().collect();
    sorted_headers.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in sorted_headers {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    // `allowed_env_vars` is a Vec but we don't want order to matter here —
    // `[A, B]` and `[B, A]` describe the same forwarding set.
    let mut sorted_env: Vec<&String> = allowed_env_vars.iter().collect();
    sorted_env.sort();
    for k in sorted_env {
        k.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Whether a parsed 2xx body looks like a valid hook-protocol response on a
/// blocking event. An empty object (`{}`) is valid — that's the documented
/// "no opinion / silent allow" shape; the parser folds it to `Allow`. Any
/// non-object value, or an object whose keys are ALL outside
/// [`RECOGNIZED_PROTOCOL_KEYS`], is rejected so a generic error envelope
/// (`{"error":"..."}`, `{"message":"ok"}`, `[]`, `"ok"`, …) can't sneak
/// through as inert → fail-open.
fn looks_like_hook_protocol_response(value: &serde_json::Value) -> bool {
    let serde_json::Value::Object(map) = value else {
        return false;
    };
    if map.is_empty() {
        return true; // `{}` → silent allow, parser will resolve to Allow
    }
    map.keys()
        .any(|k| RECOGNIZED_PROTOCOL_KEYS.contains(&k.as_str()))
}

/// Read the response body chunk-by-chunk, stopping AT `max_bytes` rather than
/// after. Returns `(text, truncated)` where `truncated=true` means at least
/// one byte was discarded — the caller decides whether to fail-closed (on a
/// blocking event) or accept the partial body (observation). Replaces
/// `resp.text().await`, which buffered the entire response before applying
/// the cap — a multi-GB endpoint would OOM the process before truncation
/// fired. Adversarial-review HIGH.
async fn read_body_bounded(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<(String, bool), reqwest::Error> {
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut truncated = false;
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() >= max_bytes {
            truncated = true;
            break;
        }
        let remaining = max_bytes - buf.len();
        if chunk.len() > remaining {
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            // Drop `resp` here — `reqwest::Response` owns the underlying
            // body stream; dropping it terminates the connection so the
            // remote stops sending.
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    // The body may be valid UTF-8 truncated mid-char by our byte cap; the
    // lossy path replaces the trailing partial codepoint with U+FFFD rather
    // than failing — sufficient for the downstream JSON parser which only
    // needs to see the truncated body as invalid JSON to fail closed on a
    // blocking event.
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
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
            // Copy the WHOLE UTF-8 char at `i`, not a single byte. `bytes[i]
            // as char` Latin-1-expands each byte of a multi-byte sequence
            // (`é` = 0xC3 0xA9 → "Ã©", CJK → mojibake), corrupting any
            // non-ASCII header / env value. `i` always lands on a char
            // boundary — every other branch advances past an ASCII delimiter
            // (`$`/`{`/`}`/identifier byte) — so this slice + `chars().next()`
            // never panics.
            let ch = value[i..]
                .chars()
                .next()
                .expect("loop index stays on a UTF-8 char boundary");
            out.push(ch);
            i += ch.len_utf8();
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
        // `(url, timeout)` alone collapses two hooks pointing at the same URL
        // but carrying different headers / env forwarding — see
        // `http_identity_hash` for the rationale. Hash is appended so the
        // existing prefix stays human-readable in audit logs.
        format!(
            "{}|timeout={:?}|{}",
            self.config.url,
            self.config.timeout,
            http_identity_hash(&self.config.headers, &self.config.allowed_env_vars),
        )
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
                        // Fail closed: `exit_code=Some(2)` makes `parse()`
                        // produce `HookDecision::Block { reason: stderr }`.
                        // `timed_out` MUST stay `false` here — `parse()`
                        // short-circuits `timed_out=true` to inert (a silent
                        // fail-OPEN on this gate), so setting it true would
                        // defeat the block. The deadline cause is preserved in
                        // `stderr` for the audit log.
                        exit_code: Some(2),
                        stdout: String::new(),
                        stderr: msg,
                        duration: start.elapsed(),
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
        // Bounded streaming read — caps memory AT the limit rather than
        // after `resp.text()` has already buffered the whole body. A
        // multi-GB endpoint is hung up at MAX_RESPONSE_BYTES.
        let (text, truncated) = match read_body_bounded(resp, MAX_RESPONSE_BYTES).await {
            Ok(t) => t,
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
        // An oversized body is itself a degraded delivery: the endpoint
        // either misconfigured or is hostile. For blocking events that
        // means fail-closed. For observation events we keep the (now
        // truncated) body, but log the truncation so the cap isn't
        // silent — a 5xx error page that happens to exceed the cap
        // still parses inert in the existing path.
        if truncated {
            if is_blocking {
                crate::app_warn!(
                    "hooks",
                    "http",
                    "blocking event fail-closed (body > {} bytes): {}",
                    MAX_RESPONSE_BYTES,
                    self.config.url
                );
                return fail_closed_block(
                    format!(
                        "hook http body exceeded {} bytes on blocking event — failing closed",
                        MAX_RESPONSE_BYTES
                    ),
                    start,
                );
            }
            crate::app_warn!(
                "hooks",
                "http",
                "observation body truncated at {} bytes: {}",
                MAX_RESPONSE_BYTES,
                self.config.url
            );
        }

        // Blocking events demand a parseable, protocol-shaped JSON verdict on a
        // 2xx response. A 401 HTML page, a 502 from a reverse proxy, and a 200
        // with a generic error envelope (`{"error":"unauthorized"}`) are all
        // "the policy didn't render a verdict for us" — the safe default for a
        // security gate is to refuse the action, not to silently let it through
        // because the parser falls back to inert. Observation events keep the
        // legacy lenient parse: their decisions are downgraded by
        // `is_observation_only` anyway, so fail-closing them would just hide
        // the real call.
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
            // 2xx — must look like a hook-protocol response. The shape check
            // splits three failure modes that all used to pass through:
            //   - empty body → no verdict at all → fail-closed
            //   - non-object JSON (`[]`, `"ok"`, `42`, `true`, `null`) →
            //     fail-closed
            //   - object whose keys are ALL outside the recognized set
            //     (`{"error":"unauthorized"}`) → fail-closed
            // `{}` (empty object) is the documented "silent allow" shape and
            // is explicitly accepted by `looks_like_hook_protocol_response`.
            let trimmed = text.trim();
            let valid = (!trimmed.is_empty())
                .then(|| serde_json::from_str::<serde_json::Value>(trimmed).ok())
                .flatten()
                .as_ref()
                .map(looks_like_hook_protocol_response)
                .unwrap_or(false);
            if !valid {
                let reason = if trimmed.is_empty() {
                    "hook http returned empty body on blocking event — failing closed".to_string()
                } else {
                    "hook http returned non-protocol body on blocking event — failing closed"
                        .to_string()
                };
                crate::app_warn!(
                    "hooks",
                    "http",
                    "blocking event fail-closed (non-protocol body): {}",
                    self.config.url
                );
                return fail_closed_block(reason, start);
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
    fn multibyte_utf8_in_literal_text_is_preserved() {
        // Regression: a per-byte `bytes[i] as char` push Latin-1-expanded
        // multi-byte UTF-8 ("café" → "cafÃ©", CJK → mojibake). The literal
        // runs around a placeholder must round-trip non-ASCII verbatim.
        let map = env(&[("TOKEN", "x")]);
        let (out, unresolved) =
            expand_env_placeholders("café ${TOKEN} 文件 — naïve $TOKEN ✓", &map);
        assert_eq!(out, "café x 文件 — naïve x ✓");
        assert!(unresolved.is_empty());
    }

    #[test]
    fn multibyte_utf8_in_resolved_value_is_preserved() {
        // The substituted value can itself be non-ASCII (e.g. an env var
        // holding a UTF-8 secret/path); it must not be corrupted either.
        let map = env(&[("NAME", "naïve-café-文件")]);
        let (out, _u) = expand_env_placeholders("X-Name: ${NAME}", &map);
        assert_eq!(out, "X-Name: naïve-café-文件");
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

    // ── Protocol-shape validation (E2) ──────────────────────────────────

    #[test]
    fn empty_object_is_valid_silent_allow() {
        // `{}` is the documented "no opinion" response — the parser folds
        // it to Allow. Fail-closing it would break every silent-observe
        // webhook.
        let v: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert!(looks_like_hook_protocol_response(&v));
    }

    #[test]
    fn object_with_recognized_key_is_valid() {
        for key in RECOGNIZED_PROTOCOL_KEYS {
            let json = format!(r#"{{"{}": null}}"#, key);
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(
                looks_like_hook_protocol_response(&v),
                "{} should be recognized",
                key
            );
        }
    }

    #[test]
    fn generic_error_envelope_is_rejected() {
        // A 200 returning `{"error":"..."}` from a generic JSON endpoint
        // currently slips through as inert → Allow. That's the most
        // dangerous fail-open shape — auth-expired webhooks routinely
        // return this.
        let cases = [
            r#"{"error":"unauthorized"}"#,
            r#"{"message":"ok"}"#,
            r#"{"status":"failed"}"#,
            r#"{"random":"thing"}"#,
        ];
        for c in cases {
            let v: serde_json::Value = serde_json::from_str(c).unwrap();
            assert!(
                !looks_like_hook_protocol_response(&v),
                "{} should be rejected",
                c
            );
        }
    }

    #[test]
    fn non_object_json_is_rejected() {
        for c in [
            "[]",
            r#""ok""#,
            "42",
            "true",
            "null",
            "[{\"decision\":\"allow\"}]", // wrapping a verdict in an array doesn't make it a verdict
        ] {
            let v: serde_json::Value = serde_json::from_str(c).unwrap();
            assert!(
                !looks_like_hook_protocol_response(&v),
                "{} should be rejected",
                c
            );
        }
    }

    #[test]
    fn mixed_object_with_one_recognized_key_is_valid() {
        // Real-world responses often carry extra metadata alongside a verdict
        // (`{"decision":"allow","trace_id":"..."}`). One recognized key is
        // enough; unknown siblings don't trip the shape check.
        let v: serde_json::Value =
            serde_json::from_str(r#"{"decision":"allow","trace_id":"abc"}"#).unwrap();
        assert!(looks_like_hook_protocol_response(&v));
    }

    // ── Identity dedup (E3) ─────────────────────────────────────────────

    fn http_config(headers: &[(&str, &str)], env_vars: &[&str]) -> HttpHookConfig {
        let mut h = HashMap::new();
        for (k, v) in headers {
            h.insert((*k).to_string(), (*v).to_string());
        }
        HttpHookConfig {
            url: "https://example.com/hook".into(),
            timeout: Some(10),
            headers: h,
            allowed_env_vars: env_vars.iter().map(|s| (*s).to_string()).collect(),
            status_message: None,
            if_rule: None,
            once: None,
        }
    }

    #[test]
    fn identity_disambiguates_different_authorization_headers() {
        // Two hooks pointing at the same URL with different bearer tokens
        // (multi-tenant webhook, or managed + user double-policy) used to
        // collide on `(url, timeout)` and the dispatch dedup silently kept
        // only one. Hash now folds headers in.
        let a = HttpHandler::new(http_config(&[("Authorization", "Bearer aaa")], &[]));
        let b = HttpHandler::new(http_config(&[("Authorization", "Bearer bbb")], &[]));
        assert_ne!(a.identity(), b.identity());

        // Same auth header → same identity (legitimate dedup).
        let c = HttpHandler::new(http_config(&[("Authorization", "Bearer aaa")], &[]));
        assert_eq!(a.identity(), c.identity());
    }

    #[test]
    fn identity_disambiguates_different_allowed_env_vars() {
        // Two hooks with different env-forwarding whitelists send different
        // `X-Hope-Env-*` headers — they're semantically distinct.
        let a = HttpHandler::new(http_config(&[], &["TOKEN_A"]));
        let b = HttpHandler::new(http_config(&[], &["TOKEN_B"]));
        assert_ne!(a.identity(), b.identity());
    }

    #[test]
    fn identity_ignores_insertion_order_of_headers_and_env() {
        // HashMap iteration order is non-deterministic; identity must be
        // order-independent so two equivalent configs in different scopes
        // (user + managed reading the same `hooks.json`) genuinely dedup.
        let a = HttpHandler::new(http_config(&[("A", "1"), ("B", "2")], &["X", "Y"]));
        let b = HttpHandler::new(http_config(&[("B", "2"), ("A", "1")], &["Y", "X"]));
        assert_eq!(a.identity(), b.identity());
    }

    // ── Bounded body capture (E1) ────────────────────────────────────────

    #[test]
    fn looks_like_hook_protocol_response_accepts_object_with_continue_field() {
        // The `continue:false` shape carries no `decision` but still drives
        // a Block via the aggregator — it's a recognized protocol key.
        let v: serde_json::Value =
            serde_json::from_str(r#"{"continue":false,"stopReason":"halt"}"#).unwrap();
        assert!(looks_like_hook_protocol_response(&v));
    }
}
