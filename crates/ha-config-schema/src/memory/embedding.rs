//! Embedding wire 类型（`AppConfig.embedding_models` / `memory_embedding` /
//! `knowledge_embedding` 等字段）。
//!
//! `EmbeddingConfig` 不在原始 33 类型清单内，但它是
//! `EmbeddingModelConfig::to_runtime_config` 的返回类型（inherent impl 必须与
//! 类型定义同 crate），随闭包一并下沉。模板 / 状态投影
//! （`EmbeddingModelTemplate` / `EmbeddingSelectionState` …）与解析函数留在
//! ha-core `memory/embedding/config.rs`。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ── Embedding Config ────────────────────────────────────────────

/// Embedding provider type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingProviderType {
    /// OpenAI /v1/embeddings compatible API (OpenAI, Jina, Cohere, SiliconFlow, etc.)
    #[default]
    OpenaiCompatible,
    /// Google Gemini Embedding API (different format)
    Google,
}

/// Embedding configuration, stored in AppConfig (config.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingConfig {
    /// Whether embedding (vector search) is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Provider type
    #[serde(default)]
    pub provider_type: EmbeddingProviderType,

    // ── API mode fields ──
    /// API Base URL (e.g. `https://api.openai.com`)
    #[serde(default)]
    pub api_base_url: Option<String>,

    /// API Key
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model name (e.g. "text-embedding-3-small")
    #[serde(default)]
    pub api_model: Option<String>,

    /// Output dimensions (some APIs support specifying this)
    #[serde(default)]
    pub api_dimensions: Option<u32>,
}

/// Reusable embedding model configuration managed from the model settings UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub provider_type: EmbeddingProviderType,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_model: Option<String>,
    #[serde(default)]
    pub api_dimensions: Option<u32>,
    #[serde(default)]
    pub source: Option<String>,
}

impl EmbeddingModelConfig {
    pub fn normalize_for_save(mut self) -> Self {
        if self.id.trim().is_empty() {
            self.id = format!("emb_{}", uuid::Uuid::new_v4().simple());
        }
        self.name = self.name.trim().to_string();
        self.api_base_url = self
            .api_base_url
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());
        self.api_key = self
            .api_key
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self.api_model = self
            .api_model
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self.source = self
            .source
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if self.name.is_empty() {
            self.name = self.api_model.clone().unwrap_or_else(|| self.id.clone());
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(anyhow!("Embedding model config id is required"));
        }
        if self.name.trim().is_empty() {
            return Err(anyhow!("Embedding model config name is required"));
        }
        if self.api_base_url.as_deref().unwrap_or("").trim().is_empty() {
            return Err(anyhow!("Embedding API base URL is required"));
        }
        if self.api_model.as_deref().unwrap_or("").trim().is_empty() {
            return Err(anyhow!("Embedding model name is required"));
        }
        Ok(())
    }

    pub fn to_runtime_config(&self, enabled: bool) -> EmbeddingConfig {
        EmbeddingConfig {
            enabled,
            provider_type: self.provider_type.clone(),
            api_base_url: self.api_base_url.clone(),
            api_key: self.api_key.clone(),
            api_model: self.api_model.clone(),
            api_dimensions: self.api_dimensions,
        }
    }

    pub fn signature(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", self.provider_type).to_ascii_lowercase());
        hasher.update(b"\n");
        hasher.update(
            self.api_base_url
                .as_deref()
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_ascii_lowercase(),
        );
        hasher.update(b"\n");
        hasher.update(self.api_model.as_deref().unwrap_or("").trim());
        hasher.update(b"\n");
        hasher.update(self.api_dimensions.unwrap_or_default().to_string());
        let digest = hasher.finalize();
        format!("{:x}", digest)
    }
}

/// Active embedding selection: which model from the shared `embedding_models`
/// library is active, plus its signature lifecycle. Used independently by both
/// memory (`memory_embedding`) and knowledge (`knowledge_embedding`) — the model
/// library is shared, the selection is per-subsystem. The selected model config
/// is resolved into `EmbeddingConfig` only at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSelection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model_config_id: Option<String>,
    #[serde(default)]
    pub active_signature: Option<String>,
    #[serde(default)]
    pub last_reembedded_signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `signature()` 是记忆与知识库两套向量索引的重嵌指纹：摘要一变，所有
    /// 用户的已存 embedding 全部作废并触发全量重嵌（API 模式产生真实计费）。
    /// 本测试把「固定输入 → 固定摘要」钉死——任何改动 hash 输入集合、顺序、
    /// 规范化（trim / lowercase / 去尾斜杠）或哈希算法的行为都会在这里红，
    /// 逼迫改动者显式确认「我就是要作废全部指纹」。
    #[test]
    fn signature_digest_is_pinned() {
        let model = EmbeddingModelConfig {
            id: "emb_test".into(),
            name: "Test".into(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            // 大小写 + 尾斜杠：顺带锁定规范化行为
            api_base_url: Some("https://api.OpenAI.com/".into()),
            api_key: Some("sk-ignored-not-part-of-signature".into()),
            api_model: Some("text-embedding-3-small".into()),
            api_dimensions: Some(1536),
            source: None,
        };
        assert_eq!(
            model.signature(),
            "c252f70634355d7039b9488176623bde81cd1c54209adad66462d1a97f084829"
        );
    }

    /// api_key 刻意不进指纹（换 key 不应作废向量库）；base_url 大小写与尾
    /// 斜杠差异也不应产生新指纹。
    #[test]
    fn signature_ignores_key_and_url_cosmetics() {
        let base = EmbeddingModelConfig {
            id: "a".into(),
            name: "A".into(),
            provider_type: EmbeddingProviderType::OpenaiCompatible,
            api_base_url: Some("https://api.openai.com".into()),
            api_key: Some("k1".into()),
            api_model: Some("m".into()),
            api_dimensions: None,
            source: None,
        };
        let mut other = base.clone();
        other.api_key = Some("k2".into());
        other.api_base_url = Some("HTTPS://API.OPENAI.COM///".into());
        assert_eq!(base.signature(), other.signature());

        let mut changed = base.clone();
        changed.api_model = Some("m2".into());
        assert_ne!(base.signature(), changed.signature());
    }
}
