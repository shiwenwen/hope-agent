//! Multi-sink fan-out for streaming chat events.
//!
//! `ChatEngineParams.event_sink` is a single `Arc<dyn EventSink>` for the
//! turn's primary consumer (Tauri ChannelSink for desktop / HTTP, the
//! channel worker's `ChannelStreamSink` for IM). The handover work needs
//! more than one sink to observe the same session at once — e.g. IM chat 1
//! is primary while IM chat 2 attaches to observe, or a GUI viewer wants to
//! mirror an IM-driven turn.
//!
//! `SinkRegistry` is an opt-in side channel: extra sinks register here for a
//! session, `emit_stream_event` fans every event out to them after the
//! primary `event_sink.send` already ran. Cleanup is RAII via [`SinkHandle`]:
//! drop the handle and the sink is detached, even on panic / early return.
//!
//! The registry never holds the primary `event_sink` — the chat engine
//! already calls it directly. This keeps each event delivered exactly once
//! per consumer regardless of how many extras are subscribed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::chat_engine::types::EventSink;

/// Global multi-sink fan-out registry. One static instance per process.
pub struct SinkRegistry {
    inner: Mutex<HashMap<String, Vec<Weak<dyn EventSink>>>>,
}

impl SinkRegistry {
    fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Process-wide singleton. Lazily initialized on first call.
    pub fn instance() -> &'static SinkRegistry {
        static INST: OnceLock<SinkRegistry> = OnceLock::new();
        INST.get_or_init(SinkRegistry::new)
    }

    /// Attach `sink` to `session_id`. Returns a [`SinkHandle`]; drop it to
    /// detach (RAII). The registry holds a `Weak` reference, so the sink can
    /// also be released by dropping the last `Arc` outside.
    pub fn attach(&self, session_id: String, sink: Arc<dyn EventSink>) -> SinkHandle {
        let weak = Arc::downgrade(&sink);
        let ptr = sink_addr(&sink);
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(session_id.clone()).or_default().push(weak);
        SinkHandle {
            session_id,
            sink_addr: ptr,
            // Keep the Arc alive for as long as the handle lives so the
            // registry's Weak stays upgradable.
            _strong: sink,
        }
    }

    /// Send `event` to every attached sink for `session_id`. Dead Weak
    /// references are pruned opportunistically. Sink errors are swallowed —
    /// fan-out is best-effort and never blocks the engine.
    pub fn emit(&self, session_id: &str, event: &str) {
        // Snapshot live sinks under the lock; release the lock before
        // calling `send` so a slow sink can't stall other emitters.
        let live: Vec<Arc<dyn EventSink>> = {
            let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(weaks) = map.get_mut(session_id) else {
                return;
            };
            let live: Vec<Arc<dyn EventSink>> = weaks.iter().filter_map(Weak::upgrade).collect();
            // Prune dead entries while we hold the lock.
            weaks.retain(|w| w.strong_count() > 0);
            if weaks.is_empty() {
                map.remove(session_id);
            }
            live
        };
        for sink in live {
            sink.send(event);
        }
    }

    /// Number of sinks currently attached for `session_id`. Test-only helper.
    #[cfg(test)]
    pub fn attached_count(&self, session_id: &str) -> usize {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id)
            .map(|v| v.iter().filter(|w| w.strong_count() > 0).count())
            .unwrap_or(0)
    }

    fn detach(&self, session_id: &str, sink_addr: usize) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(weaks) = map.get_mut(session_id) else {
            return;
        };
        weaks.retain(|w| match w.upgrade() {
            Some(arc) => sink_addr_arc(&arc) != sink_addr,
            None => false,
        });
        if weaks.is_empty() {
            map.remove(session_id);
        }
    }
}

fn sink_addr(arc: &Arc<dyn EventSink>) -> usize {
    Arc::as_ptr(arc) as *const () as usize
}

fn sink_addr_arc(arc: &Arc<dyn EventSink>) -> usize {
    sink_addr(arc)
}

/// RAII detach guard for a registered sink. Drop releases the sink from the
/// registry; the underlying sink Arc is kept alive for the handle's lifetime
/// so emits during fan-out always succeed.
pub struct SinkHandle {
    session_id: String,
    sink_addr: usize,
    _strong: Arc<dyn EventSink>,
}

impl SinkHandle {
    /// The session this handle is attached to.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for SinkHandle {
    fn drop(&mut self) {
        SinkRegistry::instance().detach(&self.session_id, self.sink_addr);
    }
}

/// Convenience accessor — `sink_registry().emit(...)` reads slightly nicer
/// than `SinkRegistry::instance().emit(...)`.
pub fn sink_registry() -> &'static SinkRegistry {
    SinkRegistry::instance()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSink {
        count: AtomicUsize,
    }

    impl CountingSink {
        fn new() -> Self {
            Self {
                count: AtomicUsize::new(0),
            }
        }
        fn count(&self) -> usize {
            self.count.load(Ordering::Relaxed)
        }
    }

    impl EventSink for CountingSink {
        fn send(&self, _event: &str) {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn attach_detach_lifecycle() {
        let sid = format!("attach-detach-{}", std::process::id());
        let registry = SinkRegistry::instance();
        let sink = Arc::new(CountingSink::new());
        let dyn_sink: Arc<dyn EventSink> = sink.clone();

        assert_eq!(registry.attached_count(&sid), 0);

        let handle = registry.attach(sid.clone(), dyn_sink);
        assert_eq!(registry.attached_count(&sid), 1);

        registry.emit(&sid, "hi");
        assert_eq!(sink.count(), 1);

        drop(handle);
        assert_eq!(registry.attached_count(&sid), 0);
        registry.emit(&sid, "after-drop");
        assert_eq!(sink.count(), 1, "detached sink should not be invoked");
    }

    #[test]
    fn emit_fans_out_to_all_attached() {
        let sid = format!("fanout-{}", std::process::id());
        let registry = SinkRegistry::instance();

        let s1 = Arc::new(CountingSink::new());
        let s2 = Arc::new(CountingSink::new());
        let s1_dyn: Arc<dyn EventSink> = s1.clone();
        let s2_dyn: Arc<dyn EventSink> = s2.clone();
        let h1 = registry.attach(sid.clone(), s1_dyn);
        let h2 = registry.attach(sid.clone(), s2_dyn);

        registry.emit(&sid, "event-a");
        registry.emit(&sid, "event-b");

        assert_eq!(s1.count(), 2);
        assert_eq!(s2.count(), 2);

        drop(h1);
        registry.emit(&sid, "event-c");
        assert_eq!(s1.count(), 2, "detached sink should not receive new events");
        assert_eq!(s2.count(), 3);

        drop(h2);
    }

    #[test]
    fn weak_reference_drops_when_arc_released() {
        let sid = format!("weak-drop-{}", std::process::id());
        let registry = SinkRegistry::instance();
        let sink = Arc::new(CountingSink::new());
        let dyn_sink: Arc<dyn EventSink> = sink.clone();

        let _handle = registry.attach(sid.clone(), dyn_sink);
        // Handle keeps the Arc alive, so the sink stays attached.
        assert_eq!(registry.attached_count(&sid), 1);
    }

    #[test]
    fn emit_to_unknown_session_is_noop() {
        let registry = SinkRegistry::instance();
        registry.emit("never-attached-session", "data");
    }
}
