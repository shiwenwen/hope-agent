use crate::commands::CmdError;
use ha_core::local_llm::{
    detect_hardware, detect_ollama, recommend_model, start_ollama, HardwareInfo,
    ModelRecommendation, OllamaStatus,
};

#[tauri::command]
pub async fn local_llm_detect_hardware() -> Result<HardwareInfo, CmdError> {
    Ok(detect_hardware())
}

#[tauri::command]
pub async fn local_llm_recommend_model() -> Result<ModelRecommendation, CmdError> {
    let hw = detect_hardware();
    Ok(recommend_model(&hw))
}

#[tauri::command]
pub async fn local_llm_detect_ollama() -> Result<OllamaStatus, CmdError> {
    Ok(detect_ollama().await)
}

#[tauri::command]
pub async fn local_llm_start_ollama() -> Result<(), CmdError> {
    start_ollama().await.map_err(Into::into)
}
