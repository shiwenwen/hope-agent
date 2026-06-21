use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::broker::EXPECTED_EXTENSION_PROTOCOL_VERSION;
use super::{BrowserExtensionBroker, DEFAULT_NATIVE_HOST_NAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserExtensionStatusKind {
    Ready,
    BrokerUnavailable,
    HostMissing,
    HostInvalid,
    ExtensionMissing,
    ExtensionDisabled,
    ExtensionProfileMismatch,
    PolicyBlocked,
    VersionMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionStatus {
    pub kind: BrowserExtensionStatusKind,
    pub backend_available: bool,
    pub native_host_name: String,
    pub native_host_manifest_path: Option<String>,
    pub native_host_manifest_exists: bool,
    pub extension_connected: bool,
    pub extension_protocol_version: Option<u32>,
    pub extension_version: Option<String>,
    pub extension_ids: Vec<String>,
    pub store_url: Option<String>,
    pub unpacked_extension_path: Option<String>,
    pub native_host_binary_hint: Option<String>,
    pub message: String,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostInstallRequest {
    pub extension_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_host_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostInstallResult {
    pub native_host_name: String,
    pub host_path: String,
    pub manifest_path: String,
    pub allowed_origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_registry_key: Option<String>,
}

pub fn current_status() -> BrowserExtensionStatus {
    let cfg = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.extension.clone())
        .unwrap_or_default();
    let native_host_name = cfg.native_host_name().to_string();
    let manifest_path = native_host_manifest_path(&native_host_name);
    let manifest_exists = manifest_path.as_ref().is_some_and(|p| p.exists());
    let broker_status = BrowserExtensionBroker::global().map(|broker| broker.status_snapshot());
    let broker_running = broker_status.as_ref().is_some_and(|s| s.running);
    let extension_connected = broker_status
        .as_ref()
        .is_some_and(|s| s.extension_connected);

    let protocol_version = broker_status
        .as_ref()
        .and_then(|s| s.extension_protocol_version);
    let extension_version = broker_status
        .as_ref()
        .and_then(|s| s.extension_version.clone());

    let (kind, message, next_action) =
        if extension_connected && protocol_version != Some(EXPECTED_EXTENSION_PROTOCOL_VERSION) {
            (
                BrowserExtensionStatusKind::VersionMismatch,
                format!(
                    "Hope Agent Chrome Extension protocol mismatch: expected {}, got {}{}.",
                    EXPECTED_EXTENSION_PROTOCOL_VERSION,
                    protocol_version
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    extension_version
                        .as_deref()
                        .map(|v| format!(" (extension {v})"))
                        .unwrap_or_default()
                ),
                Some("reload_extension".to_string()),
            )
        } else if extension_connected {
            (
                BrowserExtensionStatusKind::Ready,
                "Hope Agent Chrome Extension is connected.".to_string(),
                None,
            )
        } else if !broker_running {
            (
                BrowserExtensionStatusKind::BrokerUnavailable,
                broker_status
                    .as_ref()
                    .and_then(|s| s.last_error.clone())
                    .unwrap_or_else(|| "Chrome Extension broker is not running.".to_string()),
                Some("retry_connection".to_string()),
            )
        } else if !manifest_exists {
            (
                BrowserExtensionStatusKind::HostMissing,
                "Chrome native messaging host is not installed.".to_string(),
                Some("install_native_host".to_string()),
            )
        } else {
            (
                BrowserExtensionStatusKind::ExtensionMissing,
                "Hope Agent Chrome Extension is not connected.".to_string(),
                Some("open_extension_page".to_string()),
            )
        };

    BrowserExtensionStatus {
        kind,
        backend_available: matches!(kind, BrowserExtensionStatusKind::Ready),
        native_host_name,
        native_host_manifest_path: manifest_path.map(|p| p.to_string_lossy().to_string()),
        native_host_manifest_exists: manifest_exists,
        extension_connected,
        extension_protocol_version: protocol_version,
        extension_version,
        extension_ids: effective_extension_ids(&cfg.extension_ids),
        store_url: cfg.store_url,
        unpacked_extension_path: unpacked_extension_path().map(|p| p.to_string_lossy().to_string()),
        native_host_binary_hint: native_host_binary_hint().map(|p| p.to_string_lossy().to_string()),
        message,
        next_action,
    }
}

pub fn install_native_host_manifest(
    request: NativeHostInstallRequest,
) -> Result<NativeHostInstallResult> {
    validate_extension_id(&request.extension_id)?;
    let native_host_name = request
        .native_host_name
        .as_deref()
        .unwrap_or(DEFAULT_NATIVE_HOST_NAME);
    validate_native_host_name(native_host_name)?;

    let host_path = resolve_host_path(request.host_path)?;
    if !host_path.is_absolute() {
        bail!("Native host path must be absolute: {}", host_path.display());
    }
    if !host_path.exists() {
        bail!("Native host binary does not exist: {}", host_path.display());
    }

    let manifest_path = native_host_manifest_path(native_host_name)
        .context("Native host manifests are not supported on this platform")?;
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating native host manifest directory {}",
                parent.display()
            )
        })?;
    }

    let allowed_origin = format!("chrome-extension://{}/", request.extension_id);
    let manifest = json!({
        "name": native_host_name,
        "description": "Hope Agent Chrome Native Messaging Host",
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [allowed_origin.clone()],
    });
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    crate::platform::write_atomic(&manifest_path, &bytes).with_context(|| {
        format!(
            "writing native host manifest {}",
            manifest_path.to_string_lossy()
        )
    })?;

    let windows_registry_key = register_windows_native_host(native_host_name, &manifest_path)?;

    Ok(NativeHostInstallResult {
        native_host_name: native_host_name.to_string(),
        host_path: host_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        allowed_origin,
        windows_registry_key,
    })
}

