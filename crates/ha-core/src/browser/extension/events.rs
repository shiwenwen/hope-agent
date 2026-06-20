use serde::{Deserialize, Serialize};

use super::registry;
use super::{
    BrowserBackendContext, BrowserBackendRequirement, BrowserExtensionStatus,
    BrowserExtensionStatusKind,
};

pub const EVENT_BROWSER_CONTROL_STOPPED: &str = "browser:control_stopped";
pub const EVENT_BROWSER_EXTENSION_REQUIRED: &str = "browser:extension_required";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserControlStoppedReason {
    LeaseStolen,
    ManualRelease,
    Finalize,
    TurnFinalize,
    SessionCleanup,
    TabClosed,
    UserStop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserControlStoppedPayload {
    pub tab_id: i64,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub reason: BrowserControlStoppedReason,
    pub closed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_by_scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_by_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionRequiredPayload {
    pub requirement: String,
    pub reason: String,
    pub status_kind: BrowserExtensionStatusKind,
    pub status_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl BrowserControlStoppedPayload {
    pub fn for_scope(
        tab_id: i64,
        scope: String,
        reason: BrowserControlStoppedReason,
        closed: bool,
    ) -> Self {
        Self {
            tab_id,
            session_id: session_id_from_scope(&scope),
            scope,
            reason,
            closed,
            stopped_by_scope: None,
            stopped_by_session_id: None,
        }
    }

    pub fn stopped_by(mut self, stopped_by_scope: String) -> Self {
        self.stopped_by_session_id = session_id_from_scope(&stopped_by_scope);
        self.stopped_by_scope = Some(stopped_by_scope);
        self
    }
}

pub fn scope_for_context(ctx: &BrowserBackendContext) -> String {
    registry::scope_key(ctx)
}

pub fn emit_control_stopped(payload: BrowserControlStoppedPayload) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    match serde_json::to_value(&payload) {
        Ok(value) => bus.emit(EVENT_BROWSER_CONTROL_STOPPED, value),
        Err(e) => app_warn!(
            "browser",
            "extension_events",
            "Failed to serialize BrowserControlStoppedPayload: {}",
            e
        ),
    }
}

pub fn emit_control_stopped_for_scope(
    tab_id: i64,
    scope: String,
    reason: BrowserControlStoppedReason,
    closed: bool,
) {
    emit_control_stopped(BrowserControlStoppedPayload::for_scope(
        tab_id, scope, reason, closed,
    ));
}

pub fn emit_extension_required(
    ctx: &BrowserBackendContext,
    requirement: BrowserBackendRequirement,
    reason: &str,
    status: &BrowserExtensionStatus,
) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let payload = BrowserExtensionRequiredPayload {
        requirement: requirement.as_event_str().to_string(),
        reason: reason.to_string(),
        status_kind: status.kind,
        status_message: status.message.clone(),
        next_action: status.next_action.clone(),
        session_id: ctx.session_id.clone(),
        source: ctx.source.clone(),
    };
    match serde_json::to_value(&payload) {
        Ok(value) => bus.emit(EVENT_BROWSER_EXTENSION_REQUIRED, value),
        Err(e) => app_warn!(
            "browser",
            "extension_events",
            "Failed to serialize BrowserExtensionRequiredPayload: {}",
            e
        ),
    }
}

pub fn session_id_from_scope(scope: &str) -> Option<String> {
    scope
        .strip_prefix("session:")
        .filter(|session_id| !session_id.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_scope() {
        assert_eq!(
            session_id_from_scope("session:abc"),
            Some("abc".to_string())
        );
        assert_eq!(session_id_from_scope("session:"), None);
        assert_eq!(session_id_from_scope("turn:abc"), None);
        assert_eq!(session_id_from_scope("global"), None);
    }

    #[test]
    fn serializes_control_stopped_payload() {
        let payload = BrowserControlStoppedPayload::for_scope(
            42,
            "session:old".to_string(),
            BrowserControlStoppedReason::LeaseStolen,
            false,
        )
        .stopped_by("session:new".to_string());

        assert_eq!(
            serde_json::to_value(payload).unwrap(),
            serde_json::json!({
                "tabId": 42,
                "scope": "session:old",
                "sessionId": "old",
                "reason": "lease_stolen",
                "closed": false,
                "stoppedByScope": "session:new",
                "stoppedBySessionId": "new",
            })
        );
    }

    #[test]
    fn serializes_extension_required_payload() {
        let payload = BrowserExtensionRequiredPayload {
            requirement: "extension_required".to_string(),
            reason: "Chrome Extension backend is disabled".to_string(),
            status_kind: BrowserExtensionStatusKind::ExtensionDisabled,
            status_message: "disabled".to_string(),
            next_action: Some("enable_extension".to_string()),
            session_id: Some("s1".to_string()),
            source: Some("desktop".to_string()),
        };

        assert_eq!(
            serde_json::to_value(payload).unwrap(),
            serde_json::json!({
                "requirement": "extension_required",
                "reason": "Chrome Extension backend is disabled",
                "statusKind": "extension_disabled",
                "statusMessage": "disabled",
                "nextAction": "enable_extension",
                "sessionId": "s1",
                "source": "desktop",
            })
        );
    }
}
