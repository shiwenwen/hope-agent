//! iFlytek (讯飞) IAT realtime WebSocket transcription.
//!
//! Reference docs:
//! - <https://www.xfyun.cn/doc/asr/voicedictation/API.html>
//!
//! Endpoints (the implementation defaults to the first; the GUI lets a user
//! override `base_url` if they need the alternate hosts):
//! - `wss://iat-api.xfyun.cn/v2/iat` (recommended, zh + en)
//! - `wss://ws-api.xfyun.cn/v2/iat` (alternate)
//! - `wss://iat-niche-api.xfyun.cn/v2/iat` (minority-language pool)
//!
//! URL must carry `host`, `date`, and HMAC-SHA256 `authorization` query
//! params per the iFlytek auth contract (signature derived from APIKey +
//! APISecret + `app_id`).
//!
//! Auth dance:
//! 1. Sign `"host: iat-api.xfyun.cn\ndate: <RFC1123>\nGET /v2/iat HTTP/1.1"`
//!    with HMAC-SHA256 keyed by APISecret → base64.
//! 2. Pack `api_key="...", algorithm="hmac-sha256", headers="host date
//!    request-line", signature="<base64>"` into one string, base64 it,
//!    URL-encode → `authorization` query param.
//! 3. Append `date`, `host`, `authorization` as query params; the WS
//!    upgrade carries the rest.
//!
//! Wire shape:
//! - Upstream: JSON text frames, each carries `data.status` ∈ {0=first,
//!   1=cont, 2=last} and a base64 chunk of PCM16 (16 kHz mono).
//! - Downstream: JSON `{code, data:{status,result:{ws:[{cw:[{w:"..."}]}],
//!   pgs:"apd"|"rpl"}}}`. `pgs=rpl` means replace earlier partials at
//!   the same `sn`.
//!
//! `app_id`, `api_secret` belong in `provider.extra` (the user pastes
//! APIKey into `apiKey`, APISecret into `extra.api_secret`, APPID into
//! `extra.app_id`).

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::provider::AuthProfile;
use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{SttModelConfig, SttProviderConfig, TranscriptDelta, TranscriptOptions};

