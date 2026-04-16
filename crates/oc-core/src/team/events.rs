use serde::Serialize;

/// Emit a team event via the global EventBus.
pub fn emit_team_event<T: Serialize>(event_type: &str, payload: &T) {
    if let Some(bus) = crate::globals::get_event_bus() {
        if let Ok(value) = serde_json::to_value(payload) {
            let event = serde_json::json!({
                "type": event_type,
                "payload": value,
            });
            bus.emit("team_event", event);
        }
    }
}
