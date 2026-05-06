//! REST 层 429 / Retry-After 统一处理 helper。
//!
//! 各 IM 平台 REST API 都按各自规则限流（Discord global + per-bucket、Slack
//! tier-based、LINE 月度配额等），但都会通过 HTTP 429 + `Retry-After` header
//! 通知客户端。本模块抽出最小公分母：
//!
//! - 解析 `Retry-After`（秒数 / RFC 7231 IMF-fixdate）
//! - Discord 特有的 `X-RateLimit-Global` header + JSON body `global: true`
//! - 暴露 `with_rate_limit_retry` 包装一次请求，命中 429 时按 retry_after
//!   sleep 后再试，最多 N 次
//!
//! 使用前提：caller 的 request future 是 idempotent 的（POST /messages 在
//! 拿到 429 *之前* 服务端没记账，所以重试安全）。
//!
//! 接入点：Discord (`channel/discord/api.rs`)、Slack
//! (`channel/slack/api.rs`)、LINE (`channel/line/api.rs`)。

use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Response;

/// Parsed 429 metadata from a rejected response.
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// 服务端要求等待的时长。
    pub retry_after: Duration,
    /// `true` = 全 bot 维度限流（Discord X-RateLimit-Global / body.global）；
    /// caller 应阻塞所有请求而不仅限本次。
    pub is_global: bool,
}

/// 探测一个 response 是否触发 429；返回 None 表示不是 429（caller 走正常路径）。
///
/// 优先级：`Retry-After` header（秒数 → date）→ JSON body 字段
/// `retry_after`（Discord 用浮点秒）→ 兜底 1s。
pub fn parse_rate_limit_headers(resp: &Response) -> Option<RateLimitInfo> {
    if resp.status().as_u16() != 429 {
        return None;
    }

    let headers = resp.headers();
    let retry_after = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after_value)
        .unwrap_or_else(|| Duration::from_secs(1));

    let is_global = headers
        .get("x-ratelimit-global")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    Some(RateLimitInfo {
        retry_after,
        is_global,
    })
}

/// 进一步用 response body（已读取的 JSON）补全 retry_after / is_global。
///
/// Discord/Slack 把这两个字段也放在 body 里（`{"retry_after": 1.234, "global":
/// true}`），用于 header 缺失时的兜底。caller 已经把 body 读出来后，可以调
/// 这里再 enrich 一次。
pub fn enrich_with_body(info: &mut RateLimitInfo, body: &serde_json::Value) {
    if let Some(secs) = body.get("retry_after").and_then(|v| v.as_f64()) {
        if secs > 0.0 {
            info.retry_after = Duration::from_millis((secs * 1000.0) as u64);
        }
    }
    if let Some(global) = body.get("global").and_then(|v| v.as_bool()) {
        info.is_global = info.is_global || global;
    }
}

fn parse_retry_after_value(raw: &str) -> Option<Duration> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(secs) = trimmed.parse::<f64>() {
        if secs >= 0.0 {
            return Some(Duration::from_millis((secs * 1000.0) as u64));
        }
    }
    if let Ok(date) = DateTime::parse_from_rfc2822(trimmed) {
        let now = Utc::now();
        let diff = date.with_timezone(&Utc).signed_duration_since(now);
        if let Ok(std) = diff.to_std() {
            return Some(std);
        }
        return Some(Duration::from_secs(0));
    }
    None
}

/// 包装一次请求 future，命中 429 时尊重 Retry-After 后重试。
///
/// - `max_attempts`：包含首次的总尝试次数（≥1）
/// - `f`：每次重试调一次，须返回 `reqwest::Response`（caller 自己负责构造
///   request；429 之外的状态码由 caller 后续处理）
///
/// 上限：单次 sleep 最长 60s（防止恶意 Retry-After: 86400 卡死服务）。
pub async fn with_rate_limit_retry<F, Fut>(max_attempts: u32, mut f: F) -> Result<Response>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Response>>,
{
    const MAX_SLEEP: Duration = Duration::from_secs(60);
    let attempts = max_attempts.max(1);
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..attempts {
        match f().await {
            Ok(resp) => {
                if let Some(info) = parse_rate_limit_headers(&resp) {
                    if attempt + 1 >= attempts {
                        // 最后一次仍 429，把 response 还给 caller 自己处理。
                        return Ok(resp);
                    }
                    let sleep = info.retry_after.min(MAX_SLEEP);
                    tokio::time::sleep(sleep).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                last_err = Some(e);
                // 网络层错误不视为 rate limit；直接返回让上层退避
                break;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("rate limit retry exhausted")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_retry_after_seconds() {
        assert_eq!(parse_retry_after_value("5"), Some(Duration::from_secs(5)));
        assert_eq!(
            parse_retry_after_value("1.500"),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(parse_retry_after_value("0"), Some(Duration::from_secs(0)));
    }

    #[test]
    fn parse_retry_after_imf_fixdate() {
        // RFC 7231 IMF-fixdate 形式（HTTP-date 的子集，与 RFC 2822 兼容）
        let val = parse_retry_after_value("Wed, 21 Oct 2099 07:28:00 GMT");
        assert!(val.is_some());
        // 远未来日期至少 1 天
        assert!(val.unwrap() > Duration::from_secs(3600 * 24));
    }

    #[test]
    fn parse_retry_after_invalid() {
        assert!(parse_retry_after_value("").is_none());
        assert!(parse_retry_after_value("not-a-number-or-date").is_none());
        assert!(parse_retry_after_value("-5").is_none());
    }

    #[test]
    fn enrich_body_overrides_header() {
        let mut info = RateLimitInfo {
            retry_after: Duration::from_secs(1),
            is_global: false,
        };
        let body = serde_json::json!({ "retry_after": 3.5, "global": true });
        enrich_with_body(&mut info, &body);
        assert_eq!(info.retry_after, Duration::from_millis(3500));
        assert!(info.is_global);
    }

    #[test]
    fn enrich_body_skips_missing() {
        let mut info = RateLimitInfo {
            retry_after: Duration::from_secs(7),
            is_global: true,
        };
        enrich_with_body(&mut info, &serde_json::json!({}));
        assert_eq!(info.retry_after, Duration::from_secs(7));
        assert!(info.is_global);
    }
}
