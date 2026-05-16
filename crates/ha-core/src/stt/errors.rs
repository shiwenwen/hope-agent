//! Classified error kinds for the STT subsystem.

use std::fmt;

#[derive(Debug)]
pub enum SttError {
    /// Provider lookup failed (provider_id / model_id missing or disabled).
    NotFound(String),
    /// No STT model is configured for the requested path (active / im fallback).
    NoActiveModel,
    /// Authentication failed (invalid key / expired token).
    Auth(String),
    /// Provider rate-limited the request.
    RateLimit(String),
    /// Network / transport error (DNS, connect, timeout).
    Network(String),
    /// Audio payload was rejected (unsupported codec / too large / corrupt).
    UnsupportedAudio(String),
    /// Provider service unavailable / 5xx.
    ProviderUnavailable(String),
    /// SSRF policy blocked the destination.
    SsrfBlocked(String),
    /// I/O failure reading audio from disk.
    Io(String),
    /// Anything else worth surfacing without a specific class.
    Other(String),
}

impl SttError {
    /// Whether failover should try the next model on this kind of error.
    /// Hard input errors (`UnsupportedAudio`) are not retriable — the audio
    /// itself is the problem.
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::Network(_)
                | Self::RateLimit(_)
                | Self::ProviderUnavailable(_)
                | Self::Auth(_)
                | Self::Other(_)
        )
    }

    /// Stable short code for telemetry / UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::NoActiveModel => "no_active_model",
            Self::Auth(_) => "auth",
            Self::RateLimit(_) => "rate_limit",
            Self::Network(_) => "network",
            Self::UnsupportedAudio(_) => "unsupported_audio",
            Self::ProviderUnavailable(_) => "provider_unavailable",
            Self::SsrfBlocked(_) => "ssrf_blocked",
            Self::Io(_) => "io",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "STT provider/model not found: {id}"),
            Self::NoActiveModel => write!(f, "No STT model configured"),
            Self::Auth(msg) => write!(f, "STT auth failure: {msg}"),
            Self::RateLimit(msg) => write!(f, "STT rate-limited: {msg}"),
            Self::Network(msg) => write!(f, "STT network error: {msg}"),
            Self::UnsupportedAudio(msg) => write!(f, "Unsupported audio: {msg}"),
            Self::ProviderUnavailable(msg) => write!(f, "STT provider unavailable: {msg}"),
            Self::SsrfBlocked(msg) => write!(f, "Destination blocked by SSRF policy: {msg}"),
            Self::Io(msg) => write!(f, "STT I/O error: {msg}"),
            Self::Other(msg) => write!(f, "STT error: {msg}"),
        }
    }
}

impl std::error::Error for SttError {}

impl From<std::io::Error> for SttError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

pub type SttResult<T> = Result<T, SttError>;
