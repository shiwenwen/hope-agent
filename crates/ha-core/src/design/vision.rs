//! design 层**自包含视觉单发**（截图 → JSON）。
//!
//! side_query / one_shot 是纯文本路径；主对话 `chat` 太重（全 agent + tool loop）。
//! 这里用当前激活 provider 的凭据直发一次 vision 补全（Anthropic / OpenAI-Chat 两格式），
//! **完全不改主对话 / side_query 核心链路**（零回归风险）。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::config::AppConfig;
use crate::provider::{find_provider, ApiType, ProviderConfig};

/// vision 输出预算：需容纳完整 9 段 systemMd + 整套 token 的 JSON，2000 偏紧易截断。
const MAX_VISION_TOKENS: u32 = 4096;

/// 该 provider 格式能否走 design 自包含 vision 单发（覆盖 4 种 Provider 里的 3 种）。
fn is_vision_format(t: &ApiType) -> bool {
    matches!(
        t,
        ApiType::Anthropic | ApiType::OpenaiChat | ApiType::OpenaiResponses
    )
}

/// 视觉提取：instruction + 一张 base64 图片 → 助手文本回复。
pub async fn vision_extract(instruction: &str, mime: &str, b64: &str) -> Result<String> {
    let cfg = crate::config::cached_config();
    let (provider, model) = resolve_vision_provider(&cfg)?;
    let profile = provider
        .effective_profiles()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("provider '{}' has no API key", provider.name))?;
    let base = provider.resolve_base_url(&profile).to_string();
    let key = profile.api_key.clone();

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(90)),
    )
    .build()
    .map_err(|e| anyhow!("http client error: {e}"))?;

    crate::app_info!(
        "design",
        "vision",
        "vision extract via provider={} model={} format={:?}",
        provider.name,
        model,
        provider.api_type
    );

    match &provider.api_type {
        ApiType::Anthropic => {
            anthropic_vision(&client, &base, &key, &model, instruction, mime, b64).await
        }
        ApiType::OpenaiChat => {
            openai_vision(&client, &base, &key, &model, instruction, mime, b64).await
        }
        ApiType::OpenaiResponses => {
            openai_responses_vision(&client, &base, &key, &model, instruction, mime, b64).await
        }
        other => bail!(
            "screenshot extraction needs an Anthropic / OpenAI-Chat / OpenAI-Responses vision model \
(the active provider format is {other:?}). Switch the active model to a vision-capable provider, \
or use extract from URL / codebase / description instead."
        ),
    }
}

/// 选 vision provider：优先当前激活模型（若格式受支持 + 支持 vision），否则回退首个
/// enabled 的 Anthropic / OpenAI-Chat provider。
fn resolve_vision_provider(cfg: &AppConfig) -> Result<(&ProviderConfig, String)> {
    // Explicit override (design.extractVisionModel = "providerId:modelId"). User chose
    // it, so we trust the model but still enforce a format the code can actually call.
    if let Some(over) = cfg
        .design
        .extract_vision_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (pid, mid) = over.split_once(':').ok_or_else(|| {
            anyhow!("invalid design.extractVisionModel '{over}' (want 'providerId:modelId')")
        })?;
        let p = find_provider(&cfg.providers, pid)
            .ok_or_else(|| anyhow!("design.extractVisionModel provider '{pid}' not found"))?;
        if !is_vision_format(&p.api_type) {
            bail!(
                "design.extractVisionModel provider '{}' is {:?}; screenshot extraction \
needs an Anthropic / OpenAI-Chat / OpenAI-Responses vision model",
                p.name,
                p.api_type
            );
        }
        return Ok((p, mid.to_string()));
    }
    if let Some(am) = &cfg.active_model {
        if let Some(p) = find_provider(&cfg.providers, &am.provider_id) {
            if is_vision_format(&p.api_type) && p.model_supports_vision(&am.model_id) {
                return Ok((p, am.model_id.clone()));
            }
        }
    }
    // 回退：首个 enabled 且**真支持 vision** 的模型（此前盲取 models.first() 可能选到
    // 纯文本模型，到 API 才失败——与 active_model 路径的 vision 校验对齐）。
    for p in &cfg.providers {
        if p.enabled && is_vision_format(&p.api_type) {
            if let Some(m) = p.models.iter().find(|m| p.model_supports_vision(&m.id)) {
                return Ok((p, m.id.clone()));
            }
        }
    }
    bail!("no vision-capable Anthropic / OpenAI-Chat / OpenAI-Responses provider configured for screenshot extraction")
}

