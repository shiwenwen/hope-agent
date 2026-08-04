use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

/// Helpers for turning typed progress callbacks into EventBus emitters.
///
/// This intentionally lives on `Arc<B>` instead of `EventBus` itself so the
/// core trait stays object-safe for existing `Arc<dyn EventBus>` callers.
pub trait EventBusProgressExt {
    fn emit_progress<T>(&self, name: &'static str) -> impl Fn(&T) + Send + Sync + 'static
    where
        T: Serialize + 'static;
}

impl<B> EventBusProgressExt for Arc<B>
where
    B: EventBus + ?Sized,
{
    fn emit_progress<T>(&self, name: &'static str) -> impl Fn(&T) + Send + Sync + 'static
    where
        T: Serialize + 'static,
    {
        let bus = Arc::clone(self);
        move |progress| bus.emit(name, serde_json::json!(progress))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct TestProgress {
        step_name: String,
        percent: u8,
    }

    #[tokio::test]
    async fn emit_progress_serializes_payload_to_event_bus() {
        let bus: Arc<dyn EventBus> = Arc::new(BroadcastEventBus::new(4));
        let mut rx = bus.subscribe();
        let emit = bus.emit_progress::<TestProgress>("test:progress");

        emit(&TestProgress {
            step_name: "pull".into(),
            percent: 42,
        });

        let event = rx.recv().await.expect("progress event");
        assert_eq!(event.name, "test:progress");
        assert_eq!(
            event.payload,
            serde_json::json!({
                "stepName": "pull",
                "percent": 42,
            })
        );
    }
}
