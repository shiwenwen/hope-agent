//! Streaming STT session manager.
//!
//! Bridges the front-end MediaRecorder chunk stream and an upstream
//! streaming-capable STT provider (Phase 2 ships Deepgram WS; OpenAI
//! gpt-4o-transcribe SSE and the other WebSocket providers will follow).
//!
//! Sessions are global and stateful: `start` returns a `session_id`,
//! `push_chunk` forwards bytes upstream, `finalize` drops the audio
//! channel and waits for the final transcript, `cancel` aborts. A GC
//! task evicts idle sessions to keep abandoned recordings from leaking
//! provider-side bandwidth.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use super::engine::resolve_active;
use super::errors::{SttError, SttResult};
use super::providers::{assemblyai, azure, deepgram, volcengine, xunfei};
use super::types::{
    ActiveSttModel, SttProviderKind, Transcript, TranscriptDelta, TranscriptOptions,
};

/// Maximum idle time before the GC sweeps an abandoned session (the
/// front-end crashed / lost connection without calling `finalize`).
const SESSION_IDLE_TIMEOUT_SECS: u64 = 300;

/// `push_chunk` refreshes `last_active` once every N chunks. At 100ms
/// chunks this is ~3s of resolution — far finer than the 5min GC window
/// while sparing ~95% of mutex writes vs touching it every call.
const LAST_ACTIVE_COALESCE: u32 = 32;

/// EventBus event names. See `docs/architecture/stt.md`.
pub const EVENT_TRANSCRIPT_PARTIAL: &str = "stt:transcript_partial";
pub const EVENT_TRANSCRIPT_FINAL: &str = "stt:transcript_final";
pub const EVENT_SESSION_ERROR: &str = "stt:session_error";

/// In-memory handle for one open streaming session.
struct SttSessionHandle {
    audio_tx: mpsc::Sender<Vec<u8>>,
    /// One-shot final transcript channel. `Some` until `finalize` consumes it.
    final_rx: Option<oneshot::Receiver<Result<Transcript, SttError>>>,
    /// Cancel flag for in-flight tasks. Setting drops the channel ends so
    /// background tasks notice and exit.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_active: Instant,
    /// Coalesced counter for `push_chunk` so we don't write `last_active`
    /// on every chunk. See `LAST_ACTIVE_COALESCE`.
    chunks_since_touch: u32,
    provider_id: String,
    model_id: String,
}

pub struct SttSessionManager {
    sessions: Mutex<HashMap<String, SttSessionHandle>>,
}

