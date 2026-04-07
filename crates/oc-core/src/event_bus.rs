use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// A named event with a JSON payload, broadcast to all subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEvent {
    pub name: String,
    pub payload: serde_json::Value,
}

/// Transport-agnostic event bus for broadcasting backend events to frontends.
pub trait EventBus: Send + Sync + 'static {
    /// Publish an event to all subscribers.
    fn emit(&self, name: &str, payload: serde_json::Value);

    /// Subscribe to events. Returns a receiver (drop to unsubscribe).
    fn subscribe(&self) -> broadcast::Receiver<AppEvent>;
}

/// Default implementation using tokio broadcast channel.
pub struct BroadcastEventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl BroadcastEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
}

impl EventBus for BroadcastEventBus {
    fn emit(&self, name: &str, payload: serde_json::Value) {
        let _ = self.tx.send(AppEvent {
            name: name.to_string(),
            payload,
        });
    }

    fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}
