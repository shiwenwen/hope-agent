#![cfg_attr(test, allow(clippy::needless_return))]

//! Kernel contract for owner-plane evidence quote resolution.

use std::sync::OnceLock;

use super::types::EvidenceQuote;

pub const QUOTE_MAX_CHARS: usize = 400;

#[derive(Clone, Copy)]
pub struct DreamingEvidenceRuntime {
    pub evidence_quote: fn(&str, Option<i64>) -> EvidenceQuote,
}

static RUNTIME: OnceLock<DreamingEvidenceRuntime> = OnceLock::new();

pub fn register_dreaming_evidence_runtime(
    runtime: DreamingEvidenceRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("dreaming evidence runtime"))
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_evidence.rs"]
mod test_evidence;

/// Missing feature wiring is a strict unavailable result: evidence content is
/// never guessed or surfaced through a fallback path.
pub fn evidence_quote(session_id: &str, message_id: Option<i64>) -> EvidenceQuote {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.evidence_quote)(session_id, message_id);
    }
    #[cfg(test)]
    {
        return test_evidence::evidence_quote(session_id, message_id);
    }
    #[cfg(not(test))]
    EvidenceQuote::unavailable(session_id, message_id, "runtime_unavailable")
}

impl EvidenceQuote {
    /// Build an unavailable result that cannot leak source content.
    pub fn unavailable(session_id: &str, message_id: Option<i64>, reason: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            message_id,
            role: None,
            quote: String::new(),
            truncated: false,
            available: false,
            reason: Some(reason.to_string()),
        }
    }
}
