//! `http` hook handler — POSTs the hook input JSON to a URL and treats the
//! JSON response body as the hook's output (design §7.3).
//!
//! The outbound URL is SSRF-gated through `security::ssrf::check_url` (the
//! shared policy + trusted-host allowlist) before any network touch, and
//! redirects are NOT followed (a redirect would escape that DNS-level check) —
//! new outbound entries must never self-validate IPs (AGENTS.md red line). Any
//! delivered response (regardless of status) maps
//! to exit 0 so the shared parser handles the body — a hook can deny via a
//! non-2xx + decision JSON, and a non-JSON error page parses inert. Only a
//! transport/timeout failure is a non-blocking error.

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

        // SSRF gate FIRST — before constructing the client or touching the
        // network. Uses the shared `Default` policy + the app's trusted-host
        // allowlist, identical to every other outbound dial-out site.
        let trusted = crate::config::cached_config().ssrf.trusted_hosts.clone();
        if let Err(e) = crate::security::ssrf::check_url(
            &self.config.url,
            crate::security::ssrf::SsrfPolicy::Default,
            &trusted,
        )
        .await
        {
            return RawHookResult::non_blocking_error(format!("hook http SSRF blocked: {e}"));
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
        // Static configured headers.
        for (k, v) in &self.config.headers {
            req = req.header(k, v);
        }
        // Forward whitelisted env vars as `X-Hope-Env-<NAME>` headers so the
        // endpoint can read the same context a command hook gets on its env,
        // without leaking the full set.
        for key in &self.config.allowed_env_vars {
            if let Some(val) = env.as_vars().get(key) {
                req = req.header(format!("X-Hope-Env-{key}"), val);
            }
        }

        let resp = match tokio::time::timeout(timeout, req.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                return RawHookResult::non_blocking_error(format!("hook http error: {e}"))
            }
            Err(_) => {
                return RawHookResult {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("hook http timed out after {}s", timeout.as_secs()),
                    duration: start.elapsed(),
                    timed_out: true,
                }
            }
        };

        let status = resp.status();
        let text = match resp.text().await {
            Ok(t) => crate::truncate_utf8(&t, MAX_RESPONSE_BYTES).to_string(),
            Err(e) => {
                return RawHookResult::non_blocking_error(format!("read hook http body: {e}"))
            }
        };

        // A response was received → exit 0 so the shared parser handles the
        // body REGARDLESS of status, letting a hook deny via a non-2xx +
        // decision JSON (a 5xx error page is non-JSON → parsed inert, which is
        // safe). Transport / timeout failures are the non-blocking errors
        // (handled above); a delivered HTTP error must not silently fail open.
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

    /// A private-IP target is rejected by the SSRF gate before any network
    /// touch (literal IP → classified directly, no DNS), and surfaces as a
    /// non-blocking error rather than dialing out.
    #[tokio::test]
    async fn ssrf_blocks_private_target() {
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
        assert_eq!(r.exit_code, Some(1));
        assert!(
            r.stderr.contains("SSRF"),
            "expected SSRF block, got {:?}",
            r.stderr
        );
        assert!(!r.timed_out);
    }
}
