//! 设计空间（Design Space）子系统。
//!
//! agent 原生设计工作空间：自包含 HTML 产物 + 品牌设计系统 + 稳定预览 +
//! 可视化微调 + 一键导出。完整架构见 `docs/architecture/infra/design-space.md`。
//!
//! **零 Tauri 依赖**：业务全在此，`src-tauri` / `ha-server` 只做薄壳。

pub mod audio;
mod brands;
pub mod code_sync;
pub mod code_watcher;
pub mod compile;
pub mod components_manifest;
pub mod critique;
pub mod db;
pub mod deploy;
pub mod deploy_vercel;
pub mod design_md;
pub mod export;
pub mod extract;
pub mod figma_roundtrip;
pub mod generate;
pub mod image;
pub mod kit;
pub mod mcp_provider;
pub mod patch;
pub mod quality;
pub mod recipe;
pub mod recipe_demo;
pub mod render_native;
pub mod renderer;
pub mod review_space;
pub mod scenarios;
pub mod selfcheck;
pub mod service;
pub mod system;
pub mod theme;
pub mod threads;
pub mod token_export;

pub use critique::CritiqueResult;
pub use db::{
    DesignArtifact, DesignArtifactVersion, DesignCodeBinding, DesignComment, DesignProject,
    DesignSystemMeta,
};
pub use recipe::Recipe;
pub use renderer::{ArtifactKind, ArtifactParts};
pub use system::DesignSystemFull;
pub use threads::DesignChatThread;

// ── Config（设置三件套，见 AGENTS.md 设置约定）──────────────────────
// 类型已下沉 ha-config-schema，原地再导出保持路径不变。

pub use ha_config_schema::design::{clamp_export_jpeg_quality, clamp_export_scale, DesignConfig};

/// 设计空间是否启用。
#[allow(dead_code)]
pub fn is_design_enabled() -> bool {
    ha_core::config::cached_config().design.enabled
}

/// One-shot background model call for design generation / analysis / critique.
///
/// Single entry so every design side-task rides the unified automation model
/// chain (`function_models.automation` → chat default) through
/// `automation::run`'s chain-level failover (bad-primary-falls-through). Returns
/// the model's raw text; callers parse / validate it. Live streaming generation
/// instead calls `automation::run_streaming` directly (it needs `cancel` +
/// `on_text`). Design no longer keeps its own generation-model override — it
/// consumes the shared `function_models` config like every other background task.
pub(crate) async fn run_design_task(
    purpose: &'static str,
    session_key: &'static str,
    prompt: &str,
    max_tokens: u32,
) -> anyhow::Result<String> {
    let config = ha_core::config::cached_config();
    let chain = ha_core::automation::effective_chain(&config, None);
    if chain.is_empty() {
        anyhow::bail!(
            "no LLM provider configured — set a default model in Settings before generating designs"
        );
    }
    let out = ha_core::automation::run(ha_core::automation::ModelTaskSpec {
        purpose,
        chain,
        session_key,
        instruction: prompt,
        max_tokens,
    })
    .await?;
    Ok(out.text)
}
