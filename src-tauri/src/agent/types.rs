use serde::{Deserialize, Serialize};

use crate::provider::ThinkingStyle;

/// File/image attachment sent alongside a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime_type: String,
    /// Base64-encoded file data (used for images — passed directly through IPC)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Absolute path to the file on disk (used for non-image files)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

impl Attachment {
    /// Get base64-encoded data: use `data` field if present, otherwise read from `file_path`.
    pub(super) fn get_base64_data(&self) -> anyhow::Result<String> {
        if let Some(ref data) = self.data {
            return Ok(data.clone());
        }
        if let Some(ref path) = self.file_path {
            return read_and_encode_base64(path);
        }
        Err(anyhow::anyhow!("Attachment '{}' has neither data nor file_path", self.name))
    }
}

/// Read a file from disk and return its contents as a base64-encoded string.
pub(super) fn read_and_encode_base64(path: &str) -> anyhow::Result<String> {
    let data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("Failed to read attachment '{}': {}", path, e))?;
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&data))
}

/// Supported LLM providers
pub enum LlmProvider {
    /// Anthropic Messages API
    Anthropic { api_key: String, base_url: String, model: String },
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenAIChat { api_key: String, base_url: String, model: String },
    /// OpenAI Responses API (/v1/responses)
    OpenAIResponses { api_key: String, base_url: String, model: String },
    /// Built-in Codex OAuth (ChatGPT subscription)
    Codex { access_token: String, account_id: String, model: String },
}

pub struct AssistantAgent {
    pub(super) provider: LlmProvider,
    /// Custom User-Agent header for API requests
    pub(super) user_agent: String,
    /// Thinking/reasoning parameter format
    pub(super) thinking_style: ThinkingStyle,
    /// Conversation history persisted across chat() calls
    pub(super) conversation_history: std::sync::Mutex<Vec<serde_json::Value>>,
    /// Current agent ID (for memory context loading)
    pub(super) agent_id: String,
    /// Extra context appended to the system prompt (e.g. cron execution context)
    pub(super) extra_system_context: Option<String>,
    /// Model context window size in tokens
    pub(super) context_window: u32,
    /// Context compaction configuration
    pub(super) compact_config: crate::context_compact::CompactConfig,
    /// Token estimate calibrator (updated with actual API usage)
    pub(super) token_calibrator: std::sync::Mutex<crate::context_compact::TokenEstimateCalibrator>,
    /// Whether this agent can use the send_notification tool
    pub(super) notification_enabled: bool,
    /// Current session ID (for sub-agent context)
    pub(super) session_id: Option<String>,
    /// Sub-agent nesting depth (0 = top-level)
    pub(super) subagent_depth: u32,
}

/// Stateful filter that strips `<think>...</think>` tags from streaming content.
/// Content inside tags is redirected to thinking output; content outside goes to text output.
pub(super) struct ThinkTagFilter {
    in_thinking: bool,
    /// Buffer for potential partial tag at the end of a chunk (e.g. "<", "<th", "</thi")
    tag_buffer: String,
}

impl ThinkTagFilter {
    pub(super) fn new() -> Self {
        Self { in_thinking: false, tag_buffer: String::new() }
    }

    /// Process a chunk of content text. Returns (text_outside_tags, thinking_inside_tags).
    pub(super) fn process(&mut self, input: &str) -> (String, String) {
        let mut text_out = String::new();
        let mut think_out = String::new();

        // Prepend any buffered partial tag
        let full_input = if self.tag_buffer.is_empty() {
            input.to_string()
        } else {
            let mut s = std::mem::take(&mut self.tag_buffer);
            s.push_str(input);
            s
        };

        let mut chars = full_input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '<' {
                // Collect potential tag
                let mut tag = String::from('<');
                while let Some(&next) = chars.peek() {
                    tag.push(next);
                    chars.next();
                    if next == '>' {
                        break;
                    }
                }

                if !tag.ends_with('>') {
                    // Incomplete tag at end of chunk — buffer it
                    self.tag_buffer = tag;
                    continue;
                }

                let tag_lower = tag.to_lowercase();
                let tag_trimmed = tag_lower.trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace());
                if tag_trimmed == "think" || tag_trimmed == "thinking" {
                    self.in_thinking = true;
                } else if tag_trimmed == "/think" || tag_trimmed == "/thinking" {
                    self.in_thinking = false;
                } else {
                    // Not a think tag — emit as content
                    if self.in_thinking {
                        think_out.push_str(&tag);
                    } else {
                        text_out.push_str(&tag);
                    }
                }
            } else if self.in_thinking {
                think_out.push(ch);
            } else {
                text_out.push(ch);
            }
        }

        (text_out, think_out)
    }
}

/// Token usage accumulated across tool rounds
#[derive(Debug, Clone, Default)]
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

// ── Codex model definitions ───────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct CodexModel {
    pub id: String,
    pub name: String,
}
