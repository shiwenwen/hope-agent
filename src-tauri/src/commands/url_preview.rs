use crate::commands::CmdError;
use crate::url_preview;

#[tauri::command]
pub async fn fetch_url_preview(url: String) -> Result<url_preview::UrlPreviewMeta, CmdError> {
    url_preview::fetch_preview(&url).await.map_err(Into::into)
}

#[tauri::command]
pub async fn fetch_url_previews(
    urls: Vec<String>,
) -> Result<Vec<url_preview::UrlPreviewMeta>, CmdError> {
    let handles: Vec<_> = urls
        .into_iter()
        .map(|url| tokio::spawn(async move { url_preview::fetch_preview(&url).await }))
        .collect();

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Ok(meta)) = handle.await {
            results.push(meta);
        }
    }

    Ok(results)
}
