use anyhow::Result;
use futures_util::StreamExt;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CappedBody {
    pub bytes: Vec<u8>,
    /// True when at least one byte beyond `max_bytes` was observed.
    pub truncated: bool,
    /// Bytes observed from the response stream.  When truncated this may be
    /// larger than `bytes.len()` by the remainder of the final chunk.
    pub received_bytes: usize,
}

/// Drain `resp` into a bounded buffer and preserve whether the stream was
/// truncated.  reqwest performs content decoding before yielding chunks when
/// the corresponding feature is enabled, so the cap applies after
/// decompression.
pub async fn read_bytes_capped_with_info(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<CappedBody> {
    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut received_bytes = 0usize;
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("Stream read error: {}", e))?;
        received_bytes = received_bytes.saturating_add(chunk.len());
        let remaining = max_bytes.saturating_sub(buf.len());
        if chunk.len() > remaining {
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(CappedBody {
        bytes: buf,
        truncated,
        received_bytes,
    })
}

/// Drain `resp` into a `Vec<u8>`, truncating at `max_bytes`. Silent on cap —
/// never errors so callers decide whether a truncated body is fatal. Bounds
/// memory against hostile / misbehaving upstreams that ignore `Content-Length`.
pub async fn read_bytes_capped(resp: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    Ok(read_bytes_capped_with_info(resp, max_bytes).await?.bytes)
}

/// Like [`read_bytes_capped`] but returns a lossy UTF-8 string. `max_bytes` is
/// the post-decompression cap (reqwest transparently decodes gzip/deflate).
pub async fn read_text_capped(resp: reqwest::Response, max_bytes: usize) -> Result<String> {
    let bytes = read_bytes_capped(resp, max_bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn cap_is_applied_after_transparent_gzip_decompression() {
        let original = vec![b'a'; 4_096];
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).expect("compress input");
        let compressed = encoder.finish().expect("finish gzip");
        assert!(compressed.len() < 1_000, "fixture should compress well");

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                compressed.len()
            );
            stream.write_all(headers.as_bytes()).await.expect("headers");
            stream.write_all(&compressed).await.expect("body");
        });
        let response = reqwest::Client::new()
            .get(format!("http://{address}/compressed"))
            .send()
            .await
            .expect("response");
        let capped = read_bytes_capped_with_info(response, 1_000)
            .await
            .expect("capped body");

        assert_eq!(capped.bytes, vec![b'a'; 1_000]);
        assert!(capped.truncated);
        assert!(capped.received_bytes > capped.bytes.len());
        server.await.expect("server task");
    }
}