pub fn native_host_manifest_path(host_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return dirs::home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Google")
                .join("Chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"))
        });
    }

    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join("AppData").join("Local")))?;
        return Some(
            base.join("HopeAgent")
                .join("extension")
                .join(format!("{host_name}.json")),
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return dirs::home_dir().map(|home| {
            home.join(".config")
                .join("google-chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"))
        });
    }

    #[allow(unreachable_code)]
    None
}

pub fn default_native_host_manifest_path() -> Option<PathBuf> {
    native_host_manifest_path(DEFAULT_NATIVE_HOST_NAME)
}

fn unpacked_extension_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let path = cwd.join("extensions").join("chrome");
    path.join("manifest.json").exists().then_some(path)
}

fn effective_extension_ids(configured: &[String]) -> Vec<String> {
    let mut ids = Vec::new();
    for id in configured {
        push_unique_extension_id(&mut ids, id.clone());
    }
    if let Some(id) = unpacked_extension_id() {
        push_unique_extension_id(&mut ids, id);
    }
    ids
}

fn push_unique_extension_id(ids: &mut Vec<String>, id: String) {
    if !id.trim().is_empty() && !ids.iter().any(|existing| existing == &id) {
        ids.push(id);
    }
}

fn unpacked_extension_id() -> Option<String> {
    let manifest_path = unpacked_extension_path()?.join("manifest.json");
    let bytes = std::fs::read(manifest_path).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let key = manifest.get("key").and_then(serde_json::Value::as_str)?;
    extension_id_from_manifest_key(key).ok()
}

fn extension_id_from_manifest_key(key: &str) -> Result<String> {
    let compact: String = key.chars().filter(|ch| !ch.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .context("decoding Chrome extension manifest key")?;
    let digest = Sha256::digest(&der);
    let mut id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        id.push(chrome_extension_id_char(byte >> 4));
        id.push(chrome_extension_id_char(byte & 0x0f));
    }
    Ok(id)
}

fn chrome_extension_id_char(nibble: u8) -> char {
    char::from(b'a' + (nibble & 0x0f))
}

fn native_host_binary_hint() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "ha-browser-host.exe"
    } else {
        "ha-browser-host"
    };
    native_host_binary_candidates(exe_name)
        .into_iter()
        .find(|path| path.is_file())
}

fn resolve_host_path(input: Option<String>) -> Result<PathBuf> {
    if let Some(path) = input.filter(|s| !s.trim().is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = native_host_binary_hint() {
        return Ok(path);
    }
    bail!(
        "Native host path is required. Bundle ha-browser-host with Hope Agent, pass its absolute path, or set HOPE_AGENT_BROWSER_HOST_PATH."
    );
}

fn native_host_binary_candidates(exe_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("HOPE_AGENT_BROWSER_HOST_PATH").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current) = std::env::current_exe() {
        if current
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| stem == "ha-browser-host")
        {
            candidates.push(current.clone());
        }
        if let Some(dir) = current.parent() {
            candidates.push(dir.join(exe_name));
            candidates.push(dir.join("browser-host").join(exe_name));
            candidates.push(dir.join("..").join("Resources").join(exe_name));
            candidates.push(
                dir.join("..")
                    .join("Resources")
                    .join("browser-host")
                    .join(exe_name),
            );
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(exe_name));
        candidates.push(cwd.join("target").join("release").join(exe_name));
        candidates.push(
            cwd.join("src-tauri")
                .join("resources")
                .join("browser-host")
                .join(exe_name),
        );
    }
    candidates
}

