//! Zero-cost black-box smoke harness for the live-model evaluation path.
//!
//! The fake Provider speaks the production OpenAI Chat SSE protocol. The
//! smoke runner still launches a real `hope-agent-server`, uses the normal
//! HTTP chat endpoint, dispatches real tools, closes a durable Goal, and lets
//! the normal model-evidence verifier inspect the resulting trace.

use anyhow::{bail, Context, Result};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use ha_core::config::AppConfig;
use ha_core::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub const PROVIDER_ID: &str = "eval-anchor";
pub const MODEL_ID: &str = "configured-anchor-v1";
pub const FAKE_PROVIDER_KEY: &str = "fake-provider-smoke-key";
pub const FAKE_SERVER_TOKEN: &str = "fake-smoke-server-token-00000001";

#[derive(Clone)]
struct FakeProviderState {
    expected_model: Arc<str>,
    result_content: Arc<str>,
}

pub struct FakeProvider {
    pub base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl FakeProvider {
    pub async fn start(result_content: String) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("binding fake Provider")?;
        let address = listener.local_addr()?;
        let state = FakeProviderState {
            expected_model: Arc::from(MODEL_ID),
            result_content: Arc::from(result_content),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(fake_chat_completions))
            .with_state(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
            if let Err(error) = result {
                eprintln!("fake Provider stopped with error: {error}");
            }
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            shutdown: Some(shutdown_tx),
            task: Some(task),
        })
    }

    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub struct HopeServer {
    child: Child,
    pub base_url: String,
}

impl Drop for HopeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn write_smoke_config(data_dir: &Path, provider_base_url: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("creating fake Provider data dir {}", data_dir.display()))?;
    ha_eval_spec::write_json(
        &data_dir.join("config.json"),
        &smoke_config(provider_base_url),
    )
}

pub fn smoke_config(provider_base_url: &str) -> AppConfig {
    let mut provider = ProviderConfig::new(
        "Evaluation fake Provider".to_string(),
        ApiType::OpenaiChat,
        provider_base_url.to_string(),
        String::new(),
    );
    provider.id = PROVIDER_ID.to_string();
    provider.allow_private_network = true;
    provider.models.push(ModelConfig {
        id: MODEL_ID.to_string(),
        name: "Evaluation fake model".to_string(),
        input_types: vec!["text".to_string()],
        context_window: 64_000,
        max_tokens: 4_096,
        reasoning: false,
        thinking_style: None,
        // Synthetic non-zero pricing exercises exact cost attribution and
        // runtime budget enforcement without incurring any real API charge.
        // It is deliberately scoped to this loopback-only fake Provider.
        cost_input: Some(1.0),
        cost_output: Some(1.0),
    });
    AppConfig {
        providers: vec![provider],
        active_model: Some(ActiveModel {
            provider_id: PROVIDER_ID.to_string(),
            model_id: MODEL_ID.to_string(),
        }),
        ..AppConfig::default()
    }
}

pub fn provider_secrets_b64() -> Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(
        serde_json::to_vec(&json!({ (PROVIDER_ID): FAKE_PROVIDER_KEY }))
            .context("encoding fake Provider secret bundle")?,
    ))
}

pub fn spawn_hope_server(server_bin: &Path, data_dir: &Path) -> Result<HopeServer> {
    let server_bin = server_bin
        .canonicalize()
        .with_context(|| format!("canonicalizing Hope server {}", server_bin.display()))?;
    if server_bin.file_name().and_then(|name| name.to_str()) != Some("hope-agent-server") {
        bail!("fake Provider smoke requires the hope-agent-server binary");
    }
    let port = reserve_loopback_port()?;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let secret_bundle = provider_secrets_b64()?;
    let child = Command::new(server_bin)
        .args(["server", "start", "--bind", &address.to_string()])
        .env("HA_DATA_DIR", data_dir)
        .env("HA_MODEL_EVAL_MODE", "1")
        .env("HA_MODEL_EVAL_PROVIDER_SECRETS_B64", secret_bundle)
        .env("HA_MODEL_EVAL_SERVER_TOKEN", FAKE_SERVER_TOKEN)
        .env("HA_SERVER_AUTO_APPROVE_TOOLS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawning Hope server for fake Provider smoke")?;
    Ok(HopeServer {
        child,
        base_url: format!("http://{address}"),
    })
}

