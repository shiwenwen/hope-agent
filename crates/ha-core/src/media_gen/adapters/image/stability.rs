//! Stability AI `stable-image` adapter.
//!
//! Four wire traits that are easy to get wrong, all load-bearing here:
//! - 档位（ultra / core / sd3）在 **URL 路径**里，不是 body 字段；`model` 表单字段
//!   只有 `/generate/sd3` 接受，发给 ultra / core 会 400。
//! - 请求体**只有 multipart/form-data 一种形态**，没有 JSON 变体——纯文生图也要发
//!   multipart，靠一个空的 `none` 字段占位。
//! - 审核拦截返回 **HTTP 200 + `finish_reason: CONTENT_FILTERED` + 一张模糊图**。
//!   只看状态码会把废图当成功交付给用户，所以终态必须按字段判定。
//! - 没有 size / width / height，只有九档 `aspect_ratio`；也没有 n / samples，
//!   一次调用固定一张。
//!
//! 生成端点是同步的（200 + 内联 base64），因此没有 tongyi 那样的提交-轮询循环，
//! 也没有需要二次下载的结果 URL。

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use crate::media_gen::adapters::{GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult};

const DEFAULT_BASE_URL: &str = "https://api.stability.ai";
const CONNECT_TIMEOUT_SECS: u64 = 30;

/// 官方九档宽高比。`(名称, 宽, 高)`——宽高用来在用户给了非法比例时做最近邻回退，
/// 避免把一个服务端不认的字符串直接发出去换 400。
const ASPECT_RATIOS: &[(&str, u32, u32)] = &[
    ("21:9", 21, 9),
    ("16:9", 16, 9),
    ("3:2", 3, 2),
    ("5:4", 5, 4),
    ("1:1", 1, 1),
    ("4:5", 4, 5),
    ("2:3", 2, 3),
    ("9:16", 9, 16),
    ("9:21", 9, 21),
];
const DEFAULT_ASPECT_RATIO: &str = "1:1";

/// `/generate/sd3` 的 `model` 合法取值。turbo 必须排在 large 之前，否则
/// `sd3.5-large-turbo` 会被 `sd3.5-large` 的子串匹配吃掉。
const SD3_MODELS: &[&str] = &["sd3.5-large-turbo", "sd3.5-large", "sd3.5-medium"];

/// 官方未公布 img2img 的默认 strength，取合法区间中点，用户可用 extra 覆盖。
const DEFAULT_STRENGTH: f64 = 0.5;

// ── Response ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StabilityResponse {
    /// 顶层字段，不是 OpenAI 的 `data[].b64_json`。
    image: Option<String>,
    finish_reason: Option<String>,
    /// 错误体（200 以外）也可能落到这里，用于拼更有信息量的报错。
    name: Option<String>,
    errors: Option<Vec<String>>,
}

// ── Tier / path ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Ultra,
    Core,
    Sd3,
}

impl Tier {
    fn path(self) -> &'static str {
        match self {
            Tier::Ultra => "/v2beta/stable-image/generate/ultra",
            Tier::Core => "/v2beta/stable-image/generate/core",
            Tier::Sd3 => "/v2beta/stable-image/generate/sd3",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tier::Ultra => "ultra",
            Tier::Core => "core",
            Tier::Sd3 => "sd3",
        }
    }
}

fn resolve_tier(model: &str) -> Tier {
    let m = model.to_ascii_lowercase();
    if m.contains("ultra") {
        Tier::Ultra
    } else if m.contains("core") {
        Tier::Core
    } else {
        Tier::Sd3
    }
}

/// `model` 表单字段：只有 sd3 档接受，且必须是三个合法枚举值之一。识别不出就不发，
/// 让服务端用自己的默认值，好过发一个会 400 的字符串。
fn sd3_model_field(tier: Tier, model: &str) -> Option<&'static str> {
    if tier != Tier::Sd3 {
        return None;
    }
    let m = model.to_ascii_lowercase();
    SD3_MODELS.iter().find(|cand| m.contains(*cand)).copied()
}

