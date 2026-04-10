use crate::docker;
use crate::tools;

#[tauri::command]
pub async fn searxng_docker_status() -> Result<docker::SearxngDockerStatus, String> {
    Ok(docker::status().await)
}

#[tauri::command]
pub async fn searxng_docker_deploy(channel: tauri::ipc::Channel<String>) -> Result<String, String> {
    let url = docker::deploy(|step| {
        let _ = channel.send(step.to_string());
    })
    .await
    .map_err(|e| e.to_string())?;
    // Auto-save the URL into the SearXNG provider entry and mark as docker-managed
    if let Ok(mut store) = oc_core::config::load_config() {
        if let Some(entry) = store
            .web_search
            .providers
            .iter_mut()
            .find(|e| e.id == tools::web_search::WebSearchProvider::Searxng)
        {
            entry.base_url = Some(url.clone());
            entry.enabled = true;
        }
        store.web_search.searxng_docker_managed = Some(true);
        let _ = oc_core::config::save_config(&store);
    }
    Ok(url)
}

#[tauri::command]
pub async fn searxng_docker_start() -> Result<(), String> {
    docker::start().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn searxng_docker_stop() -> Result<(), String> {
    docker::stop().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn searxng_docker_remove() -> Result<(), String> {
    docker::remove().await.map_err(|e| e.to_string())?;
    // Clear docker-managed flag
    if let Ok(mut store) = oc_core::config::load_config() {
        store.web_search.searxng_docker_managed = None;
        let _ = oc_core::config::save_config(&store);
    }
    Ok(())
}