fn validate_extension_id(extension_id: &str) -> Result<()> {
    if extension_id.len() != 32 || !extension_id.chars().all(|c| ('a'..='p').contains(&c)) {
        bail!(
            "Invalid Chrome extension id '{}'. Expected 32 lowercase characters in the range a-p.",
            extension_id
        );
    }
    Ok(())
}

fn validate_native_host_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        bail!("Invalid native host name length");
    }
    if name.starts_with('.') || name.ends_with('.') || name.contains("..") {
        bail!("Invalid native host name '{}'", name);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    {
        bail!(
            "Invalid native host name '{}'. Use lowercase letters, digits, underscores, and dots only.",
            name
        );
    }
    Ok(())
}

#[cfg(windows)]
fn register_windows_native_host(
    host_name: &str,
    manifest_path: &std::path::Path,
) -> Result<Option<String>> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let subkey = format!(r"Software\Google\Chrome\NativeMessagingHosts\{host_name}");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(&subkey)
        .with_context(|| format!("creating Chrome NativeMessagingHosts registry key {subkey}"))?;
    key.set_value("", &manifest_path.to_string_lossy().to_string())
        .with_context(|| format!("writing Chrome NativeMessagingHosts registry key {subkey}"))?;
    Ok(Some(format!(r"HKEY_CURRENT_USER\{subkey}")))
}

#[cfg(not(windows))]
fn register_windows_native_host(
    _host_name: &str,
    _manifest_path: &std::path::Path,
) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_fail_closed_without_broker() {
        let status = current_status();
        assert!(!status.backend_available);
        assert!(!status.extension_connected);
    }

    #[test]
    fn default_manifest_path_uses_host_name() {
        let path = native_host_manifest_path("com.example.test").expect("manifest path");
        assert!(path.to_string_lossy().contains("com.example.test.json"));
    }

    #[test]
    fn validates_chrome_extension_ids() {
        assert!(validate_extension_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_ok());
        assert!(validate_extension_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(validate_extension_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }

    #[test]
    fn derives_chrome_extension_id_from_manifest_key() {
        let key = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA37N8lhc6y9uoV/64yn6MwtA3BSNdvnXtjybtfgVzdcklJ6E7GQf6dA1DrHHc1EU7k2dFRtLmFRWVSqIR+E+oAHWxWFLop6Q4uvgySaL5pzpgk2tSYVhrCfOKo6A2xf+DhAB9JwEaS2B30EXEX8rMuNhyBZb2aWmeF4dK4vpjzpyCtcdb5Y3Gi3RBuxiG96UFRnO8ms6GoKH/uCSYipO2c3YWm/DZbj1WxJFolCoMlXyL0/XkroM1UVTLtmuKCGV6jbz98ouHL+DeZ9l909HOmxWckcE3ffR0wSF9NPOGQk/aiSA7LXQcrw4brG4iVgrkD4NRMFwAuCjn/dsUG2cHvQIDAQAB";
        assert_eq!(
            extension_id_from_manifest_key(key).unwrap(),
            "ejafepfkhjdjopjonfgalbkelimgeeji"
        );
    }

    #[test]
    fn effective_extension_ids_appends_unpacked_id_without_duplicate() {
        let configured = vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ];
        let ids = effective_extension_ids(&configured);
        assert_eq!(ids[0], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(
            ids.iter()
                .filter(|id| id.as_str() == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .count(),
            1
        );
        if unpacked_extension_path().is_some() {
            assert!(ids
                .iter()
                .any(|id| id == "ejafepfkhjdjopjonfgalbkelimgeeji"));
        }
    }

    #[test]
    fn validates_native_host_name() {
        assert!(validate_native_host_name("com.hope_agent.chrome").is_ok());
        assert!(validate_native_host_name("Com.HopeAgent.Chrome").is_err());
        assert!(validate_native_host_name(".com.hope_agent.chrome").is_err());
        assert!(validate_native_host_name("com..hope_agent.chrome").is_err());
    }
}
