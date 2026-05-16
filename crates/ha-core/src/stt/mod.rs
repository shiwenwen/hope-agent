//! Speech-to-Text (STT) subsystem.
//!
//! Independent of the LLM provider list: separate config bucket
//! (`AppConfig.stt`), separate provider list, separate engine trait, four
//! known local backends (whisper.cpp / faster-whisper / FunASR / sherpa-onnx)
//! and a multi-protocol provider surface (OpenAI multipart, SSE, Deepgram /
//! AssemblyAI / Azure / Volcengine / iFlytek WebSockets).
//!
//! See `docs/architecture/stt.md` for the subsystem design.

pub mod crud;
pub mod engine;
pub mod errors;
pub mod local;
pub mod providers;
pub mod types;

pub use crud::{
    add_stt_provider, clear_active_stt_model, delete_stt_provider, reorder_stt_providers,
    set_active_stt_model, set_im_fallback_stt_model, set_stt_fallback_models, update_stt_provider,
    upsert_known_local_stt_provider, SttWriteError, SttWriteResult,
};
pub use engine::{
    current_desktop_chain, current_im_chain, failover_transcribe_batch, resolve_active,
    transcribe_with, AttemptedModel, FailoverError,
};
pub use errors::{SttError, SttResult};
pub use local::{
    known_local_stt_backend, known_local_stt_backend_matches, known_local_stt_backends,
    probe_local_backend_alive, KnownLocalSttBackend, FASTER_WHISPER_KEY, FUNASR_KEY,
    SHERPA_ONNX_KEY, WHISPER_CPP_KEY,
};
pub use types::{
    ActiveSttModel, AudioPayload, SttConfig, SttModelConfig, SttProviderConfig, SttProviderKind,
    Transcript, TranscriptDelta, TranscriptOptions, TranscriptSegment,
};