// ── Aspect ratio ────────────────────────────────────────────────

fn parse_ratio(s: &str) -> Option<f64> {
    let (w, h) = s.split_once(':')?;
    let w: f64 = w.trim().parse().ok()?;
    let h: f64 = h.trim().parse().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(w / h)
}

fn parse_size_ratio(size: &str) -> Option<f64> {
    let (w, h) = size.trim().split_once(['x', 'X'])?;
    let w: f64 = w.trim().parse().ok()?;
    let h: f64 = h.trim().parse().ok()?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(w / h)
}

/// 对数距离而非差值：比例是乘性量，21:9 与 16:9 的差值远大于 4:5 与 1:1，
/// 用裸差值会系统性偏向窄比例。
fn nearest_aspect_ratio(ratio: f64) -> &'static str {
    ASPECT_RATIOS
        .iter()
        .min_by(|a, b| {
            let da = ((a.1 as f64 / a.2 as f64) / ratio).ln().abs();
            let db = ((b.1 as f64 / b.2 as f64) / ratio).ln().abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|e| e.0)
        .unwrap_or(DEFAULT_ASPECT_RATIO)
}

/// 显式 aspect_ratio 命中九档直接用；非法比例或只有 size 时折算成最近的合法档位。
fn resolve_aspect_ratio(explicit: Option<&str>, size: &str) -> &'static str {
    if let Some(raw) = explicit {
        let t = raw.trim();
        if let Some(hit) = ASPECT_RATIOS.iter().find(|(name, _, _)| *name == t) {
            return hit.0;
        }
        if let Some(r) = parse_ratio(t) {
            return nearest_aspect_ratio(r);
        }
    }
    if let Some(r) = parse_size_ratio(size) {
        return nearest_aspect_ratio(r);
    }
    DEFAULT_ASPECT_RATIO
}

// ── Output format ───────────────────────────────────────────────

fn resolve_output_format(extra: &std::collections::HashMap<String, String>) -> &'static str {
    match extra
        .get("output_format")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("jpeg") | Some("jpg") => "jpeg",
        Some("webp") => "webp",
        _ => "png",
    }
}