pub async fn wait_healthy(server: &mut HopeServer) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()?;
    for _ in 0..120 {
        if let Some(status) = server.child.try_wait()? {
            bail!("Hope smoke server exited during startup with {status}");
        }
        if client
            .get(format!("{}/api/health", server.base_url))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("Hope smoke server did not become healthy within 30 seconds")
}

fn reserve_loopback_port() -> Result<u16> {
    let listener =
        StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("reserving Hope smoke port")?;
    Ok(listener.local_addr()?.port())
}

async fn fake_chat_completions(
    State(state): State<FakeProviderState>,
    Json(request): Json<Value>,
) -> Response {
    if request.get("stream").and_then(Value::as_bool) != Some(true) {
        return match fake_non_stream_response(&state, &request) {
            Ok(body) => (StatusCode::OK, Json(body)).into_response(),
            Err(error) => (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": error.to_string()}})),
            )
                .into_response(),
        };
    }
    match fake_chat_response(&state, &request) {
        Ok(body) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream; charset=utf-8"),
            );
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            (StatusCode::OK, headers, body).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": error.to_string()}})),
        )
            .into_response(),
    }
}

fn fake_non_stream_response(state: &FakeProviderState, request: &Value) -> Result<Value> {
    if request.get("model").and_then(Value::as_str) != Some(state.expected_model.as_ref()) {
        bail!("fake Provider received an unexpected model");
    }
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("fake Provider request has no messages"))?;
    let joined = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let content = if joined.contains("independent semantic grader for a durable Goal") {
        json!({
            "summary": "No semantic criteria require model judgment; deterministic artifact evidence is sufficient.",
            "criteria": [],
            "nextActions": []
        })
        .to_string()
    } else {
        "Synthetic helper response for the isolated fake-provider smoke.".to_string()
    };
    Ok(json!({
        "id": "chatcmpl_hope_eval_smoke_helper",
        "object": "chat.completion",
        "created": 1,
        "model": state.expected_model.as_ref(),
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 32, "completion_tokens": 12, "total_tokens": 44}
    }))
}

fn fake_chat_response(state: &FakeProviderState, request: &Value) -> Result<String> {
    if request.get("model").and_then(Value::as_str) != Some(state.expected_model.as_ref()) {
        bail!("fake Provider received an unexpected model");
    }
    if request.get("stream").and_then(Value::as_bool) != Some(true) {
        bail!("fake Provider smoke requires streaming Chat Completions");
    }
    let seen = observed_tool_names(request);
    let delta = if !seen.contains("read") {
        tool_delta(vec![(
            "smoke_read",
            "read",
            json!({"path": "fixtures/task-state.json"}),
        )])
    } else if !seen.contains("write") {
        tool_delta(vec![(
            "smoke_write",
            "write",
            json!({
                "path": "deliverables/result.json",
                "content": state.result_content.as_ref()
            }),
        )])
    } else if !seen.contains("task_create") {
        tool_delta(vec![(
            "smoke_task_create",
            "task_create",
            json!({
                "tasks": [{
                    "content": "Create and verify deliverables/result.json for HA-GL-001",
                    "activeForm": "Verifying the HA-GL-001 result artifact"
                }]
            }),
        )])
    } else if !seen.contains("task_update") {
        tool_delta(vec![(
            "smoke_task_update",
            "task_update",
            json!({
                "id": created_task_id(request).unwrap_or(1),
                "status": "completed"
            }),
        )])
    } else if !seen.contains("goal_prepare_contract") {
        tool_delta(vec![
            (
                "smoke_goal_contract",
                "goal_prepare_contract",
                json!({
                    "criteria": [{
                        "id": "criterion-1",
                        "text": "Create and verify deliverables/result.json for HA-GL-001",
                        "kind": "required",
                        "checkKind": "artifact",
                        "expectedEvidence": ["artifact_reviewed"]
                    }],
                    "scopeRationale": "The single artifact and its durable evidence fully cover the synthetic smoke objective.",
                    "requiredTools": ["read", "write"],
                    "requiredPaths": ["fixtures/task-state.json"],
                    "requiresApproval": false,
                    "requiresNetwork": false
                }),
            ),
            ("smoke_loop_status", "loop_status", json!({})),
        ])
    } else if !seen.contains("goal_record_evidence") {
        tool_delta(vec![(
            "smoke_goal_evidence",
            "goal_record_evidence",
            json!({
                "relation": "artifact_reviewed",
                "title": "HA-GL-001 result artifact verified",
                "summary": "The synthetic result JSON was written with the expected digest and item count.",
                "sourceId": "deliverables/result.json",
                "goalCriterionId": "criterion-1",
                "metadata": {"synthetic": true}
            }),
        )])
    } else if !seen.contains("goal_evaluate") {
        tool_delta(vec![(
            "smoke_goal_evaluate",
            "goal_evaluate",
            json!({"reason": "Artifact and durable evidence are ready.", "strict": false}),
        )])
    } else if !seen.contains("goal_finish_request") {
        tool_delta(vec![(
            "smoke_goal_finish",
            "goal_finish_request",
            json!({
                "summary": "HA-GL-001 fake-provider smoke completed with verified artifact evidence.",
                "followUpItems": [],
                "remainingRisk": "None; all data and Provider responses are synthetic.",
                "strictEvaluation": false
            }),
        )])
    } else {
        json!({
            "role": "assistant",
            "content": "Fake-provider smoke completed: the artifact, Goal state, and trace were verified."
        })
    };
    let finish_reason = if delta.get("tool_calls").is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    let chunk = json!({
        "id": "chatcmpl_hope_eval_smoke",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": state.expected_model.as_ref(),
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]
    });
    let usage = json!({
        "id": "chatcmpl_hope_eval_smoke",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": state.expected_model.as_ref(),
        "choices": [],
        "usage": {"prompt_tokens": 64, "completion_tokens": 16, "total_tokens": 80}
    });
    Ok(format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::to_string(&chunk)?,
        serde_json::to_string(&usage)?
    ))
}