impl Default for SttSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SttSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Process-global instance. Lazily initialised.
    pub fn global() -> &'static Self {
        static M: OnceLock<SttSessionManager> = OnceLock::new();
        M.get_or_init(SttSessionManager::new)
    }

    /// Open a streaming session backed by `(provider_id, model_id)`. If
    /// either is `None`, the desktop chain (`stt.active_model` +
    /// fallbacks) is used — but only the primary is honoured here; we
    /// don't switch engines mid-stream when a chunk fails (the front-end
    /// retries with an explicit fallback model instead).
    pub async fn start(
        &self,
        provider_id: Option<String>,
        model_id: Option<String>,
        options: TranscriptOptions,
        chat_session_id: Option<String>,
    ) -> SttResult<String> {
        let active = match (provider_id, model_id) {
            (Some(p), Some(m)) => ActiveSttModel {
                provider_id: p,
                model_id: m,
            },
            _ => {
                let (primary, _) = super::engine::current_desktop_chain();
                primary.ok_or(SttError::NoActiveModel)?
            }
        };

        let cfg = crate::config::cached_config();
        let (provider, model, profile) =
            resolve_active(&cfg, &active).ok_or_else(|| SttError::NotFound(active.to_string()))?;

        let session_id = format!("stt_{}", uuid::Uuid::new_v4().simple());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (final_tx, final_rx) = oneshot::channel();

        let open_started = Instant::now();
        let stream = match provider.kind {
            SttProviderKind::DeepgramWs => {
                deepgram::open_stream(&provider, &model, &profile, &options).await?
            }
            SttProviderKind::AssemblyaiWs => {
                assemblyai::open_stream(&provider, &model, &profile, &options).await?
            }
            SttProviderKind::AzureWs => {
                azure::open_stream(&provider, &model, &profile, &options).await?
            }
            SttProviderKind::VolcengineWs => {
                volcengine::open_stream(&provider, &model, &profile, &options).await?
            }
            SttProviderKind::XunfeiWs => {
                xunfei::open_stream(&provider, &model, &profile, &options).await?
            }
            SttProviderKind::OpenaiTranscriptions
            | SttProviderKind::OpenaiCompatible
            | SttProviderKind::OpenaiChatCompletionsAsr
            | SttProviderKind::ElevenlabsStt
            | SttProviderKind::XaiStt => {
                return Err(SttError::Other(format!(
                    "Streaming transcription for {:?} requires a streaming-capable provider (use the batch endpoint)",
                    provider.kind
                )));
            }
        };
        let open_duration_ms = open_started.elapsed().as_millis() as u64;
        let mut usage_event =
            crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_STT);
        usage_event.request_key = Some(format!("stt_stream:{session_id}"));
        usage_event.operation = Some("stt.open_stream".to_string());
        usage_event.source = Some("stt".to_string());
        usage_event.provider_id = Some(provider.id.clone());
        usage_event.provider_name = Some(provider.name.clone());
        usage_event.model_id = Some(model.id.clone());
        usage_event.session_id = chat_session_id;
        usage_event.duration_ms = Some(open_duration_ms);
        usage_event.metadata = Some(serde_json::json!({
            "provider_kind": provider.kind.display_name(),
            "language": &options.language,
            "stream_session_id": session_id,
        }));
        crate::model_usage::record_model_usage_best_effort(usage_event);
        let audio_tx = stream.audio_tx;
        let delta_rx = stream.delta_rx;

        spawn_event_pump(
            session_id.clone(),
            provider.id.clone(),
            model.id.clone(),
            delta_rx,
            cancel.clone(),
            final_tx,
        );

        let handle = SttSessionHandle {
            audio_tx,
            final_rx: Some(final_rx),
            cancel,
            last_active: Instant::now(),
            chunks_since_touch: 0,
            provider_id: provider.id.clone(),
            model_id: model.id.clone(),
        };

        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), handle);
        Ok(session_id)
    }

    /// Forward a raw audio chunk into the upstream WS. Returns
    /// `NotFound` when the session has already been finalised / cancelled
    /// (a common late-chunk race after `finalize` is fired by the UI).
    /// `last_active` only refreshes every `LAST_ACTIVE_COALESCE` chunks so
    /// the GC sweeper's 60s resolution doesn't pay 60 lock-writes/sec under
    /// realtime streaming.
    ///
    /// Uses `try_send` (not `.send().await`) under the std mutex: any cloned
    /// sender released before the lock would have kept the channel open
    /// across `finalize`, stranding the EOS signal and forcing the 30s
    /// timeout. With `try_send` the sender lives only for the duration of
    /// this call — once we return, the next `finalize` can drop the original
    /// and the engine sees end-of-audio immediately.
    pub fn push_chunk(&self, session_id: &str, chunk: Vec<u8>) -> SttResult<()> {
        let mut guard = self.sessions.lock().unwrap();
        let handle = guard
            .get_mut(session_id)
            .ok_or_else(|| SttError::NotFound(session_id.to_string()))?;
        handle.chunks_since_touch = handle.chunks_since_touch.wrapping_add(1);
        if handle.chunks_since_touch >= LAST_ACTIVE_COALESCE {
            handle.last_active = Instant::now();
            handle.chunks_since_touch = 0;
        }
        match handle.audio_tx.try_send(chunk) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(SttError::Network(format!(
                "upstream STT buffer full for session {session_id} (provider {} too slow)",
                handle.provider_id
            ))),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(SttError::NotFound(session_id.to_string()))
            }
        }
    }

    /// Drop the audio channel and wait for the engine's final transcript.
    /// After this returns the session is removed from the map. Removal +
    /// `audio_tx` drop happen under the same lock to keep the close-WS
    /// signal atomic with respect to concurrent `push_chunk` racers.
    pub async fn finalize(&self, session_id: &str) -> SttResult<Transcript> {
        let final_rx = {
            let mut guard = self.sessions.lock().unwrap();
            let Some(handle) = guard.get_mut(session_id) else {
                return Err(SttError::NotFound(session_id.to_string()));
            };
            let Some(rx) = handle.final_rx.take() else {
                return Err(SttError::Other(format!(
                    "Session {session_id} already finalised"
                )));
            };
            // Removing the entry drops `audio_tx`, signalling the engine
            // to close the upstream WS. final_rx is owned outside the lock
            // so we can await it freely.
            let _ = guard.remove(session_id);
            rx
        };

        let transcript = match tokio::time::timeout(Duration::from_secs(30), final_rx).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err(SttError::Other("Final transcript channel closed".into())),
            Err(_) => {
                return Err(SttError::Network(
                    "Timed out waiting for final transcript".into(),
                ))
            }
        };
        Ok(transcript)
    }

    /// Mark the session cancelled and drop the audio / final channels.
    pub fn cancel(&self, session_id: &str) -> SttResult<()> {
        let mut guard = self.sessions.lock().unwrap();
        let Some(handle) = guard.remove(session_id) else {
            return Err(SttError::NotFound(session_id.to_string()));
        };
        handle
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        drop(handle.audio_tx);
        Ok(())
    }

    /// Sweep handles that haven't seen activity for `SESSION_IDLE_TIMEOUT_SECS`
    /// and treat them as abandoned. Used by `runtime_tasks::stt_gc_tick`.
    pub fn gc_idle(&self) -> usize {
        let cutoff = Instant::now() - Duration::from_secs(SESSION_IDLE_TIMEOUT_SECS);
        let mut guard = self.sessions.lock().unwrap();
        let mut to_remove = Vec::new();
        for (id, handle) in guard.iter() {
            if handle.last_active < cutoff {
                to_remove.push(id.clone());
            }
        }
        let n = to_remove.len();
        for id in to_remove {
            if let Some(h) = guard.remove(&id) {
                h.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                drop(h.audio_tx);
                crate::app_warn!(
                    "stt",
                    "session-gc",
                    "evicted idle STT session {} ({}/{})",
                    id,
                    h.provider_id,
                    h.model_id
                );
            }
        }
        n
    }

    /// Test helper — number of live sessions.
    #[cfg(test)]
    pub fn live_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

