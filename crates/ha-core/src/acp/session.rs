//! ACP session store — maps ACP session IDs to internal state.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::acp::types::ClientCapabilities;
use crate::agent::AssistantAgent;

/// A single ACP session tracked by the server
pub struct AcpSession {
    /// ACP session ID (uuid)
    pub session_id: String,
    /// Hope Agent internal session ID (in SessionDB)
    pub internal_session_id: String,
    /// Agent ID used for this session
    pub agent_id: String,
    /// Working directory set by the client
    pub cwd: Option<String>,
    /// The AssistantAgent instance for this session
    pub agent: AssistantAgent,
    /// Cancel flag for this session's active prompt
    pub cancel: Arc<AtomicBool>,
    /// Whether a prompt is currently running
    pub active_prompt: bool,
    /// Creation timestamp
    pub created_at: u64,
    /// Last activity timestamp
    pub last_activity_at: u64,
}

/// In-memory store for ACP sessions.
///
/// ACP sessions are scoped to the lifecycle of the ACP server process.
/// They map to Hope Agent sessions which are persisted in SQLite via SessionDB.
pub struct AcpSessionStore {
    sessions: HashMap<String, AcpSession>,
    /// Maximum concurrent sessions
    max_sessions: usize,
    /// Client capabilities cached from initialize
    client_capabilities: Option<ClientCapabilities>,
}

impl AcpSessionStore {
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions,
            client_capabilities: None,
        }
    }

    pub fn set_client_capabilities(&mut self, caps: ClientCapabilities) {
        self.client_capabilities = Some(caps);
    }

    pub fn client_capabilities(&self) -> Option<&ClientCapabilities> {
        self.client_capabilities.as_ref()
    }

    /// Insert a new session. Returns error if max sessions exceeded.
    pub fn insert(&mut self, session: AcpSession) -> anyhow::Result<()> {
        if self.sessions.len() >= self.max_sessions {
            // Evict oldest idle session
            let oldest = self
                .sessions
                .iter()
                .filter(|(_, s)| !s.active_prompt)
                .min_by_key(|(_, s)| s.last_activity_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.sessions.remove(&id);
            } else {
                anyhow::bail!(
                    "Maximum concurrent sessions ({}) exceeded",
                    self.max_sessions
                );
            }
        }
        self.sessions.insert(session.session_id.clone(), session);
        Ok(())
    }

    /// Get a session by ID
    pub fn get(&self, session_id: &str) -> Option<&AcpSession> {
        self.sessions.get(session_id)
    }

    /// Get a mutable session by ID
    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut AcpSession> {
        self.sessions.get_mut(session_id)
    }

    /// Remove a session by ID
    pub fn remove(&mut self, session_id: &str) -> Option<AcpSession> {
        self.sessions.remove(session_id)
    }

    /// List all sessions
    pub fn list(&self) -> Vec<&AcpSession> {
        self.sessions.values().collect()
    }

    /// Touch a session (update last_activity_at)
    pub fn touch(&mut self, session_id: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.last_activity_at = now_epoch_secs();
        }
    }

    /// Number of active sessions
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// Current epoch seconds
pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
