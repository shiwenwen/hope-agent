use anyhow::Result;
use futures_util::StreamExt;

/// Drain `resp` into a `Vec<u8>`, truncating at `max_bytes`. Silent on cap —
/// never errors so callers decide whether a truncated body is fatal. Bounds
/// memory against hostile / misbehaving upstreams that ignore `Content-Length`.
pub async fn read_bytes_capped(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("Stream read error: {}", e))?;
        buf.extend_from_slice(&chunk);
        if buf.len() > max_bytes {
            buf.truncate(max_bytes);
            break;
        }
    }
    Ok(buf)
}

/// Like [`read_bytes_capped`] but returns a lossy UTF-8 string. `max_bytes` is
/// the post-decompression cap (reqwest transparently decodes gzip/deflate).
pub async fn read_text_capped(resp: reqwest::Response, max_bytes: usize) -> Result<String> {
    let bytes = read_bytes_capped(resp, max_bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
