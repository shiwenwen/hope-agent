use crate::commands::CmdError;
use crate::docker;
use crate::tools;

#[tauri::command]
pub async fn searxng_docker_status() -> Result<docker::SearxngDockerStatus, CmdError> {
    Ok(docker::status().await)
}

#[tauri::command]
pub async fn searxng_docker_deploy(
    channel: tauri::ipc::Channel<String>,
) -> Result<String, CmdError> {
    let url = docker::deploy(|step| {
        let _ = channel.send(step.to_string());
    })
    .await?;
    // Auto-save the URL into the SearXNG provider entry and mark as docker-managed
    let url_for_mut = url.clone();
    let _ = ha_core::config::mutate_config(("web_search", "searxng-docker-deploy"), |store| {
        if let Some(entry) = store
            .web_search
            .providers
            .iter_mut()
            .find(|e| e.id == tools::web_search::WebSearchProvider::Searxng)
        {
            entry.base_url = Some(url_for_mut);
            entry.enabled = true;
        }
        store.web_search.searxng_docker_managed = Some(true);
        Ok(())
    });
    Ok(url)
}

#[tauri::command]
pub async fn searxng_docker_start() -> Result<(), CmdError> {
    docker::start().await.map_err(Into::into)
}

#[tauri::command]
pub async fn searxng_docker_stop() -> Result<(), CmdError> {
    docker::stop().await.map_err(Into::into)
}

#[tauri::command]
pub async fn searxng_docker_remove() -> Result<(), CmdError> {
    docker::remove().await?;
    // Clear docker-managed flag
    let _ = ha_core::config::mutate_config(("web_search", "searxng-docker-remove"), |store| {
        store.web_search.searxng_docker_managed = None;
        Ok(())
    });
    Ok(())
}
