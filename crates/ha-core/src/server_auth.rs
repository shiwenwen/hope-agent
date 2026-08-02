//! Owner-token lifecycle for the HTTP/WS server.
//!
//! The long-lived token is a credential, not ordinary application config. New
//! writes live in `credentials/server-auth.json` (0600) and legacy
//! `config.server.api_key` values migrate there on first resolution.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CREDENTIAL_VERSION: u32 = 1;
const MAX_BROWSER_SESSION_SECS: u64 = 31 * 24 * 60 * 60;
const MAX_SCOPED_ACCESS_TICKET_SECS: u64 = 60 * 60;
type HmacSha256 = Hmac<Sha256>;
pub const API_KEY_ENV: &str = "HA_API_KEY";
pub const API_KEY_FILE_ENV: &str = "HA_API_KEY_FILE";
pub const ALLOW_UNAUTHENTICATED_NETWORK_ENV: &str = "HA_ALLOW_UNAUTHENTICATED_NETWORK";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredServerAuth {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    legacy_service_argv_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    ApiKeyFile,
    Environment,
    Credentials,
    DockerGenerated,
}

impl TokenSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKeyFile => "api-key-file",
            Self::Environment => "environment",
            Self::Credentials => "credentials",
            Self::DockerGenerated => "docker-generated",
        }
    }

    pub fn externally_managed(self) -> bool {
        matches!(self, Self::ApiKeyFile | Self::Environment)
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedToken {
    pub token: String,
    pub source: TokenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTokenMatch {
    Missing,
    Matches,
    RetiredServiceArgument,
    Differs,
}

fn validate_token(token: String, label: &str) -> Result<String> {
    let token = token.trim_end_matches(['\r', '\n']).to_string();
    if token.is_empty() {
        bail!("{label} is empty");
    }
    if token.len() > 4_096 {
        bail!("{label} exceeds the 4096-byte limit");
    }
    if token.chars().any(char::is_control) {
        bail!("{label} contains control characters");
    }
    Ok(token)
}

pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("hope_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Generate an ephemeral key for transport capabilities. Keeping this key
/// independent from an operator-chosen Owner Token prevents a disclosed
/// scoped ticket from becoming an offline password-guessing oracle.
pub fn generate_access_ticket_signing_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    key
}

pub fn token_fingerprint(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn create_browser_session(owner_token: &str, ttl_secs: u64, now: u64) -> Result<String> {
    let ttl_secs = ttl_secs.clamp(60, MAX_BROWSER_SESSION_SECS);
    let expires_at = now.saturating_add(ttl_secs);
    let mut nonce = [0u8; 18];
    rand::rng().fill_bytes(&mut nonce);
    let message = format!("v1.{expires_at}.{}", URL_SAFE_NO_PAD.encode(nonce));
    let mut mac = HmacSha256::new_from_slice(owner_token.as_bytes())
        .map_err(|_| anyhow!("invalid owner-token signing key"))?;
    mac.update(message.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{message}.{signature}"))
}

pub fn verify_browser_session(owner_token: &str, session: &str, now: u64) -> bool {
    let Some((message, signature)) = session.rsplit_once('.') else {
        return false;
    };
    let mut fields = message.split('.');
    if fields.next() != Some("v1") {
        return false;
    }
    let Some(expires_at) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
        return false;
    };
    if fields.next().is_none() || fields.next().is_some() {
        return false;
    }
    if expires_at < now || expires_at > now.saturating_add(MAX_BROWSER_SESSION_SECS) {
        return false;
    }
    let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else {
        return false;
    };
    HmacSha256::new_from_slice(owner_token.as_bytes()).is_ok_and(|mut mac| {
        mac.update(message.as_bytes());
        mac.verify_slice(&signature).is_ok()
    })
}

/// Create a short-lived capability derived from the Owner Token. Unlike the
/// root credential, these tickets are constrained to one transport surface
/// (`events` or `resources`) and cannot authorize control-plane APIs.
pub fn create_scoped_access_ticket(
    signing_key: &[u8],
    scope: &str,
    ttl_secs: u64,
    now: u64,
) -> Result<String> {
    if scope.is_empty()
        || !scope
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid access-ticket scope");
    }
    let ttl_secs = ttl_secs.clamp(60, MAX_SCOPED_ACCESS_TICKET_SECS);
    let expires_at = now.saturating_add(ttl_secs);
    let mut nonce = [0u8; 18];
    rand::rng().fill_bytes(&mut nonce);
    let message = format!("v1.{scope}.{expires_at}.{}", URL_SAFE_NO_PAD.encode(nonce));
    let mut mac = HmacSha256::new_from_slice(signing_key)
        .map_err(|_| anyhow!("invalid access-ticket signing key"))?;
    mac.update(message.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{message}.{signature}"))
}

pub fn verify_scoped_access_ticket(
    signing_key: &[u8],
    ticket: &str,
    expected_scope: &str,
    now: u64,
) -> bool {
    let Some((message, signature)) = ticket.rsplit_once('.') else {
        return false;
    };
    let mut fields = message.split('.');
    if fields.next() != Some("v1") || fields.next() != Some(expected_scope) {
        return false;
    }
    let Some(expires_at) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
        return false;
    };
    if fields.next().is_none() || fields.next().is_some() {
        return false;
    }
    if expires_at < now || expires_at > now.saturating_add(MAX_SCOPED_ACCESS_TICKET_SECS) {
        return false;
    }
    let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else {
        return false;
    };
    HmacSha256::new_from_slice(signing_key).is_ok_and(|mut mac| {
        mac.update(message.as_bytes());
        mac.verify_slice(&signature).is_ok()
    })
}

fn read_token_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let raw = String::from_utf8(bytes).with_context(|| format!("decode {}", path.display()))?;
    validate_token(raw, "server owner-token file")
}

/// Consume bootstrap secrets before runtime initialization snapshots the
/// login-shell environment. The returned token stays in process memory only;
/// tool and hook subprocesses cannot inherit the original environment value.
pub fn consume_bootstrap_token(cli_file: Option<PathBuf>) -> Result<Option<ResolvedToken>> {
    let file_path = cli_file
        .map(Into::into)
        .or_else(|| std::env::var_os(API_KEY_FILE_ENV).filter(|value| !value.is_empty()));
    std::env::remove_var(API_KEY_FILE_ENV);
    let env_token = std::env::var(API_KEY_ENV)
        .ok()
        .filter(|value| !value.is_empty());
    std::env::remove_var(API_KEY_ENV);

    if let Some(path) = file_path {
        let token = read_token_file(Path::new(&path))?;
        return Ok(Some(ResolvedToken {
            token,
            source: TokenSource::ApiKeyFile,
        }));
    }
    if let Some(token) = env_token {
        return Ok(Some(ResolvedToken {
            token: validate_token(token, API_KEY_ENV)?,
            source: TokenSource::Environment,
        }));
    }
    Ok(None)
}

fn load_credential() -> Result<Option<StoredServerAuth>> {
    let path = crate::paths::server_auth_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(anyhow!("read {}: {error}", path.display())),
    };
    let stored: StoredServerAuth =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    if stored.version != CREDENTIAL_VERSION {
        bail!(
            "unsupported server auth credential version {}",
            stored.version
        );
    }
    if let Some(token) = stored.token.as_ref() {
        validate_token(token.clone(), "stored server token")?;
    }
    if let Some(digest) = stored.legacy_service_argv_digest.as_deref() {
        let decoded = URL_SAFE_NO_PAD
            .decode(digest)
            .context("decode legacy service argv digest")?;
        if decoded.len() != 32 {
            bail!("invalid legacy service argv digest");
        }
    }
    if stored.token.is_none() && stored.legacy_service_argv_digest.is_none() {
        bail!("server auth credential contains no active token or service migration marker");
    }
    Ok(Some(stored))
}