/// `base_url` 归一化：末尾已含 `/v1` 则直接接后缀，否则补 `/v1`。
fn join_v1(base: &str, suffix: &str) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/v1") {
        format!("{b}{suffix}")
    } else {
        format!("{b}/v1{suffix}")
    }
}

async fn anthropic_vision(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    instruction: &str,
    mime: &str,
    b64: &str,
) -> Result<String> {
    let url = join_v1(base, "/messages");
    let body = json!({
        "model": model,
        "max_tokens": MAX_VISION_TOKENS,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": instruction },
                { "type": "image", "source": { "type": "base64", "media_type": mime, "data": b64 } }
            ]
        }]
    });
    let resp = client
        .post(&url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("anthropic vision request failed: {e}"))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("anthropic vision response parse failed: {e}"))?;
    if !status.is_success() {
        bail!("anthropic vision error {}: {}", status.as_u16(), v);
    }
    let text = v["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b["type"] == "text")
                .and_then(|b| b["text"].as_str())
        })
        .unwrap_or_default()
        .to_string();
    if text.trim().is_empty() {
        bail!("empty vision response from anthropic");
    }
    Ok(text)
}

async fn openai_vision(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    instruction: &str,
    mime: &str,
    b64: &str,
) -> Result<String> {
    let url = join_v1(base, "/chat/completions");
    let data_uri = format!("data:{mime};base64,{b64}");
    let body = json!({
        "model": model,
        "max_tokens": MAX_VISION_TOKENS,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": instruction },
                { "type": "image_url", "image_url": { "url": data_uri } }
            ]
        }]
    });
    let resp = client
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("openai vision request failed: {e}"))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("openai vision response parse failed: {e}"))?;
    if !status.is_success() {
        bail!("openai vision error {}: {}", status.as_u16(), v);
    }
    let text = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if text.trim().is_empty() {
        bail!("empty vision response from openai");
    }
    Ok(text)
}

async fn openai_responses_vision(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    model: &str,
    instruction: &str,
    mime: &str,
    b64: &str,
) -> Result<String> {
    let url = join_v1(base, "/responses");
    let data_uri = format!("data:{mime};base64,{b64}");
    let body = json!({
        "model": model,
        "max_output_tokens": MAX_VISION_TOKENS,
        "input": [{
            "role": "user",
            "content": [
                { "type": "input_text", "text": instruction },
                { "type": "input_image", "image_url": data_uri }
            ]
        }]
    });
    let resp = client
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("openai responses vision request failed: {e}"))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("openai responses vision response parse failed: {e}"))?;
    if !status.is_success() {
        bail!("openai responses vision error {}: {}", status.as_u16(), v);
    }
    // Responses API：优先 `output_text` 便捷字段，否则从 `output[].content[]` 收集 output_text。
    let text = v["output_text"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            v["output"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|it| it["content"].as_array())
                        .flatten()
                        .filter(|c| c["type"] == "output_text")
                        .filter_map(|c| c["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default()
        });
    if text.trim().is_empty() {
        bail!("empty vision response from openai responses");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_v1_variants() {
        assert_eq!(
            join_v1("https://api.anthropic.com", "/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            join_v1("https://api.openai.com/v1", "/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            join_v1("https://gw.example.com/v1/", "/chat/completions"),
            "https://gw.example.com/v1/chat/completions"
        );
    }
}
