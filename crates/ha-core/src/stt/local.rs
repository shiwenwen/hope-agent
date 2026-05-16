//! Known local / self-hosted STT backends.
//!
//! Each entry is a server the user runs themselves that exposes the
//! `/v1/audio/transcriptions` endpoint. The App does not manage these
//! processes — we only detect them and connect as an HTTP client. Mirrors
//! the LLM `known_local_backends` pattern.

use serde::{Deserialize, Serialize};

use super::types::{SttModelConfig, SttProviderKind};

pub const WHISPER_CPP_KEY: &str = "whisper-cpp";
pub const FASTER_WHISPER_KEY: &str = "faster-whisper";
pub const FUNASR_KEY: &str = "funasr";
pub const SHERPA_ONNX_KEY: &str = "sherpa-onnx";

pub const WHISPER_CPP_BASE_URL: &str = "http://127.0.0.1:8080";
pub const FASTER_WHISPER_BASE_URL: &str = "http://127.0.0.1:8000";
pub const FUNASR_BASE_URL: &str = "http://127.0.0.1:10097";
pub const SHERPA_ONNX_BASE_URL: &str = "http://127.0.0.1:6006";

const LOCAL_HOSTS: &[&str] = &["127.0.0.1", "localhost", "::1"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnownLocalSttBackend {
    pub key: String,
    pub name: String,
    pub kind: SttProviderKind,
    pub base_url: String,
    pub hosts: Vec<String>,
    pub port: u16,
    /// Stock model ids users typically pull for this backend. Used to
    /// pre-populate the "Add model" dropdown and to suggest sensible
    /// defaults during one-click upsert.
    pub known_models: Vec<SttModelConfig>,
    /// Install hint, English.
    pub install_hint_en: String,
    /// Install hint, Simplified Chinese.
    pub install_hint_zh: String,
    /// Canonical install URL (github / official site).
    pub install_url: String,
}

pub fn known_local_stt_backends() -> Vec<KnownLocalSttBackend> {
    vec![whisper_cpp(), faster_whisper(), funasr(), sherpa_onnx()]
}

pub fn known_local_stt_backend(key: &str) -> Option<KnownLocalSttBackend> {
    known_local_stt_backends()
        .into_iter()
        .find(|b| b.key == key)
}

pub fn known_local_stt_backend_matches(
    backend: &KnownLocalSttBackend,
    kind: SttProviderKind,
    base_url: &str,
) -> bool {
    if backend.kind != kind {
        return false;
    }
    let Some((host, port)) = parse_host_port(base_url) else {
        return false;
    };
    port == backend.port
        && backend
            .hosts
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&host))
}

fn parse_host_port(base_url: &str) -> Option<(String, u16)> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    let host = parsed
        .host_str()?
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    let port = parsed.port_or_known_default()?;
    Some((host, port))
}

// ── Catalog entries ───────────────────────────────────────────────

fn whisper_cpp() -> KnownLocalSttBackend {
    KnownLocalSttBackend {
        key: WHISPER_CPP_KEY.to_string(),
        name: "whisper.cpp server".to_string(),
        kind: SttProviderKind::OpenaiCompatible,
        base_url: WHISPER_CPP_BASE_URL.to_string(),
        hosts: LOCAL_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        port: 8080,
        known_models: vec![
            stock_model("base", "Whisper base (74M)"),
            stock_model("small", "Whisper small (244M)"),
            stock_model("medium", "Whisper medium (769M)"),
            stock_model("large-v3", "Whisper large-v3 (1.5B)"),
        ],
        install_hint_en: "Build whisper.cpp from source and run `./server -m <model> --port 8080`."
            .to_string(),
        install_hint_zh: "源码编译 whisper.cpp 后运行 `./server -m <模型> --port 8080`。"
            .to_string(),
        install_url: "https://github.com/ggerganov/whisper.cpp".to_string(),
    }
}

fn faster_whisper() -> KnownLocalSttBackend {
    KnownLocalSttBackend {
        key: FASTER_WHISPER_KEY.to_string(),
        name: "faster-whisper-server".to_string(),
        kind: SttProviderKind::OpenaiCompatible,
        base_url: FASTER_WHISPER_BASE_URL.to_string(),
        hosts: LOCAL_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        port: 8000,
        known_models: vec![
            stock_model("Systran/faster-whisper-small", "faster-whisper small"),
            stock_model("Systran/faster-whisper-medium", "faster-whisper medium"),
            stock_model("Systran/faster-whisper-large-v3", "faster-whisper large-v3"),
        ],
        install_hint_en:
            "Run faster-whisper-server (CTranslate2 accelerated Whisper, OpenAI-compatible)."
                .to_string(),
        install_hint_zh: "运行 faster-whisper-server（CTranslate2 加速版 Whisper，OpenAI 兼容）。"
            .to_string(),
        install_url: "https://github.com/fedirz/faster-whisper-server".to_string(),
    }
}