fn write_stored_credential(stored: &StoredServerAuth) -> Result<()> {
    let path = crate::paths::server_auth_path()?;
    let bytes = serde_json::to_vec_pretty(stored).context("serialize server credentials")?;
    crate::platform::write_secure_file(&path, &bytes)
        .with_context(|| format!("write {}", path.display()))
}

fn replacement_credential(
    token: &str,
    previous: Option<&StoredServerAuth>,
) -> Result<StoredServerAuth> {
    Ok(StoredServerAuth {
        version: CREDENTIAL_VERSION,
        token: Some(validate_token(token.to_string(), "server token")?),
        created_at: chrono::Utc::now().timestamp(),
        legacy_service_argv_digest: previous
            .and_then(|stored| stored.legacy_service_argv_digest.clone()),
    })
}

fn write_credential(token: &str) -> Result<()> {
    let previous = load_credential()?;
    let stored = replacement_credential(token, previous.as_ref())?;
    write_stored_credential(&stored)
}

fn credential_without_active_token(previous: Option<StoredServerAuth>) -> Option<StoredServerAuth> {
    previous.and_then(|mut stored| {
        stored.token = None;
        if stored.legacy_service_argv_digest.is_some() {
            Some(stored)
        } else {
            None
        }
    })
}

fn clear_credential() -> Result<()> {
    let path = crate::paths::server_auth_path()?;
    if let Some(stored) = credential_without_active_token(load_credential()?) {
        return write_stored_credential(&stored);
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!("remove {}: {error}", path.display())),
    }
}

