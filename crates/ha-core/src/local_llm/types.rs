use serde::{Deserialize, Serialize};

/// Resource budget axis used to size the model recommendation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetSource {
    /// macOS unified memory (system RAM doubles as VRAM)
    UnifiedMemory,
    /// Discrete GPU VRAM (Linux/Windows with dGPU)
    DedicatedVram,
    /// System RAM fallback when no dGPU is detected
    SystemMemory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    /// VRAM in MiB. `None` when the OS reports the adapter but not its memory
    /// (integrated graphics on Linux often hit this path).
    pub vram_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    /// `"macos"` / `"linux"` / `"windows"` / `"unknown"`.
    pub os: String,
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub gpu: Option<GpuInfo>,
    /// Which axis the recommender should use as the budget.
    pub budget_source: BudgetSource,
    /// 60% of the chosen axis minus a runtime buffer, in MiB. Recommendations
    /// must fit in here.
    pub budget_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCandidate {
    /// Ollama tag, e.g. `"qwen3.6:27b"`.
    pub id: String,
    pub display_name: String,
    pub family: String,
    /// On-disk size in MiB at default quantization (Q4_K_M for Qwen3.6,
    /// Ollama-default for Gemma 4). Numbers cross-checked against
    /// `ollama.com/library/<model>/tags`.
    pub size_mb: u64,
    pub context_window: u32,
    /// Whether the model supports reasoning/thinking output.
    pub reasoning: bool,
}

/// Why the recommender picked a particular budget — front-end uses this as the
/// i18n key suffix (`settings.localLlm.hardware.<reason>`). Sent over the wire
/// in kebab-case so the JSON form stays string-stable.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RecommendationReason {
    Insufficient,
    UnifiedMemory,
    Dgpu,
    RamFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecommendation {
    pub hardware: HardwareInfo,
    /// Best fit (largest model that fits in budget). `None` when budget is
    /// below the smallest catalog entry.
    pub recommended: Option<ModelCandidate>,
    /// All catalog entries that fit in the budget, descending size order.
    pub alternatives: Vec<ModelCandidate>,
    pub reason: RecommendationReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OllamaPhase {
    NotInstalled,
    Installed,
    Running,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub phase: OllamaPhase,
    pub base_url: String,
    /// `true` when the platform supports the bundled install script
    /// (`curl … install.sh | sh`). Windows users must download manually.
    pub install_script_supported: bool,
}

/// One frame of progress emitted while pulling a model. Maps onto Ollama's
/// `/api/pull` NDJSON `status` field plus our own bookkeeping phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullProgress {
    pub model_id: String,
    /// Phase string from Ollama (`"pulling manifest"`, `"downloading"`,
    /// `"verifying digest"`, `"writing manifest"`, `"success"`, …) plus our
    /// own post-download phases: `"register-provider"`,
    /// `"configure-embedding"`, `"done"`. Stays a raw String so unknown
    /// future phases pass through unmodified.
    pub phase: String,
    /// 0..=100, only set when both completed and total are known.
    pub percent: Option<u8>,
    /// Bytes downloaded so far, when Ollama reports byte-level progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_completed: Option<u64>,
    /// Total bytes expected, when Ollama reports byte-level progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPullRequest {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallScriptKind {
    Step,
    Log,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallScriptProgress {
    pub kind: InstallScriptKind,
    pub message: String,
}

/// Static catalog of Ollama tags we know how to recommend, in descending
/// size order. The recommender returns the first entry that fits in the
/// hardware budget.
///
/// Sizes track the on-disk download reported by `ollama.com/library`. Update
/// this list whenever Google / Alibaba ship a new minor release — the GUI is
/// the only thing that uses it, so a simple `cargo test` after the edit is
/// sufficient verification.
pub fn model_catalog() -> Vec<ModelCandidate> {
    vec![
        ModelCandidate {
            id: "qwen3.6:35b-a3b".into(),
            display_name: "Qwen3.6 35B (MoE A3B)".into(),
            family: "qwen3.6".into(),
            // 24 GB on disk; MoE so activation is ~3B params.
            size_mb: 24_576,
            context_window: 32_768,
            reasoning: true,
        },
        ModelCandidate {
            id: "gemma4:31b".into(),
            display_name: "Gemma 4 31B".into(),
            family: "gemma4".into(),
            size_mb: 20_480,
            context_window: 131_072,
            reasoning: false,
        },
        ModelCandidate {
            id: "gemma4:26b-a4b".into(),
            display_name: "Gemma 4 26B (MoE A4B)".into(),
            family: "gemma4".into(),
            size_mb: 18_432,
            context_window: 131_072,
            reasoning: false,
        },
        ModelCandidate {
            id: "qwen3.6:27b".into(),
            display_name: "Qwen3.6 27B".into(),
            family: "qwen3.6".into(),
            size_mb: 17_408,
            context_window: 32_768,
            reasoning: true,
        },
        ModelCandidate {
            id: "gemma4:e4b".into(),
            display_name: "Gemma 4 E4B".into(),
            family: "gemma4".into(),
            size_mb: 9_830,
            context_window: 131_072,
            reasoning: false,
        },
        ModelCandidate {
            id: "gemma4:e2b".into(),
            display_name: "Gemma 4 E2B".into(),
            family: "gemma4".into(),
            size_mb: 7_373,
            context_window: 131_072,
            reasoning: false,
        },
    ]
}