fn mime_for_format(fmt: &str) -> &'static str {
    match fmt {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

// ── Terminal state ──────────────────────────────────────────────

/// 200 不等于成功：CONTENT_FILTERED 会连着一张模糊图一起返回。未知的非 SUCCESS
/// 值同样按失败处理——宁可报错，也不要把废图当成品交出去。
fn check_finish_reason(reason: Option<&str>) -> Result<()> {
    match reason.map(|r| r.trim().to_ascii_uppercase()).as_deref() {
        None | Some("") | Some("SUCCESS") => Ok(()),
        Some("CONTENT_FILTERED") => anyhow::bail!(
            "Stability AI 内容审核拦截（finish_reason=CONTENT_FILTERED），返回的是模糊图而非成品；请调整提示词后重试"
        ),
        Some(other) => anyhow::bail!("Stability AI 生成未成功（finish_reason={}）", other),
    }
}

// ── Adapter ─────────────────────────────────────────────────────

pub(crate) struct Provider;

impl ImageGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let tier = resolve_tier(params.model);
    let input_image = params.input_images.first();

    if input_image.is_some() && tier == Tier::Core {
        anyhow::bail!(
            "Stability AI core 档不接受参考图（image-to-image 仅 ultra / sd3 支持）；请改用 ultra 或 sd3 模型"
        );
    }
    if params.input_images.len() > 1 {
        app_warn!(
            "tool",
            "image_generate",
            "Stability AI 单次仅接受一张参考图，忽略多余 {} 张",
            params.input_images.len() - 1
        );
    }
    if params.n > 1 {
        app_warn!(
            "tool",
            "image_generate",
            "Stability AI 无 n / samples 参数，本次仅生成一张（请求 n={}）",
            params.n
        );
    }

    let url = format!("{}{}", base, tier.path());
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;

    let output_format = resolve_output_format(params.extra);
    // img2img 的画幅由参考图决定，此时不发 aspect_ratio 免得与输入冲突。
    let aspect_ratio = if input_image.is_none() {
        Some(resolve_aspect_ratio(params.aspect_ratio, params.size))
    } else {
        None
    };

    let mut form = reqwest::multipart::Form::new()
        .text("prompt", params.prompt.to_string())
        .text("output_format", output_format.to_string());

    if let Some(ar) = aspect_ratio {
        form = form.text("aspect_ratio", ar.to_string());
    }
    if let Some(model_field) = sd3_model_field(tier, params.model) {
        form = form.text("model", model_field.to_string());
    }

    if let Some(img) = input_image {
        let strength = params
            .extra
            .get("strength")
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(DEFAULT_STRENGTH)
            .clamp(0.0, 1.0);
        let mime = if img.mime.is_empty() {
            "image/png"
        } else {
            &img.mime
        };
        form = form
            .part(
                "image",
                reqwest::multipart::Part::bytes(img.data.clone())
                    .file_name("image.png")
                    .mime_str(mime)?,
            )
            .text("strength", strength.to_string());
        if tier == Tier::Sd3 {
            form = form.text("mode", "image-to-image".to_string());
        }
    } else {
        // 纯文生图也必须是 multipart；空 `none` 字段是官方示例的占位做法。
        form = form.text("none", "");
    }

    app_info!(
        "tool",
        "image_generate",
        "Stability image request: tier={}, model={}, aspect_ratio={}, format={}, img2img={}",
        tier.label(),
        params.model,
        aspect_ratio.unwrap_or("-"),
        output_format,
        input_image.is_some()
    );

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(std::time::Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    let request_start = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await?;

    let status = resp.status();
    let ttfb_ms = request_start.elapsed().as_millis() as u64;

    if let Some(logger) = crate::get_logger() {
        logger.log(
            if status.is_success() {
                "debug"
            } else {
                "error"
            },
            "tool",
            "image_generate::stability::response",
            &format!(
                "Stability image response: status={}, ttfb={}ms",
                status.as_u16(),
                ttfb_ms
            ),
            Some(
                serde_json::json!({
                    "status": status.as_u16(),
                    "ttfb_ms": ttfb_ms,
                    "tier": tier.label(),
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    // 限流是 150 请求/10 秒，429 单独点名以便用户能对上官方文档。
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Stability AI 触发限流（429，官方上限 150 请求/10 秒）: {}",
            crate::truncate_utf8(&body, 300)
        );
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Stability AI 图像生成失败 ({}): {}",
            status,
            crate::truncate_utf8(&body, 300)
        );
    }

    let body = resp.text().await?;
    let parsed: StabilityResponse = serde_json::from_str(&body).map_err(|e| {
        anyhow::anyhow!(
            "Stability AI 响应解析失败: {} — {}",
            e,
            crate::truncate_utf8(&body, 300)
        )
    })?;

    check_finish_reason(parsed.finish_reason.as_deref())?;

    let b64 = parsed.image.ok_or_else(|| {
        let detail = parsed
            .errors
            .map(|e| e.join("; "))
            .or(parsed.name)
            .unwrap_or_else(|| crate::truncate_utf8(&body, 300).to_string());
        anyhow::anyhow!("Stability AI 响应缺少 image 字段: {}", detail)
    })?;

    let data = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("Stability AI 返回的 base64 图像解码失败: {}", e))?;
    if data.is_empty() {
        anyhow::bail!("Stability AI 返回了空图像");
    }

    app_info!(
        "tool",
        "image_generate",
        "Stability image done: tier={}, {} bytes in {}ms",
        tier.label(),
        data.len(),
        request_start.elapsed().as_millis() as u64
    );

    Ok(ImageGenResult {
        images: vec![GeneratedImage {
            data,
            mime: mime_for_format(output_format).to_string(),
            revised_prompt: None,
        }],
        text: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn tier_and_path_come_from_model_name() {
        assert_eq!(resolve_tier("stable-image-ultra"), Tier::Ultra);
        assert_eq!(resolve_tier("Stable Image CORE"), Tier::Core);
        assert_eq!(resolve_tier("sd3.5-large"), Tier::Sd3);
        assert_eq!(resolve_tier("anything-else"), Tier::Sd3);
        assert_eq!(Tier::Ultra.path(), "/v2beta/stable-image/generate/ultra");
    }

    #[test]
    fn model_field_only_for_sd3_and_turbo_wins_over_large() {
        // ultra / core 发 model 字段会 400。
        assert_eq!(sd3_model_field(Tier::Ultra, "sd3.5-large"), None);
        assert_eq!(sd3_model_field(Tier::Core, "sd3.5-large"), None);
        // 子串顺序陷阱：turbo 必须先命中。
        assert_eq!(
            sd3_model_field(Tier::Sd3, "sd3.5-large-turbo"),
            Some("sd3.5-large-turbo")
        );
        assert_eq!(
            sd3_model_field(Tier::Sd3, "sd3.5-large"),
            Some("sd3.5-large")
        );
        assert_eq!(
            sd3_model_field(Tier::Sd3, "sd3.5-medium"),
            Some("sd3.5-medium")
        );
        // 认不出就不发，让服务端用默认值。
        assert_eq!(sd3_model_field(Tier::Sd3, "sd3"), None);
    }

    #[test]
    fn aspect_ratio_snaps_to_the_nine_legal_buckets() {
        assert_eq!(resolve_aspect_ratio(Some("16:9"), "1024x1024"), "16:9");
        // 非法比例 → 最近合法档位，而不是原样发出去换 400。4:3≈1.333 距 5:4 比距 3:2 近。
        assert_eq!(resolve_aspect_ratio(Some("4:3"), "1024x1024"), "5:4");
        assert_eq!(resolve_aspect_ratio(Some("32:9"), "1024x1024"), "21:9");
        // 没给比例时从 size 折算（Stability 无 size 字段）。
        assert_eq!(resolve_aspect_ratio(None, "1344x768"), "16:9");
        assert_eq!(resolve_aspect_ratio(None, "768x1344"), "9:16");
        assert_eq!(resolve_aspect_ratio(None, "1024x1024"), "1:1");
        assert_eq!(resolve_aspect_ratio(None, "garbage"), "1:1");
        assert_eq!(resolve_aspect_ratio(Some("0:0"), "0x0"), "1:1");
    }

    #[test]
    fn content_filtered_is_a_failure_despite_http_200() {
        assert!(check_finish_reason(Some("SUCCESS")).is_ok());
        assert!(check_finish_reason(None).is_ok());

        let err = check_finish_reason(Some("CONTENT_FILTERED")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("CONTENT_FILTERED"), "{msg}");

        // 未知终态同样按失败处理。
        assert!(check_finish_reason(Some("WEIRD_NEW_STATE")).is_err());
    }

    #[test]
    fn output_format_falls_back_to_png() {
        let mut extra = HashMap::new();
        assert_eq!(resolve_output_format(&extra), "png");
        assert_eq!(mime_for_format(resolve_output_format(&extra)), "image/png");

        extra.insert("output_format".into(), " JPG ".into());
        assert_eq!(resolve_output_format(&extra), "jpeg");
        assert_eq!(mime_for_format("jpeg"), "image/jpeg");

        extra.insert("output_format".into(), "webp".into());
        assert_eq!(resolve_output_format(&extra), "webp");

        extra.insert("output_format".into(), "tiff".into());
        assert_eq!(resolve_output_format(&extra), "png");
    }
}