pub fn has_managed_token() -> Result<bool> {
    Ok(
        load_credential()?.is_some_and(|stored| stored.token.is_some())
            || crate::config::cached_config()
                .server
                .api_key
                .as_deref()
                .is_some_and(|token| !token.is_empty()),
    )
}

pub fn managed_token_fingerprint() -> Result<Option<String>> {
    Ok(load_managed_token()?.map(|token| token_fingerprint(&token)))
}

pub fn masked_managed_token() -> Result<Option<String>> {
    Ok(has_managed_token()?.then(|| "••••••••".to_string()))
}

fn clear_legacy_config_token() -> Result<()> {
    crate::config::clear_legacy_server_token_without_backup()?;
    crate::backup::scrub_legacy_server_tokens()
        .map_err(anyhow::Error::msg)
        .context("scrub legacy server token from config backups")
}

/// Load the credential-store token, migrating a legacy config value. The
/// credential is published before config redaction so a failed config write
/// never disables authentication; startup still fails closed and retries the
/// cleanup next time instead of leaving a forgotten plaintext duplicate.
pub fn load_managed_token() -> Result<Option<String>> {
    if let Some(token) = load_credential()?.and_then(|stored| stored.token) {
        // Always rescan backups. A prior run may have cleared live config but
        // failed part-way through historical snapshot cleanup.
        clear_legacy_config_token().context("clear legacy server token from config")?;
        return Ok(Some(token));
    }
    let legacy = crate::config::cached_config().server.api_key.clone();
    let Some(token) = legacy.filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    write_credential(&token)?;
    clear_legacy_config_token().context("clear legacy server token from config")?;
    Ok(Some(token))
}