fn funasr() -> KnownLocalSttBackend {
    KnownLocalSttBackend {
        key: FUNASR_KEY.to_string(),
        name: "FunASR".to_string(),
        kind: SttProviderKind::OpenaiCompatible,
        base_url: FUNASR_BASE_URL.to_string(),
        hosts: LOCAL_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        port: 10097,
        known_models: vec![
            stock_model_lang("paraformer-zh", "Paraformer (zh)", &["zh"]),
            stock_model_lang("qwen3-asr-flash", "Qwen3-ASR Flash", &["zh", "en"]),
            stock_model_lang("sensevoice-small", "SenseVoice Small", &["zh", "en", "ja", "ko", "yue"]),
            stock_model_lang("paraformer-realtime-zh", "Paraformer Realtime (zh)", &["zh"]),
        ],
        install_hint_en:
            "Run FunASR with an OpenAI-compatible wrapper (e.g. funasr-openai-server) on port 10097.".to_string(),
        install_hint_zh:
            "运行 FunASR + OpenAI 兼容 wrapper（如 funasr-openai-server），端口 10097。".to_string(),
        install_url: "https://github.com/modelscope/FunASR".to_string(),
    }
}

fn sherpa_onnx() -> KnownLocalSttBackend {
    KnownLocalSttBackend {
        key: SHERPA_ONNX_KEY.to_string(),
        name: "sherpa-onnx server".to_string(),
        kind: SttProviderKind::OpenaiCompatible,
        base_url: SHERPA_ONNX_BASE_URL.to_string(),
        hosts: LOCAL_HOSTS.iter().map(|h| (*h).to_string()).collect(),
        port: 6006,
        known_models: vec![
            stock_model("zipformer-en", "Zipformer (en)"),
            stock_model_lang("paraformer-zh-onnx", "Paraformer ONNX (zh)", &["zh"]),
            stock_model_lang(
                "sensevoice-small-onnx",
                "SenseVoice ONNX (zh/en/ja/ko)",
                &["zh", "en", "ja", "ko"],
            ),
        ],
        install_hint_en: "Run sherpa-onnx server (ONNX runtime, ARM / edge friendly).".to_string(),
        install_hint_zh: "运行 sherpa-onnx 服务（ONNX 运行时，ARM / 嵌入式友好）。".to_string(),
        install_url: "https://github.com/k2-fsa/sherpa-onnx".to_string(),
    }
}

fn stock_model(id: &str, name: &str) -> SttModelConfig {
    SttModelConfig::new(id, name)
}

fn stock_model_lang(id: &str, name: &str, langs: &[&str]) -> SttModelConfig {
    let mut m = SttModelConfig::new(id, name);
    m.languages = langs.iter().map(|s| (*s).to_string()).collect();
    m
}

/// Cheap port probe — TCP connect with 500 ms timeout.
pub async fn probe_local_backend_alive(backend: &KnownLocalSttBackend) -> bool {
    let addr = format!("127.0.0.1:{}", backend.port);
    let connect = tokio::net::TcpStream::connect(&addr);
    matches!(
        tokio::time::timeout(std::time::Duration::from_millis(500), connect).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_four_backends_in_stable_order() {
        let catalog = known_local_stt_backends();
        let keys: Vec<&str> = catalog.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                WHISPER_CPP_KEY,
                FASTER_WHISPER_KEY,
                FUNASR_KEY,
                SHERPA_ONNX_KEY
            ]
        );
    }

    #[test]
    fn match_ignores_path_and_localhost_aliases() {
        let funasr = known_local_stt_backend(FUNASR_KEY).unwrap();
        assert!(known_local_stt_backend_matches(
            &funasr,
            SttProviderKind::OpenaiCompatible,
            "http://127.0.0.1:10097"
        ));
        assert!(known_local_stt_backend_matches(
            &funasr,
            SttProviderKind::OpenaiCompatible,
            "http://localhost:10097/v1"
        ));
        assert!(!known_local_stt_backend_matches(
            &funasr,
            SttProviderKind::OpenaiCompatible,
            "http://localhost:11434"
        ));
        assert!(!known_local_stt_backend_matches(
            &funasr,
            SttProviderKind::DeepgramWs,
            "http://localhost:10097"
        ));
    }

    #[test]
    fn funasr_carries_chinese_first_models() {
        let funasr = known_local_stt_backend(FUNASR_KEY).unwrap();
        assert!(funasr
            .known_models
            .iter()
            .any(|m| m.id == "qwen3-asr-flash"));
        assert!(funasr.known_models.iter().any(|m| m.id == "paraformer-zh"));
    }
}
