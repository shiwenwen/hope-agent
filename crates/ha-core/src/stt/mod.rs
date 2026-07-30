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
pub mod session;
pub mod types;

// ── i18n helpers ─────────────────────────────────────────────────

/// Format a `[Voice transcript] {text}\n\n` prefix that gets prepended to
/// an inbound IM message when [`crate::channel::ChannelAccountConfig::auto_transcribe_voice`]
/// is on. Localised to the user's `AppConfig.language` so the LLM sees
/// natural copy in the user's preferred language.
///
/// Covers every locale shipped under `src/i18n/locales/`:
/// `ar / en / es / ja / ko / ms / pt / ru / tr / vi / zh / zh-TW`. `auto`
/// (the default) falls back to English. Unknown codes also fall back to
/// English rather than raise — this is a UX helper, not a wire contract.
pub fn voice_prefix_for_locale(locale: &str, text: &str) -> String {
    let label = match locale {
        "zh" | "zh-CN" | "zh-Hans" => "语音转录",
        "zh-TW" | "zh-Hant" => "語音轉錄",
        "ja" => "音声書き起こし",
        "ko" => "음성 전사",
        "ar" => "تفريغ صوتي",
        "es" => "Transcripción de voz",
        "pt" | "pt-BR" | "pt-PT" => "Transcrição de voz",
        "ru" => "Голосовая расшифровка",
        "tr" => "Sesli mesaj çevirisi",
        "vi" => "Phiên âm giọng nói",
        "ms" => "Transkripsi suara",
        // "en", "auto", and anything else → English
        _ => "Voice transcript",
    };
    format!("[{label}] {text}\n\n")
}

pub use crud::{
    add_stt_provider, clear_active_stt_model, delete_stt_provider, reorder_stt_providers,
    set_active_stt_model, set_im_fallback_stt_model, set_stt_default_language_async,
    set_stt_default_options, set_stt_default_options_async, set_stt_fallback_models,
    update_stt_provider, upsert_known_local_stt_provider, SttWriteError, SttWriteResult,
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
pub use session::{
    SttSessionManager, EVENT_SESSION_ERROR, EVENT_TRANSCRIPT_FINAL, EVENT_TRANSCRIPT_PARTIAL,
};
pub use types::{
    ActiveSttModel, AudioPayload, SttConfig, SttModelConfig, SttProviderConfig, SttProviderKind,
    Transcript, TranscriptDelta, TranscriptOptions, TranscriptSegment, MAX_BATCH_AUDIO_BYTES,
};

#[cfg(test)]
mod prefix_tests {
    use super::voice_prefix_for_locale;

    #[test]
    fn prefix_localises_for_every_shipped_locale() {
        for code in [
            "ar", "en", "es", "ja", "ko", "ms", "pt", "ru", "tr", "vi", "zh", "zh-TW", "auto",
        ] {
            let out = voice_prefix_for_locale(code, "你好");
            assert!(out.contains("你好"));
            assert!(out.ends_with("\n\n"));
            // Each locale renders a non-empty bracketed label.
            assert!(out.starts_with('['));
        }
    }

    #[test]
    fn unknown_locale_falls_back_to_english() {
        let out = voice_prefix_for_locale("xx-YY", "hello");
        assert!(out.starts_with("[Voice transcript] "));
        assert!(out.ends_with("hello\n\n"));
    }

    #[test]
    fn zh_variants_each_pick_correct_script() {
        assert!(voice_prefix_for_locale("zh", "x").starts_with("[语音转录]"));
        assert!(voice_prefix_for_locale("zh-Hans", "x").starts_with("[语音转录]"));
        assert!(voice_prefix_for_locale("zh-TW", "x").starts_with("[語音轉錄]"));
        assert!(voice_prefix_for_locale("zh-Hant", "x").starts_with("[語音轉錄]"));
    }
}