fn token_values_match(candidate: &str, expected: &str) -> bool {
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn legacy_service_argv_digest(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

fn classify_managed_token(
    existing: Option<&str>,
    retired_service_digest: Option<&str>,
    candidate: &str,
) -> ManagedTokenMatch {
    match existing {
        Some(expected) if token_values_match(candidate, expected) => ManagedTokenMatch::Matches,
        _ if retired_service_digest.is_some_and(|expected| {
            token_values_match(&legacy_service_argv_digest(candidate), expected)
        }) =>
        {
            ManagedTokenMatch::RetiredServiceArgument
        }
        None if retired_service_digest.is_some() => ManagedTokenMatch::Differs,
        None => ManagedTokenMatch::Missing,
        Some(_) => ManagedTokenMatch::Differs,
    }
}

/// Classify a legacy service argv token without exposing the managed
/// credential. Callers must never replace a `Differs` credential with the
/// stale command-line value.
pub fn compare_managed_token(candidate: &str) -> Result<ManagedTokenMatch> {
    let existing = load_managed_token()?;
    let retired_service_digest =
        load_credential()?.and_then(|stored| stored.legacy_service_argv_digest);
    Ok(classify_managed_token(
        existing.as_deref(),
        retired_service_digest.as_deref(),
        candidate,
    ))
}

/// Remember the exact legacy service argv token after it has been imported or
/// superseded. Service managers may keep restarting a cached command after the
/// on-disk definition is cleaned, so rotations must retain this non-plaintext
/// marker until the old command disappears.
pub fn remember_legacy_service_argv_token(candidate: &str) -> Result<()> {
    let candidate = validate_token(candidate.to_string(), "legacy service argv token")?;
    let Some(mut stored) = load_credential()? else {
        bail!("cannot remember a legacy service argument without a managed token");
    };
    let digest = legacy_service_argv_digest(&candidate);
    if stored.token.is_none()
        && !stored
            .legacy_service_argv_digest
            .as_deref()
            .is_some_and(|expected| token_values_match(&digest, expected))
    {
        bail!("cannot replace a retired service argument without a managed token");
    }
    stored.legacy_service_argv_digest = Some(digest);
    write_stored_credential(&stored)
}

pub fn set_managed_token(token: Option<&str>, source: &str) -> Result<()> {
    // Complete legacy migration before the ordinary config mutation below;
    // otherwise a CLI rotation could autosave the old plaintext token just
    // before clearing the live field.
    let _ = load_managed_token()?;
    let previous = load_credential()?;
    match token {
        Some(token) if !token.is_empty() => write_credential(token)?,
        _ => clear_credential()?,
    }
    let config_result = crate::config::mutate_config(("server.auth", source), |config| {
        config.server.api_key = None;
        Ok(())
    });
    if let Err(error) = config_result {
        match previous {
            Some(previous) => write_stored_credential(&previous)?,
            None => clear_credential()?,
        }
        return Err(error);
    }
    Ok(())
}

/// Persist a Settings/HTTP update while keeping the secret out of AppConfig.
/// `api_key=None` preserves the existing credential; `Some("")` clears it.
pub fn update_server_config(
    mut next: crate::config::EmbeddedServerConfig,
    source: &str,
    external_runtime_auth: bool,
) -> Result<()> {
    let token_update = next.api_key.take();
    let _ = load_managed_token()?;
    let previous = load_credential()?;

    // Validate before touching either the credential store or config. The
    // current bind matters as well as the requested bind: changing a running
    // public listener back to loopback does not take effect until restart, so
    // clearing its token in the same save would create a temporary auth gap.
    let current = crate::config::cached_config().server.clone();
    let mut requested = next.clone().merge_over_existing(&current);
    requested.api_key = None;
    let managed_auth_after_update = match token_update.as_deref() {
        Some("") => false,
        Some(_) => true,
        None => previous
            .as_ref()
            .is_some_and(|stored| stored.token.is_some()),
    };
    let public_now_or_after_restart =
        public_binding_requires_auth(&current.bind_addr, &requested.bind_addr);
    if public_now_or_after_restart
        && !managed_auth_after_update
        && !external_runtime_auth
        && !unauthenticated_network_override_enabled()
    {
        bail!(
            "refusing to expose the server without an owner token; configure a token first or explicitly set {ALLOW_UNAUTHENTICATED_NETWORK_ENV}=1"
        );
    }

    if let Some(token) = token_update.as_deref() {
        match token {
            "" => clear_credential()?,
            value => write_credential(value)?,
        }
    }

    let result = crate::config::mutate_config(("server", source), move |config| {
        let mut merged = next.merge_over_existing(&config.server);
        merged.api_key = None;
        config.server = merged;
        Ok(())
    });
    if let Err(error) = result {
        if token_update.is_some() {
            match previous {
                Some(previous) => write_stored_credential(&previous)?,
                None => clear_credential()?,
            }
        }
        return Err(error);
    }
    Ok(())
}

pub fn ensure_docker_token() -> Result<ResolvedToken> {
    if let Some(token) = load_managed_token()? {
        return Ok(ResolvedToken {
            token,
            source: TokenSource::Credentials,
        });
    }
    let token = generate_token();
    set_managed_token(Some(&token), "docker-first-boot")?;
    Ok(ResolvedToken {
        token,
        source: TokenSource::DockerGenerated,
    })
}

pub fn rotate_managed_token(source: &str) -> Result<ResolvedToken> {
    let token = generate_token();
    set_managed_token(Some(&token), source)?;
    Ok(ResolvedToken {
        token,
        source: TokenSource::Credentials,
    })
}

pub fn resolve_effective_token(
    bootstrap: Option<ResolvedToken>,
    auto_generate_for_docker: bool,
) -> Result<Option<ResolvedToken>> {
    if bootstrap.is_some() {
        return Ok(bootstrap);
    }
    if auto_generate_for_docker {
        return ensure_docker_token().map(Some);
    }
    Ok(load_managed_token()?.map(|token| ResolvedToken {
        token,
        source: TokenSource::Credentials,
    }))
}

pub fn bind_is_loopback(bind_addr: &str) -> bool {
    if let Ok(addr) = bind_addr.parse::<SocketAddr>() {
        return addr.ip().is_loopback();
    }
    bind_addr
        .rsplit_once(':')
        .is_some_and(|(host, _)| host.eq_ignore_ascii_case("localhost"))
}

fn public_binding_requires_auth(current_bind: &str, requested_bind: &str) -> bool {
    !bind_is_loopback(current_bind) || !bind_is_loopback(requested_bind)
}

pub fn unauthenticated_network_override_enabled() -> bool {
    std::env::var(ALLOW_UNAUTHENTICATED_NETWORK_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_high_entropy_and_stably_prefixed() {
        let first = generate_token();
        let second = generate_token();
        assert!(first.starts_with("hope_"));
        assert!(first.len() >= 48);
        assert_ne!(first, second);
    }

    #[test]
    fn public_binding_requires_auth_before_and_until_restart() {
        assert!(!public_binding_requires_auth(
            "127.0.0.1:8420",
            "localhost:8420"
        ));
        assert!(public_binding_requires_auth(
            "127.0.0.1:8420",
            "0.0.0.0:8420"
        ));
        assert!(public_binding_requires_auth(
            "0.0.0.0:8420",
            "127.0.0.1:8420"
        ));
    }

    #[test]
    fn browser_sessions_are_signed_expiring_and_owner_bound() {
        let session = create_browser_session("owner-one", 3_600, 1_000).unwrap();
        assert!(verify_browser_session("owner-one", &session, 1_001));
        assert!(!verify_browser_session("owner-two", &session, 1_001));
        assert!(!verify_browser_session("owner-one", &session, 4_601));

        let mut tampered = session;
        tampered.push('x');
        assert!(!verify_browser_session("owner-one", &tampered, 1_001));
    }

    #[test]
    fn scoped_access_tickets_are_expiring_signing_key_bound_and_scope_bound() {
        let signing_key = [7u8; 32];
        let other_signing_key = [8u8; 32];
        let ticket = create_scoped_access_ticket(&signing_key, "resources", 900, 1_000).unwrap();
        assert!(verify_scoped_access_ticket(
            &signing_key,
            &ticket,
            "resources",
            1_001
        ));
        assert!(!verify_scoped_access_ticket(
            &signing_key,
            &ticket,
            "events",
            1_001
        ));
        assert!(!verify_scoped_access_ticket(
            &other_signing_key,
            &ticket,
            "resources",
            1_001
        ));
        assert!(!verify_scoped_access_ticket(
            &signing_key,
            &ticket,
            "resources",
            1_901
        ));
    }

    #[test]
    fn fingerprints_are_short_and_do_not_reveal_token_text() {
        let token = "hope_test-token-that-must-not-appear";
        let fingerprint = token_fingerprint(token);
        assert_eq!(fingerprint.len(), 12);
        assert!(!token.contains(&fingerprint));
    }

    #[test]
    fn token_comparison_rejects_wrong_values_and_lengths() {
        assert!(token_values_match("same-token", "same-token"));
        assert!(!token_values_match("same-tokee", "same-token"));
        assert!(!token_values_match("short", "longer"));
    }

    #[test]
    fn managed_token_match_distinguishes_missing_matching_and_rotated() {
        assert_eq!(
            classify_managed_token(None, None, "legacy"),
            ManagedTokenMatch::Missing
        );
        assert_eq!(
            classify_managed_token(Some("legacy"), None, "legacy"),
            ManagedTokenMatch::Matches
        );
        assert_eq!(
            classify_managed_token(
                Some("rotated"),
                Some(&legacy_service_argv_digest("legacy")),
                "legacy"
            ),
            ManagedTokenMatch::RetiredServiceArgument
        );
        assert_eq!(
            classify_managed_token(None, Some(&legacy_service_argv_digest("legacy")), "legacy"),
            ManagedTokenMatch::RetiredServiceArgument
        );
        assert_eq!(
            classify_managed_token(Some("rotated"), None, "legacy"),
            ManagedTokenMatch::Differs
        );
        assert_eq!(
            classify_managed_token(None, Some(&legacy_service_argv_digest("other")), "legacy"),
            ManagedTokenMatch::Differs
        );
    }

    #[test]
    fn credential_rotation_preserves_the_retired_service_marker() {
        let previous = StoredServerAuth {
            version: CREDENTIAL_VERSION,
            token: Some("legacy".to_string()),
            created_at: 1,
            legacy_service_argv_digest: Some(legacy_service_argv_digest("legacy")),
        };

        let rotated = replacement_credential("rotated", Some(&previous)).unwrap();

        assert_eq!(rotated.token.as_deref(), Some("rotated"));
        assert_eq!(
            rotated.legacy_service_argv_digest,
            previous.legacy_service_argv_digest
        );
    }

    #[test]
    fn clearing_authentication_preserves_only_the_retired_service_marker() {
        let previous = StoredServerAuth {
            version: CREDENTIAL_VERSION,
            token: Some("active".to_string()),
            created_at: 1,
            legacy_service_argv_digest: Some(legacy_service_argv_digest("legacy")),
        };

        let cleared = credential_without_active_token(Some(previous)).unwrap();

        assert_eq!(cleared.token, None);
        assert!(cleared.legacy_service_argv_digest.is_some());
        assert!(serde_json::to_value(&cleared)
            .unwrap()
            .get("token")
            .is_none());
        assert!(credential_without_active_token(Some(StoredServerAuth {
            version: CREDENTIAL_VERSION,
            token: Some("active".to_string()),
            created_at: 1,
            legacy_service_argv_digest: None,
        }))
        .is_none());
    }

    #[test]
    fn legacy_credential_json_deserializes_into_an_active_token() {
        let stored: StoredServerAuth = serde_json::from_value(serde_json::json!({
            "version": CREDENTIAL_VERSION,
            "token": "legacy",
            "createdAt": 1
        }))
        .unwrap();

        assert_eq!(stored.token.as_deref(), Some("legacy"));
        assert_eq!(stored.legacy_service_argv_digest, None);
    }
}
