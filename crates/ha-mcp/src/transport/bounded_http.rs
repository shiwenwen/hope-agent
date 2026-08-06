//! Byte-bounded Streamable HTTP client for rmcp.
//!
//! rmcp's default reqwest adapter deserializes JSON responses and accumulates
//! SSE events without a response-size ceiling. Catalog discovery is reachable
//! from ordinary chat, so every inbound JSON-RPC message must be bounded before
//! serde or the SSE parser materializes attacker-controlled collections.

use std::{borrow::Cow, collections::HashMap, io, sync::Arc};

use futures_util::{stream::BoxStream, StreamExt};
use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, JsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, SseError, StreamableHttpClient, StreamableHttpError,
    StreamableHttpPostResponse,
};
use sse_stream::SseStream;

pub(super) const MAX_INBOUND_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

const HEADER_SESSION_ID: &str = "mcp-session-id";
const HEADER_LAST_EVENT_ID: &str = "last-event-id";
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const JSON_MIME_TYPE: &str = "application/json";

#[derive(Clone)]
pub(super) struct BoundedMcpHttpClient {
    client: reqwest::Client,
}

impl BoundedMcpHttpClient {
    pub(super) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

fn client_error(error: impl std::fmt::Display) -> StreamableHttpError<io::Error> {
    StreamableHttpError::Client(io::Error::other(error.to_string()))
}

fn apply_custom_headers(
    mut request: reqwest::RequestBuilder,
    headers: HashMap<HeaderName, HeaderValue>,
) -> Result<reqwest::RequestBuilder, StreamableHttpError<io::Error>> {
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "accept" | HEADER_SESSION_ID | HEADER_LAST_EVENT_ID
        ) {
            return Err(StreamableHttpError::ReservedHeaderConflict(
                name.to_string(),
            ));
        }
        request = request.header(name, value);
    }
    Ok(request)
}

async fn read_body_bounded(response: reqwest::Response) -> Result<Vec<u8>, io::Error> {
    read_body_with_limit(response, MAX_INBOUND_MESSAGE_BYTES).await
}

