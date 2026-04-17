use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::agent::{AssistantAgent, PlanAgentMode};
use crate::context_compact::CompactConfig;
use crate::provider::{ActiveModel, ProviderConfig};
use crate::session::SessionDB;
use crate::tools::image_generate::ImageGenConfig;

// ── Shared Types ────────────────────────────────────────────────────

/// Token usage and metrics captured from streaming callbacks.
/// See `ChatUsage` for the `input_tokens` vs `last_input_tokens` split.
#[derive(Default, Clone)]
pub(crate) struct CapturedUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub last_input_tokens: Option<i64>,
    pub model: Option<String>,
    pub ttft_ms: Option<i64>,
}

// ── EventSink trait ─────────────────────────────────────────────────

/// Abstract output layer for chat events.
/// UI chat uses a Tauri-side `ChannelSink` (in oc-tauri),
/// IM channel worker uses `ChannelStreamSink` (event bus emit).
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: &str);
}

/// EventSink for IM channel worker — pushes streaming events via the global EventBus
/// AND forwards them to a background task for progressive Telegram message editing.
pub struct ChannelStreamSink {
    pub session_id: String,
    /// Forwards raw events to the channel streaming background task.
    pub event_tx: tokio::sync::mpsc::Sender<String>,
}

impl ChannelStreamSink {
    pub fn new(session_id: String, event_tx: tokio::sync::mpsc::Sender<String>) -> Self {
        Self {
            session_id,
            event_tx,
        }
    }
}

impl EventSink for ChannelStreamSink {
    fn send(&self, event: &str) {
        // 1. Emit to frontend for real-time streaming display
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "channel:stream_delta",
                serde_json::json!({
                    "sessionId": &self.session_id,
                    "event": event,
                }),
            );
        }
        // 2. Forward to background task for progressive IM channel delivery
        let _ = self.event_tx.try_send(event.to_string());
    }
}

// ── ChatEngineParams ────────────────────────────────────────────────

/// All parameters needed by the chat engine. Callers extract these from
/// `State<AppState>` (UI chat) or disk (channel worker).
pub struct ChatEngineParams {
    // Basic
    pub session_id: String,
    pub agent_id: String,
    pub message: String,
    pub attachments: Vec<crate::agent::Attachment>,
    pub session_db: Arc<SessionDB>,

    // Model chain (pre-resolved by caller)
    pub model_chain: Vec<ActiveModel>,
    /// Provider configs needed to build agents (snapshot, not reference to State)
    pub providers: Vec<ProviderConfig>,
    /// Codex OAuth token, if available
    pub codex_token: Option<(String, String)>,

    // Agent configuration
    pub resolved_temperature: Option<f64>,
    pub web_search_enabled: bool,
    pub notification_enabled: bool,
    pub image_gen_config: Option<ImageGenConfig>,
    pub canvas_enabled: bool,
    pub compact_config: CompactConfig,

    // Optional
    pub extra_system_context: Option<String>,
    pub reasoning_effort: Option<String>,
    pub cancel: Arc<AtomicBool>,
    /// Plan Mode agent configuration (set by chat command, None for channel worker)
    pub plan_agent_mode: Option<PlanAgentMode>,
    pub plan_mode_allow_paths: Option<Vec<String>>,
    /// Skill-level tool restriction (set when a skill with `allowed-tools` is activated)
    pub skill_allowed_tools: Vec<String>,

    /// When true, all tool calls are auto-approved (IM channel auto-approve mode).
    pub auto_approve_tools: bool,

    // Output
    pub event_sink: Arc<dyn EventSink>,
}

/// Result returned by the chat engine.
pub struct ChatEngineResult {
    pub response: String,
    /// The model that produced the successful response.
    pub model_used: Option<ActiveModel>,
    /// The agent instance after chat (for UI chat to update State).
    pub agent: Option<AssistantAgent>,
}