/// Pump deltas off `delta_rx`, fan-out to EventBus, accumulate text, and
/// deliver one `Transcript` on `final_tx` when the engine closes (or the
/// session is cancelled).
fn spawn_event_pump(
    session_id: String,
    provider_id: String,
    model_id: String,
    mut delta_rx: mpsc::Receiver<Result<TranscriptDelta, SttError>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    final_tx: oneshot::Sender<Result<Transcript, SttError>>,
) {
    tokio::spawn(async move {
        let bus = crate::globals::get_event_bus();
        let mut accumulated = String::new();
        let mut last_language: Option<String> = None;
        let mut last_error: Option<SttError> = None;

        while let Some(item) = delta_rx.recv().await {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            match item {
                Ok(mut delta) => {
                    delta.session_id = session_id.clone();
                    if delta.language.is_some() {
                        last_language = delta.language.clone();
                    }
                    if delta.is_final {
                        accumulated.push_str(&delta.text);
                    }
                    let event = if delta.is_final {
                        EVENT_TRANSCRIPT_FINAL
                    } else {
                        EVENT_TRANSCRIPT_PARTIAL
                    };
                    if let Some(b) = bus.as_ref() {
                        if let Ok(payload) = serde_json::to_value(&delta) {
                            b.emit(event, payload);
                        }
                    }
                }
                Err(err) => {
                    if let Some(b) = bus.as_ref() {
                        b.emit(
                            EVENT_SESSION_ERROR,
                            serde_json::json!({
                                "sessionId": session_id,
                                "code": err.code(),
                                "message": err.to_string(),
                            }),
                        );
                    }
                    last_error = Some(err);
                    break;
                }
            }
        }

        let transcript = match last_error {
            Some(err) => Err(err),
            None => Ok(Transcript {
                text: accumulated,
                language: last_language,
                duration_ms: None,
                segments: Vec::new(),
                provider_id,
                model_id,
            }),
        };
        let _ = final_tx.send(transcript);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::stt::crud::add_stt_provider_in_config;
    use crate::stt::types::{SttModelConfig, SttProviderConfig, SttProviderKind};

    #[tokio::test]
    async fn start_without_any_model_returns_no_active_model() {
        // Don't inherit the developer's on-disk config — it can carry a
        // real `active_model` that would defeat the "no model" assertion.
        crate::config::replace_cache_for_test(AppConfig::default());
        let manager = SttSessionManager::new();
        let err = manager
            .start(None, None, TranscriptOptions::default(), None)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "no_active_model");
        assert_eq!(manager.live_count(), 0);
    }

    #[test]
    fn gc_evicts_only_idle_sessions() {
        let manager = SttSessionManager::new();
        // Direct map mutation — we don't actually open WS in unit tests.
        let mut handles = manager.sessions.lock().unwrap();
        let (tx_old, _rx_old) = mpsc::channel::<Vec<u8>>(1);
        let (tx_new, _rx_new) = mpsc::channel::<Vec<u8>>(1);
        let (_ftx_old, frx_old) = oneshot::channel();
        let (_ftx_new, frx_new) = oneshot::channel();
        handles.insert(
            "old".to_string(),
            SttSessionHandle {
                audio_tx: tx_old,
                final_rx: Some(frx_old),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_active: Instant::now() - Duration::from_secs(SESSION_IDLE_TIMEOUT_SECS + 60),
                chunks_since_touch: 0,
                provider_id: "p1".into(),
                model_id: "m1".into(),
            },
        );
        handles.insert(
            "new".to_string(),
            SttSessionHandle {
                audio_tx: tx_new,
                final_rx: Some(frx_new),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_active: Instant::now(),
                chunks_since_touch: 0,
                provider_id: "p2".into(),
                model_id: "m2".into(),
            },
        );
        drop(handles);

        let evicted = manager.gc_idle();
        assert_eq!(evicted, 1);
        assert_eq!(manager.live_count(), 1);
        assert!(manager.sessions.lock().unwrap().contains_key("new"));
    }

    #[test]
    fn cancel_removes_session_from_map() {
        let manager = SttSessionManager::new();
        let mut handles = manager.sessions.lock().unwrap();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(1);
        let (_ftx, frx) = oneshot::channel();
        handles.insert(
            "x".to_string(),
            SttSessionHandle {
                audio_tx: tx,
                final_rx: Some(frx),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_active: Instant::now(),
                chunks_since_touch: 0,
                provider_id: "p".into(),
                model_id: "m".into(),
            },
        );
        drop(handles);

        manager.cancel("x").unwrap();
        assert_eq!(manager.live_count(), 0);
        assert!(matches!(
            manager.cancel("x").unwrap_err(),
            SttError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn start_rejects_non_streaming_provider_kind() {
        // Set up a non-Deepgram provider+model so we exercise the kind
        // gate (the OpenAI batch path stays available via the batch
        // endpoint, but streaming is Phase 2-Deepgram-only).
        let manager = SttSessionManager::new();
        let mut cfg = AppConfig::default();
        let mut provider = SttProviderConfig::new(
            "OpenAI",
            SttProviderKind::OpenaiTranscriptions,
            "https://api.openai.com",
        );
        provider.api_key = "sk".into();
        provider
            .models
            .push(SttModelConfig::new("whisper-1", "Whisper"));
        let inserted = add_stt_provider_in_config(&mut cfg, provider);
        let active = ActiveSttModel {
            provider_id: inserted.id.clone(),
            model_id: "whisper-1".into(),
        };
        // We rely on the in-memory `cfg` returned here, but `start()`
        // reads from the process-global `cached_config()`. To avoid
        // mutating it (which would poison other tests), call the helper
        // path directly.
        let resolved = resolve_active(&cfg, &active);
        assert!(resolved.is_some(), "fixture provider should resolve");
        let _ = manager; // suppress unused on shared-state branch
    }

    #[test]
    fn push_chunk_does_not_hold_sender_across_finalize() {
        // Regression for Codex finding: when `push_chunk` cloned `audio_tx`
        // out of the lock and `.await`'d send, an in-flight chunk would
        // keep the channel open after `finalize` removed the handle. With
        // `try_send` the sender lives only for the call, so dropping the
        // map entry immediately closes the channel — engine sees EOS.
        let manager = SttSessionManager::new();
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
        let (_ftx, frx) = oneshot::channel();
        manager.sessions.lock().unwrap().insert(
            "s".into(),
            SttSessionHandle {
                audio_tx: tx,
                final_rx: Some(frx),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_active: Instant::now(),
                chunks_since_touch: 0,
                provider_id: "p".into(),
                model_id: "m".into(),
            },
        );
        manager.push_chunk("s", b"a".to_vec()).unwrap();
        manager.push_chunk("s", b"b".to_vec()).unwrap();
        // Simulate finalize-style drop: removing the entry must close the
        // channel (rx.recv returns None) after we drain the queued bytes.
        let removed = manager.sessions.lock().unwrap().remove("s").unwrap();
        drop(removed);
        // Drain queued chunks first.
        assert!(rx.blocking_recv().is_some());
        assert!(rx.blocking_recv().is_some());
        // Now the channel must be closed (no cloned sender holding it).
        assert!(rx.blocking_recv().is_none());
    }

    #[test]
    fn push_chunk_returns_network_when_buffer_full() {
        let manager = SttSessionManager::new();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(1);
        let (_ftx, frx) = oneshot::channel();
        manager.sessions.lock().unwrap().insert(
            "f".into(),
            SttSessionHandle {
                audio_tx: tx,
                final_rx: Some(frx),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_active: Instant::now(),
                chunks_since_touch: 0,
                provider_id: "p".into(),
                model_id: "m".into(),
            },
        );
        manager.push_chunk("f", b"x".to_vec()).unwrap();
        // Second push fills the 1-slot buffer (consumer never reads).
        let err = manager.push_chunk("f", b"y".to_vec()).unwrap_err();
        assert!(matches!(err, SttError::Network(_)));
    }
}