pub async fn open_stream(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    options: &TranscriptOptions,
) -> SttResult<super::SttStream> {
    let api_secret = provider
        .require_extra("api_secret", "APISecret")?
        .to_string();
    let app_id = provider.require_extra("app_id", "APPID")?.to_string();

    let base = provider.resolve_base_url(profile).trim_end_matches('/');
    let path = "/v2/iat";
    let signed_url = build_signed_url(base, path, &profile.api_key, &api_secret)?;

    let https_twin = super::ws_to_https_twin(&signed_url, "iFlytek")?;
    provider.check_ssrf(&https_twin).await?;

    let request = signed_url
        .as_str()
        .into_client_request()
        .map_err(|e| SttError::Other(format!("Invalid iFlytek URL: {e}")))?;

    let ws = super::ws_connect_with_caps(request, "iFlytek").await?;
    let (mut ws_sink, mut ws_stream) = ws.split();

    let language = options
        .language
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or("zh_cn")
        .to_string();
    // iFlytek IAT doesn't have a `model id` concept on the wire — the
    // capability switch is `business.domain` (iat / iat-niche-chs /
    // medical / fpa / …). We surface that through the model id so the
    // existing GUI flow (provider has models → pick one as active) maps
    // onto something real. `accent` is configurable via `extra.accent`
    // (defaults to mandarin); `language` flows through `options.language`.
    let domain = if model.id.trim().is_empty() {
        "iat".to_string()
    } else {
        model.id.trim().to_string()
    };
    let accent = provider
        .extra
        .get("accent")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("mandarin")
        .to_string();
    let business = json!({
        "language": language,
        "domain": domain,
        "accent": accent,
        "vad_eos": 10_000_u32,
    });

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(super::STT_STREAM_CHANNEL_CAPACITY);
    let (delta_tx, delta_rx) =
        mpsc::channel::<Result<TranscriptDelta, SttError>>(super::STT_STREAM_CHANNEL_CAPACITY);

    tokio::spawn(async move {
        let mut first = true;
        while let Some(chunk) = audio_rx.recv().await {
            let status: u8 = if first { 0 } else { 1 };
            let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&chunk);
            let frame = if first {
                first = false;
                json!({
                    "common": { "app_id": app_id },
                    "business": business,
                    "data": {
                        "status": status,
                        "format": "audio/L16;rate=16000",
                        "encoding": "raw",
                        "audio": audio_b64,
                    }
                })
            } else {
                json!({
                    "data": {
                        "status": status,
                        "format": "audio/L16;rate=16000",
                        "encoding": "raw",
                        "audio": audio_b64,
                    }
                })
            };
            if ws_sink
                .send(Message::Text(frame.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }
        // EOS frame: status=2, empty audio. Then let iFlytek flush
        // its trailing data.status=2 final-result frame before closing
        // — sending WS Close here races against the terminal frame.
        let last = json!({
            "data": {
                "status": 2_u8,
                "format": "audio/L16;rate=16000",
                "encoding": "raw",
                "audio": "",
            }
        });
        let _ = ws_sink.send(Message::Text(last.to_string().into())).await;
    });

    let session_id = String::new();
    tokio::spawn(async move {
        let mut accumulated: std::collections::BTreeMap<i64, String> =
            std::collections::BTreeMap::new();
        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match parse_message(&session_id, &text, &mut accumulated) {
                        Ok(Some(delta)) => {
                            if delta_tx.send(Ok(delta)).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let _ = delta_tx.send(Err(e)).await;
                            break;
                        }
                    }
                }
                Ok(Message::Binary(_))
                | Ok(Message::Frame(_))
                | Ok(Message::Ping(_))
                | Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    let _ = delta_tx
                        .send(Err(SttError::Network(format!("iFlytek WS recv: {e}"))))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(super::SttStream { audio_tx, delta_rx })
}

fn build_signed_url(
    base_wss: &str,
    path: &str,
    api_key: &str,
    api_secret: &str,
) -> SttResult<String> {
    let parsed = url::Url::parse(base_wss)
        .map_err(|e| SttError::Other(format!("Invalid iFlytek base URL: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| SttError::Other("iFlytek base URL is missing host".into()))?;
    let date = http_date_rfc1123();

    let signature_origin = format!("host: {}\ndate: {}\nGET {} HTTP/1.1", host, date, path);
    type HmacSha256 = Hmac<Sha256>;
    // HMAC-SHA256 accepts any key length, so `new_from_slice` is
    // infallible here — `expect` keeps the type signature simple.
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
        .expect("HMAC-SHA256 accepts any key length");
    mac.update(signature_origin.as_bytes());
    let signature_b64 =
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let authorization_origin = format!(
        "api_key=\"{}\", algorithm=\"hmac-sha256\", headers=\"host date request-line\", signature=\"{}\"",
        api_key, signature_b64
    );
    let authorization =
        base64::engine::general_purpose::STANDARD.encode(authorization_origin.as_bytes());

    Ok(format!(
        "{}{}?authorization={}&date={}&host={}",
        base_wss.trim_end_matches('/'),
        path,
        urlencoding::encode(&authorization),
        urlencoding::encode(&date),
        urlencoding::encode(host),
    ))
}

fn http_date_rfc1123() -> String {
    // iFlytek accepts RFC1123 GMT, e.g. `Tue, 16 May 2026 12:34:56 GMT`.
    chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// Parse one server frame. iFlytek can send `pgs=apd` (append the new
/// `sn` segment) or `pgs=rpl` with `rg=[start, end]` (replace previously
/// emitted segments in that sn range). We index by `sn` so a corrective
/// `rpl` only rewrites the dynamic tail and the stable prefix survives.
///
/// Returns `Err` for non-zero server `code` values (auth failure, quota
/// exceeded, etc) so the downstream task can surface a useful error
/// instead of letting `finalize` return an empty-but-Ok transcript.
/// Returns `Ok(None)` for keep-alive / no-text frames.
fn parse_message(
    session_id: &str,
    raw: &str,
    acc: &mut std::collections::BTreeMap<i64, String>,
) -> SttResult<Option<TranscriptDelta>> {
    let value: Value = match serde_json::from_str::<Value>(raw) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
    if code != 0 {
        let message = value
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        let sid = value
            .get("sid")
            .and_then(|v| v.as_str())
            .unwrap_or("<no-sid>");
        return Err(classify_xunfei_code(code, message, sid));
    }
    let Some(data) = value.get("data") else {
        return Ok(None);
    };
    let status = data.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let is_final = status == 2;

    let sn = data
        .get("result")
        .and_then(|r| r.get("sn"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| acc.keys().next_back().map(|k| k + 1).unwrap_or(0));

    let mut chunk = String::new();
    if let Some(ws) = data
        .get("result")
        .and_then(|r| r.get("ws"))
        .and_then(|v| v.as_array())
    {
        for w in ws {
            if let Some(cw) = w.get("cw").and_then(|v| v.as_array()) {
                for c in cw {
                    if let Some(text) = c.get("w").and_then(|v| v.as_str()) {
                        chunk.push_str(text);
                    }
                }
            }
        }
    }

    let pgs = data
        .get("result")
        .and_then(|r| r.get("pgs"))
        .and_then(|v| v.as_str());
    if pgs == Some("rpl") {
        // `rg=[start_sn, end_sn]` — iFlytek's dynamic-correction
        // contract: drop all segments whose sn falls in that range,
        // then install the new chunk at the current sn. Falls back to
        // "replace just this sn" if rg is malformed.
        let rg = data
            .get("result")
            .and_then(|r| r.get("rg"))
            .and_then(|v| v.as_array());
        let (start_sn, end_sn) = match rg {
            Some(arr) if arr.len() == 2 => {
                (arr[0].as_i64().unwrap_or(sn), arr[1].as_i64().unwrap_or(sn))
            }
            _ => (sn, sn),
        };
        acc.retain(|k, _| !(start_sn..=end_sn).contains(k));
    }
    if !chunk.is_empty() {
        acc.insert(sn, chunk);
    }

    if acc.is_empty() && !is_final {
        return Ok(None);
    }
    let text: String = acc.values().cloned().collect();
    Ok(Some(TranscriptDelta {
        session_id: session_id.to_string(),
        text,
        is_final,
        start_ms: None,
        end_ms: None,
        confidence: None,
        language: None,
        accumulated: None,
    }))
}

/// Map iFlytek's numeric `code` field to a typed `SttError`. Codes
/// covered are pulled from the IAT documentation's error table; anything
/// else falls through to `Other` with the raw fields so logs aren't
/// blank.
fn classify_xunfei_code(code: i64, message: &str, sid: &str) -> SttError {
    let detail = format!("iFlytek code {code}: {message} (sid {sid})");
    match code {
        // 10005..10007 — appid / authentication failures.
        10005..=10007 => SttError::Auth(detail),
        // 10160..10163 — invalid request parameters (often bad audio config).
        10160 | 10161 | 10163 => SttError::UnsupportedAudio(detail),
        // 10043 / 10165 — concurrent / QPS limit.
        10043 | 10165 => SttError::RateLimit(detail),
        // 10114 / 10202 — service busy, transient.
        10114 | 10202 => SttError::ProviderUnavailable(detail),
        _ => SttError::Other(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date_includes_weekday_and_gmt() {
        let d = http_date_rfc1123();
        assert!(d.ends_with(" GMT"));
        // Weekday prefix is three letters + comma + space.
        assert_eq!(d.chars().nth(3), Some(','));
    }

    #[test]
    fn build_signed_url_contains_required_params() {
        let url =
            build_signed_url("wss://iat-api.xfyun.cn", "/v2/iat", "key123", "secret456").unwrap();
        assert!(url.starts_with("wss://iat-api.xfyun.cn/v2/iat?"));
        assert!(url.contains("authorization="));
        assert!(url.contains("date="));
        assert!(url.contains("host="));
    }

    fn new_acc() -> std::collections::BTreeMap<i64, String> {
        std::collections::BTreeMap::new()
    }

    #[test]
    fn parse_append_accumulates_partials() {
        let mut acc = new_acc();
        let frame = r#"{"code":0,"data":{"status":1,"result":{"sn":0,"ws":[{"cw":[{"w":"你"}]}],"pgs":"apd"}}}"#;
        let d = parse_message("s", frame, &mut acc).unwrap().unwrap();
        assert_eq!(d.text, "你");
        let frame2 = r#"{"code":0,"data":{"status":1,"result":{"sn":1,"ws":[{"cw":[{"w":"好"}]}],"pgs":"apd"}}}"#;
        let d2 = parse_message("s", frame2, &mut acc).unwrap().unwrap();
        assert_eq!(d2.text, "你好");
        assert!(!d2.is_final);
    }

    #[test]
    fn parse_replace_preserves_stable_prefix_outside_rg() {
        // Build a 3-segment history then dynamic-correct only sn=2.
        // Earlier bug wiped the entire accumulator on any `rpl` —
        // stable prefix at sn=0/1 must survive.
        let mut acc = new_acc();
        let p0 = r#"{"code":0,"data":{"status":1,"result":{"sn":0,"ws":[{"cw":[{"w":"稳"}]}],"pgs":"apd"}}}"#;
        parse_message("s", p0, &mut acc).unwrap();
        let p1 = r#"{"code":0,"data":{"status":1,"result":{"sn":1,"ws":[{"cw":[{"w":"定"}]}],"pgs":"apd"}}}"#;
        parse_message("s", p1, &mut acc).unwrap();
        let p2 = r#"{"code":0,"data":{"status":1,"result":{"sn":2,"ws":[{"cw":[{"w":"旧"}]}],"pgs":"apd"}}}"#;
        parse_message("s", p2, &mut acc).unwrap();
        let correction = r#"{"code":0,"data":{"status":1,"result":{"sn":2,"rg":[2,2],"ws":[{"cw":[{"w":"新"}]}],"pgs":"rpl"}}}"#;
        let d = parse_message("s", correction, &mut acc).unwrap().unwrap();
        assert_eq!(d.text, "稳定新");
    }

    #[test]
    fn parse_status_two_marks_final() {
        let mut acc = new_acc();
        acc.insert(0, "hello".into());
        let frame = r#"{"code":0,"data":{"status":2,"result":{"sn":1,"ws":[{"cw":[{"w":""}]}],"pgs":"apd"}}}"#;
        let d = parse_message("s", frame, &mut acc).unwrap().unwrap();
        assert!(d.is_final);
        assert_eq!(d.text, "hello");
    }

    #[test]
    fn parse_non_zero_code_returns_auth_error() {
        let mut acc = new_acc();
        let frame = r#"{"code":10005,"message":"invalid appid","sid":"sid-abc"}"#;
        let err = parse_message("s", frame, &mut acc).unwrap_err();
        match err {
            SttError::Auth(msg) => {
                assert!(msg.contains("10005"));
                assert!(msg.contains("invalid appid"));
                assert!(msg.contains("sid-abc"));
            }
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn parse_unknown_code_falls_through_to_other() {
        let mut acc = new_acc();
        let frame = r#"{"code":99999,"message":"weird"}"#;
        let err = parse_message("s", frame, &mut acc).unwrap_err();
        assert!(matches!(err, SttError::Other(_)));
    }
}