async fn read_body_with_limit(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, io::Error> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(io::Error::other(format!(
            "MCP response exceeds the {}-byte limit",
            max_bytes
        )));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| io::Error::other(error.to_string()))?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(io::Error::other(format!(
                "MCP response exceeds the {}-byte limit",
                max_bytes
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Default)]
struct SseFrameBudget {
    event_bytes: usize,
    line_content_bytes: usize,
    previous_was_cr: bool,
}

impl SseFrameBudget {
    fn observe(&mut self, chunk: &[u8]) -> io::Result<()> {
        for byte in chunk {
            self.event_bytes = self.event_bytes.saturating_add(1);
            if self.event_bytes > MAX_INBOUND_MESSAGE_BYTES {
                return Err(io::Error::other(format!(
                    "MCP SSE event exceeds the {}-byte limit",
                    MAX_INBOUND_MESSAGE_BYTES
                )));
            }

            match *byte {
                b'\r' => {
                    self.finish_line();
                    self.previous_was_cr = true;
                }
                b'\n' if self.previous_was_cr => {
                    self.previous_was_cr = false;
                }
                b'\n' => self.finish_line(),
                _ => {
                    self.previous_was_cr = false;
                    self.line_content_bytes = self.line_content_bytes.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    fn finish_line(&mut self) {
        if self.line_content_bytes == 0 {
            self.event_bytes = 0;
        }
        self.line_content_bytes = 0;
    }
}

pub(super) fn bounded_sse_stream(
    response: reqwest::Response,
) -> BoxStream<'static, Result<sse_stream::Sse, SseError>> {
    let byte_stream = response.bytes_stream().scan(
        (SseFrameBudget::default(), false),
        |(budget, terminated), chunk| {
            let next = if *terminated {
                None
            } else {
                Some(match chunk {
                    Ok(bytes) => match budget.observe(&bytes) {
                        Ok(()) => Ok(bytes),
                        Err(error) => {
                            *terminated = true;
                            Err(error)
                        }
                    },
                    Err(error) => {
                        *terminated = true;
                        Err(io::Error::other(error.to_string()))
                    }
                })
            };
            std::future::ready(next)
        },
    );
    SseStream::from_bytes_stream(byte_stream).boxed()
}

fn parse_json_rpc_error(body: &[u8]) -> Option<ServerJsonRpcMessage> {
    match serde_json::from_slice::<ServerJsonRpcMessage>(body) {
        Ok(message @ JsonRpcMessage::Error(_)) => Some(message),
        _ => None,
    }
}

fn extract_scope(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let start = lower.find("scope=")? + "scope=".len();
    let value = &header[start..];
    if let Some(quoted) = value.strip_prefix('"') {
        return quoted.find('"').map(|end| quoted[..end].to_string());
    }
    let end = value
        .find(|character: char| character == ',' || character == ';' || character.is_whitespace())
        .unwrap_or(value.len());
    (end > 0).then(|| value[..end].to_string())
}

impl StreamableHttpClient for BoundedMcpHttpClient {
    type Error = io::Error;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<
        BoxStream<'static, Result<sse_stream::Sse, SseError>>,
        StreamableHttpError<Self::Error>,
    > {
        let mut request = self
            .client
            .get(uri.as_ref())
            .header(
                reqwest::header::ACCEPT,
                [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
            )
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(auth_header) = auth_header {
            request = request.bearer_auth(auth_header);
        }
        let response = apply_custom_headers(request, custom_headers)?
            .send()
            .await
            .map_err(client_error)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        let response = response.error_for_status().map_err(client_error)?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|value| value.as_bytes());
        if !content_type.is_some_and(|value| {
            value.starts_with(EVENT_STREAM_MIME_TYPE.as_bytes())
                || value.starts_with(JSON_MIME_TYPE.as_bytes())
        }) {
            return Err(StreamableHttpError::UnexpectedContentType(
                content_type.map(|value| String::from_utf8_lossy(value).to_string()),
            ));
        }
        Ok(bounded_sse_stream(response))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .delete(uri.as_ref())
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(auth_header) = auth_header {
            request = request.bearer_auth(auth_header);
        }
        let response = apply_custom_headers(request, custom_headers)?
            .send()
            .await
            .map_err(client_error)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        response.error_for_status().map_err(client_error)?;
        Ok(())
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self.client.post(uri.as_ref()).header(
            reqwest::header::ACCEPT,
            [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
        );
        if let Some(auth_header) = auth_header {
            request = request.bearer_auth(auth_header);
        }
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = &session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = apply_custom_headers(request, custom_headers)?
            .json(&message)
            .send()
            .await
            .map_err(client_error)?;

        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            if let Some(header) = response.headers().get(reqwest::header::WWW_AUTHENTICATE) {
                let header = header
                    .to_str()
                    .map_err(|error| client_error(error))?
                    .to_string();
                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                        header,
                    )));
                }
                return Err(StreamableHttpError::InsufficientScope(
                    InsufficientScopeError::new(header.clone(), extract_scope(&header)),
                ));
            }
        }

        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).to_string());
        let content_length = response.content_length();
        let response_session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        if status.is_success()
            && content_length == Some(0)
            && matches!(
                message,
                ClientJsonRpcMessage::Notification(_)
                    | ClientJsonRpcMessage::Response(_)
                    | ClientJsonRpcMessage::Error(_)
            )
        {
            return Ok(StreamableHttpPostResponse::Accepted);
        }

        if !status.is_success() {
            let body = read_body_bounded(response).await.map_err(client_error)?;
            if content_type
                .as_deref()
                .is_some_and(|value| value.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()))
            {
                if let Some(message) = parse_json_rpc_error(&body) {
                    return Ok(StreamableHttpPostResponse::Json(
                        message,
                        response_session_id,
                    ));
                }
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
                format!("HTTP {status}: {}", String::from_utf8_lossy(&body)),
            )));
        }

        match content_type.as_deref() {
            Some(value)
                if value
                    .as_bytes()
                    .starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) =>
            {
                Ok(StreamableHttpPostResponse::Sse(
                    bounded_sse_stream(response),
                    response_session_id,
                ))
            }
            Some(value) if value.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
                let body = read_body_bounded(response).await.map_err(client_error)?;
                match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                    Ok(message) => Ok(StreamableHttpPostResponse::Json(
                        message,
                        response_session_id,
                    )),
                    Err(_) => Ok(StreamableHttpPostResponse::Accepted),
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn local_response(headers: &str, body: &[u8]) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let headers = headers.to_string();
        let body = body.to_vec();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(format!("HTTP/1.1 200 OK\r\n{headers}\r\n").as_bytes())
                .await
                .unwrap();
            stream.write_all(&body).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap()
    }

    #[test]
    fn sse_budget_rejects_one_oversized_event_before_parsing() {
        let mut budget = SseFrameBudget::default();
        let oversized = vec![b'x'; MAX_INBOUND_MESSAGE_BYTES + 1];
        assert!(budget.observe(&oversized).is_err());
    }

    #[test]
    fn sse_budget_resets_on_lf_and_crlf_event_boundaries() {
        let mut budget = SseFrameBudget::default();
        let event = vec![b'x'; MAX_INBOUND_MESSAGE_BYTES / 2];
        budget.observe(&event).unwrap();
        budget.observe(b"\n\n").unwrap();
        budget.observe(&event).unwrap();
        budget.observe(b"\r\n\r\n").unwrap();
        budget.observe(&event).unwrap();
    }

    #[tokio::test]
    async fn json_body_limit_checks_declared_and_streamed_sizes() {
        let declared = local_response("Content-Length: 17\r\n", b"0123456789abcdefg").await;
        assert!(read_body_with_limit(declared, 16).await.is_err());

        let chunked = local_response(
            "Transfer-Encoding: chunked\r\n",
            b"11\r\n0123456789abcdefg\r\n0\r\n\r\n",
        )
        .await;
        assert!(read_body_with_limit(chunked, 16).await.is_err());
    }
}