fn observed_tool_names(request: &Value) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    let Some(messages) = request.get("messages").and_then(Value::as_array) else {
        return names;
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for tool_call in tool_calls {
            if let Some(name) = tool_call
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                names.insert(name);
            }
        }
    }
    names
}

fn created_task_id(request: &Value) -> Option<i64> {
    let messages = request.get("messages")?.as_array()?;
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("tool")
            || message.get("tool_call_id").and_then(Value::as_str) != Some("smoke_task_create")
        {
            continue;
        }
        let value: Value = serde_json::from_str(message.get("content")?.as_str()?).ok()?;
        if let Some(id) = first_integer_id(&value) {
            return Some(id);
        }
    }
    None
}

fn first_integer_id(value: &Value) -> Option<i64> {
    match value {
        Value::Object(object) => object
            .get("id")
            .and_then(Value::as_i64)
            .or_else(|| object.values().find_map(first_integer_id)),
        Value::Array(items) => items.iter().find_map(first_integer_id),
        _ => None,
    }
}

fn tool_delta(calls: Vec<(&str, &str, Value)>) -> Value {
    let calls = calls
        .into_iter()
        .enumerate()
        .map(|(index, (id, name, arguments))| {
            json!({
                "index": index,
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments.to_string()
                }
            })
        })
        .collect::<Vec<_>>();
    json!({"role": "assistant", "tool_calls": calls})
}

pub fn default_server_bin(root: &Path) -> PathBuf {
    root.join("target").join("debug").join("hope-agent-server")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_provider_progresses_only_from_structured_assistant_tool_calls() {
        let state = FakeProviderState {
            expected_model: Arc::from(MODEL_ID),
            result_content: Arc::from("{}\n"),
        };
        let first = json!({"model": MODEL_ID, "stream": true, "messages": []});
        let first_body = fake_chat_response(&state, &first).unwrap();
        assert!(first_body.contains("\"name\":\"read\""));

        let second = json!({
            "model": MODEL_ID,
            "stream": true,
            "messages": [{
                "role": "assistant",
                "tool_calls": [{"function": {"name": "read"}}]
            }]
        });
        let second_body = fake_chat_response(&state, &second).unwrap();
        assert!(second_body.contains("\"name\":\"write\""));

        let grader = json!({
            "model": MODEL_ID,
            "messages": [{"role": "user", "content": "You are the independent semantic grader for a durable Goal"}]
        });
        let grader_body = fake_non_stream_response(&state, &grader).unwrap();
        let content = grader_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap();
        let parsed: Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["criteria"], json!([]));
    }
}
