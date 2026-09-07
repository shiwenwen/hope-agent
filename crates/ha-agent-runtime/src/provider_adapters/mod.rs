pub(crate) mod anthropic_adapter;
pub(crate) mod cancel;
pub(crate) mod codex_adapter;
pub(crate) mod openai_chat_adapter;
pub(crate) mod openai_responses_adapter;

#[cfg(test)]
mod test_support {
    use crate::agent::streaming_adapter::RoundRequest;
    use serde_json::Value;

    pub fn round_request(history: &[Value]) -> RoundRequest<'_> {
        RoundRequest {
            session_id: Some("synthetic-contract"),
            system_prompt: "stable system",
            run_instruction_suffix: None,
            run_data_suffix: None,
            awareness_suffix: None,
            active_memory_suffix: None,
            legacy_memory_suffix: None,
            coding_profile_suffix: None,
            procedure_memory_suffix: None,
            related_notes_suffix: None,
            attached_knowledge_suffix: None,
            capability_catalog_suffix: None,
            user_profile_suffix: None,
            environment_context_suffix: None,
            lsp_diagnostics_suffix: None,
            task_reminder_suffix: None,
            tool_schemas: &[],
            deferred_tool_schemas: &[],
            eager_tool_count: 0,
            deferred_tool_count: 0,
            activated_tool_count: 0,
            prompt_cache_key: None,
            history_for_api: history,
            vision_bridge_available: false,
            reasoning_effort: None,
            temperature: Some(0.2),
            max_tokens: 4096,
            is_final_round: false,
            round: 0,
        }
    }

    /// Loopback-only response fixture; no credentials or real provider calls.
    pub async fn sse_response(events: &[Value]) -> reqwest::Response {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let body: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let len = socket.read(&mut buffer).await.unwrap();
                assert!(len > 0);
                request.extend_from_slice(&buffer[..len]);
                assert!(request.len() < 8192);
            }
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
            ).as_bytes()).await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/synthetic-sse"))
            .send()
            .await
            .unwrap();
        server.await.unwrap();
        response
    }
}
