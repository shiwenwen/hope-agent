use std::collections::HashMap;
use std::time::Duration;

use ha_core::security::ssrf::SsrfPolicy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{siliconflow::SiliconFlowProvider, zhipu::ZhipuProvider};
use crate::media_gen::adapters::{ImageGenAdapter, ImageGenParams};

#[tokio::test]
async fn returned_image_urls_use_guarded_redirects_without_api_credentials() {
    let _config_restore =
        ha_core::test_support::replace_config_cache(ha_core::config::AppConfig::default());
    for vendor in ["siliconflow", "zhipu"] {
        // Success, initial-URL rejection and a forbidden second hop each
        // exercise the real adapter and shared download path.
        for scenario in ["success", "strict", "metadata"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let asset = format!("{base}/asset");
            let body = if vendor == "siliconflow" {
                serde_json::json!({"images": [{"url": asset}]})
            } else {
                serde_json::json!({"data": [{"url": asset}]})
            }
            .to_string();
            let expected_requests = match scenario {
                "success" => 3,
                "strict" => 1,
                _ => 2,
            };
            let server = tokio::spawn(async move {
                for step in 0..expected_requests {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let header_end = loop {
                        let mut buf = [0; 1024];
                        let len = socket.read(&mut buf).await.unwrap();
                        assert_ne!(len, 0);
                        request.extend_from_slice(&buf[..len]);
                        if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
                    let body_len = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .map(|value| value.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    while request.len() < header_end + body_len {
                        let mut buf = [0; 1024];
                        let len = socket.read(&mut buf).await.unwrap();
                        assert_ne!(len, 0);
                        request.extend_from_slice(&buf[..len]);
                    }
                    let response = match step {
                        0 => {
                            assert!(headers.starts_with("post "));
                            assert!(
                                headers.contains("authorization: bearer synthetic-test-key\r\n")
                            );
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        }
                        1 => {
                            assert!(headers.starts_with("get /asset "));
                            assert!(!headers.contains("authorization:"));
                            let target = if scenario == "metadata" {
                                "http://169.254.169.254/latest/meta-data"
                            } else {
                                "/final"
                            };
                            format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        }
                        _ => {
                            assert!(headers.starts_with("get /final "));
                            assert!(!headers.contains("authorization:"));
                            "HTTP/1.1 200 OK\r\nContent-Type: image/webp; charset=binary\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage".to_string()
                        }
                    };
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
            });
            let adapter: &dyn ImageGenAdapter = if vendor == "siliconflow" {
                &SiliconFlowProvider
            } else {
                &ZhipuProvider
            };
            let extra = HashMap::new();
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                adapter.generate(ImageGenParams {
                    api_key: "synthetic-test-key",
                    base_url: Some(&base),
                    model: "synthetic-model",
                    prompt: "synthetic prompt",
                    size: "1024x1024",
                    n: 1,
                    timeout_secs: 5,
                    extra: &extra,
                    aspect_ratio: None,
                    resolution: None,
                    input_images: &[],
                    mask: None,
                    ssrf: if scenario == "strict" {
                        SsrfPolicy::Strict
                    } else {
                        SsrfPolicy::AllowPrivate
                    },
                }),
            )
            .await
            .expect("local adapter request timed out");
            if scenario == "success" {
                let images = result.unwrap().images;
                assert_eq!(images.len(), 1);
                assert_eq!(images[0].data, b"image");
                assert_eq!(images[0].mime, "image/webp");
            } else {
                assert!(result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("blocked asset URL"));
            }
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .expect("local test server timed out")
                .unwrap();
        }
    }
}
