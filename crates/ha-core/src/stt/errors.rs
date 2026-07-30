//! Classified error kinds for the STT subsystem.

use std::fmt;

#[derive(Debug)]
pub enum SttError {
    /// Provider lookup failed (provider_id / model_id missing or disabled).
    NotFound(String),
    /// No STT model is configured for the requested path (active / im fallback).
    NoActiveModel,
    /// Provider/request configuration is incomplete or malformed.
    Config(String),
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
    /// Hard input/configuration errors (`UnsupportedAudio`, `Config`) are not
    /// retriable because the same request would fail again.
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
            Self::Config(_) => "config",
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
    /// Renders as `stt:<code>: <message>` so callers across the Tauri /
    /// HTTP boundary (where the typed enum collapses into a string) can
    /// still recover the stable `code()` via a `stt:<code>:` prefix split.
    /// Keeps `code()` as the source of truth — `Display` derives from it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        let body = match self {
            Self::NotFound(id) => format!("STT provider/model not found: {id}"),
            Self::NoActiveModel => "No STT model configured".into(),
            Self::Config(msg) => format!("STT configuration error: {msg}"),
            Self::Auth(msg) => format!("STT auth failure: {msg}"),
            Self::RateLimit(msg) => format!("STT rate-limited: {msg}"),
            Self::Network(msg) => format!("STT network error: {msg}"),
            Self::UnsupportedAudio(msg) => format!("Unsupported audio: {msg}"),
            Self::ProviderUnavailable(msg) => format!("STT provider unavailable: {msg}"),
            Self::SsrfBlocked(msg) => format!("Destination blocked by SSRF policy: {msg}"),
            Self::Io(msg) => format!("STT I/O error: {msg}"),
            Self::Other(msg) => format!("STT error: {msg}"),
        };
        write!(f, "stt:{code}: {body}")
    }
}

impl std::error::Error for SttError {}

impl From<std::io::Error> for SttError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

pub type SttResult<T> = Result<T, SttError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_errors_are_stable_and_not_retriable() {
        let error = SttError::Config("language is required".into());
        assert_eq!(error.code(), "config");
        assert!(!error.is_retriable());
        assert_eq!(
            error.to_string(),
            "stt:config: STT configuration error: language is required"
        );
    }
}
