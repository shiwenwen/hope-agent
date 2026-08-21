//! Read-only host toolchain diagnosis for the owner UI and support workflow.
//!
//! Probes are deliberately fixed and bounded: no shell, no installer, no
//! daemon/context mutation, no response body passthrough, and no host path in
//! the returned report. Child output is ANSI/control-cleaned and credential
//! redacted before the version parser sees it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_PROBE_OUTPUT_BYTES: u64 = 8 * 1024;
const MAX_SANITIZED_OUTPUT_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainDoctorStatus {
    Detected,
    Supported,
    Degraded,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainDoctorCheck {
    pub id: String,
    pub status: ToolchainDoctorStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub related_versions: BTreeMap<String, String>,
    pub detail_code: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainDoctorSummary {
    pub detected: usize,
    pub supported: usize,
    pub degraded: usize,
    pub blocked: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainDoctorReport {
    pub generated_at: String,
    pub platform: String,
    pub read_only: bool,
    pub checks: Vec<ToolchainDoctorCheck>,
    pub summary: ToolchainDoctorSummary,
}

#[derive(Debug)]
struct ProbeOutput {
    success: bool,
    text: String,
}

#[derive(Clone, Copy)]
struct SimpleProbe<'a> {
    id: &'a str,
    candidates: &'a [&'a str],
    args: &'a [&'a str],
    minimum: Option<&'a str>,
    missing_status: ToolchainDoctorStatus,
    prerelease_degraded: bool,
}

/// Run every host probe concurrently and return a bounded, non-sensitive
/// report. This function never installs, upgrades, starts, or reconfigures a
/// dependency.
pub async fn diagnose_toolchain() -> ToolchainDoctorReport {
    let (
        os,
        docker,
        chrome,
        ffmpeg,
        github_cli,
        ollama,
        python,
        rust_analyzer,
        typescript_language_server,
        clangd,
        libreoffice,
        poppler,
    ) = tokio::join!(
        async { diagnose_os() },
        diagnose_docker(),
        diagnose_simple(SimpleProbe {
            id: "chrome",
            candidates: chrome_candidates(),
            args: &["--version"],
            minimum: Some("151.0.7922.109"),
            missing_status: ToolchainDoctorStatus::Blocked,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "ffmpeg",
            candidates: &["ffmpeg"],
            args: &["-version"],
            minimum: Some("8.1.2"),
            missing_status: ToolchainDoctorStatus::Blocked,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "github_cli",
            candidates: &["gh"],
            args: &["--version"],
            minimum: Some("2.97.0"),
            missing_status: ToolchainDoctorStatus::Degraded,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "ollama",
            candidates: &["ollama"],
            args: &["--version"],
            minimum: Some("0.32.9"),
            missing_status: ToolchainDoctorStatus::Degraded,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "python",
            candidates: &["python3", "python"],
            args: &["--version"],
            minimum: Some("3.10.0"),
            missing_status: ToolchainDoctorStatus::Degraded,
            prerelease_degraded: true,
        }),
        diagnose_simple(SimpleProbe {
            id: "rust_analyzer",
            candidates: &["rust-analyzer"],
            args: &["--version"],
            minimum: None,
            missing_status: ToolchainDoctorStatus::Degraded,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "typescript_language_server",
            candidates: &["typescript-language-server"],
            args: &["--version"],
            minimum: None,
            missing_status: ToolchainDoctorStatus::Degraded,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "clangd",
            candidates: &["clangd"],
            args: &["--version"],
            minimum: None,
            missing_status: ToolchainDoctorStatus::Detected,
            prerelease_degraded: false,
        }),
        diagnose_simple(SimpleProbe {
            id: "libreoffice",
            candidates: &["libreoffice", "soffice"],
            args: &["--version"],
            minimum: None,
            missing_status: ToolchainDoctorStatus::Detected,
            prerelease_degraded: true,
        }),
        diagnose_simple(SimpleProbe {
            id: "poppler",
            candidates: &["pdftoppm"],
            args: &["-v"],
            minimum: None,
            missing_status: ToolchainDoctorStatus::Detected,
            prerelease_degraded: false,
        }),
    );
    let checks = vec![
        os,
        docker,
        chrome,
        ffmpeg,
        github_cli,
        ollama,
        python,
        rust_analyzer,
        typescript_language_server,
        clangd,
        libreoffice,
        poppler,
    ];
    let summary = summarize(&checks);
    ToolchainDoctorReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        platform: std::env::consts::OS.to_string(),
        read_only: true,
        checks,
        summary,
    }
}

fn diagnose_os() -> ToolchainDoctorCheck {
    let raw = crate::platform::os_version_string();
    let sanitized = sanitize_probe_output(&raw);
    let detected_version = extract_version(&sanitized);
    #[cfg(target_os = "macos")]
    let minimum = Some("26.6");
    #[cfg(not(target_os = "macos"))]
    let minimum: Option<&str> = None;
    let status = match (detected_version.as_deref(), minimum) {
        (Some(version), Some(minimum)) if version_at_least(version, minimum) => {
            ToolchainDoctorStatus::Supported
        }
        (Some(_), Some(_)) => ToolchainDoctorStatus::Degraded,
        (Some(_), None) => ToolchainDoctorStatus::Detected,
        (None, _) => ToolchainDoctorStatus::Degraded,
    };
    ToolchainDoctorCheck {
        id: "operating_system".to_string(),
        status,
        detected_version,
        minimum_version: minimum.map(str::to_string),
        related_versions: BTreeMap::new(),
        detail_code: status_detail(status, true).to_string(),
        facts: Vec::new(),
    }
}

async fn diagnose_docker() -> ToolchainDoctorCheck {
    const MINIMUM: &str = "29.7.2";
    let Some(binary) = resolve_candidate(&["docker"]) else {
        return missing_check("docker", ToolchainDoctorStatus::Blocked, Some(MINIMUM));
    };
    let mut facts = binary_facts(&binary);
    let client = run_probe(&binary, &["--version"], false).await;
    let client_version = client
        .as_ref()
        .and_then(|output| extract_version(&output.text));
    if client.as_ref().is_some_and(|output| !output.success) {
        facts.push("client_probe_failed".to_string());
    }

    let (server, context) = tokio::join!(
        run_probe(&binary, &["info", "--format", "{{.ServerVersion}}"], true),
        run_probe(&binary, &["context", "show"], true),
    );
    let server_version = server
        .as_ref()
        .filter(|output| output.success)
        .and_then(|output| extract_version(&output.text));
    if server_version.is_some() {
        facts.push("daemon_reachable".to_string());
    } else {
        facts.push("daemon_unreachable".to_string());
    }
    match context
        .as_ref()
        .filter(|output| output.success)
        .map(|output| output.text.trim())
    {
        Some("default") => facts.push("context_default".to_string()),
        Some(value) if !value.is_empty() => facts.push("context_custom".to_string()),
        _ => facts.push("context_unavailable".to_string()),
    }
    facts.extend(docker_socket_facts());
    facts.sort();
    facts.dedup();

    let mut related_versions = BTreeMap::new();
    if let Some(version) = &client_version {
        related_versions.insert("client".to_string(), version.clone());
    }
    if let Some(version) = &server_version {
        related_versions.insert("server".to_string(), version.clone());
    }
    let status = if client_version.is_none() {
        ToolchainDoctorStatus::Blocked
    } else if server_version.is_none() {
        ToolchainDoctorStatus::Blocked
    } else if client_version
        .as_deref()
        .is_some_and(|version| !version_at_least(version, MINIMUM))
        || server_version
            .as_deref()
            .is_some_and(|version| !version_at_least(version, MINIMUM))
        || facts.iter().any(|fact| fact == "socket_world_writable")
    {
        ToolchainDoctorStatus::Degraded
    } else {
        ToolchainDoctorStatus::Supported
    };
    ToolchainDoctorCheck {
        id: "docker".to_string(),
        status,
        detected_version: client_version,
        minimum_version: Some(MINIMUM.to_string()),
        related_versions,
        detail_code: status_detail(status, true).to_string(),
        facts,
    }
}

async fn diagnose_simple(spec: SimpleProbe<'_>) -> ToolchainDoctorCheck {
    let Some(binary) = resolve_candidate(spec.candidates) else {
        return missing_check(spec.id, spec.missing_status, spec.minimum);
    };
    let mut facts = binary_facts(&binary);
    let Some(output) = run_probe(&binary, spec.args, false).await else {
        facts.push("probe_timed_out".to_string());
        return ToolchainDoctorCheck {
            id: spec.id.to_string(),
            status: ToolchainDoctorStatus::Degraded,
            detected_version: None,
            minimum_version: spec.minimum.map(str::to_string),
            related_versions: BTreeMap::new(),
            detail_code: "probe_failed".to_string(),
            facts,
        };
    };
    let version = extract_version(&output.text);
    if !output.success {
        facts.push("probe_failed".to_string());
    }
    let prerelease = spec.prerelease_degraded && contains_prerelease_marker(&output.text);
    if prerelease {
        facts.push("prerelease_detected".to_string());
    }
    let status = if !output.success || version.is_none() {
        ToolchainDoctorStatus::Degraded
    } else if prerelease
        || spec.minimum.is_some_and(|minimum| {
            !version_at_least(version.as_deref().unwrap_or_default(), minimum)
        })
    {
        ToolchainDoctorStatus::Degraded
    } else if spec.minimum.is_some() {
        ToolchainDoctorStatus::Supported
    } else {
        ToolchainDoctorStatus::Detected
    };
    ToolchainDoctorCheck {
        id: spec.id.to_string(),
        status,
        detected_version: version,
        minimum_version: spec.minimum.map(str::to_string),
        related_versions: BTreeMap::new(),
        detail_code: status_detail(status, true).to_string(),
        facts,
    }
}

fn missing_check(
    id: &str,
    status: ToolchainDoctorStatus,
    minimum: Option<&str>,
) -> ToolchainDoctorCheck {
    ToolchainDoctorCheck {
        id: id.to_string(),
        status,
        detected_version: None,
        minimum_version: minimum.map(str::to_string),
        related_versions: BTreeMap::new(),
        detail_code: "not_detected".to_string(),
        facts: Vec::new(),
    }
}

fn status_detail(status: ToolchainDoctorStatus, detected: bool) -> &'static str {
    match status {
        ToolchainDoctorStatus::Detected => "detected",
        ToolchainDoctorStatus::Supported => "supported",
        ToolchainDoctorStatus::Degraded if detected => "below_recommended_or_unstable",
        ToolchainDoctorStatus::Degraded => "not_detected_optional",
        ToolchainDoctorStatus::Blocked => "feature_blocked",
    }
}

fn summarize(checks: &[ToolchainDoctorCheck]) -> ToolchainDoctorSummary {
    let mut summary = ToolchainDoctorSummary {
        detected: 0,
        supported: 0,
        degraded: 0,
        blocked: 0,
    };
    for check in checks {
        match check.status {
            ToolchainDoctorStatus::Detected => summary.detected += 1,
            ToolchainDoctorStatus::Supported => summary.supported += 1,
            ToolchainDoctorStatus::Degraded => summary.degraded += 1,
            ToolchainDoctorStatus::Blocked => summary.blocked += 1,
        }
    }
    summary
}

fn resolve_candidate(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().find_map(|candidate| {
        let path = Path::new(candidate);
        if path.is_absolute() && path.is_file() {
            Some(path.to_path_buf())
        } else {
            which::which(candidate).ok()
        }
    })
}

fn chrome_candidates() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "google-chrome",
            "chromium",
            "chrome",
        ]
    }
    #[cfg(not(target_os = "macos"))]
    {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chrome",
        ]
    }
}

async fn run_probe(path: &Path, args: &[&str], docker_env: bool) -> Option<ProbeOutput> {
    let mut command = tokio::process::Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    copy_env_if_set(&mut command, "PATH");
    #[cfg(target_os = "windows")]
    {
        copy_env_if_set(&mut command, "SystemRoot");
        copy_env_if_set(&mut command, "ComSpec");
    }
    if docker_env {
        for key in [
            "HOME",
            "USERPROFILE",
            "DOCKER_HOST",
            "DOCKER_CONTEXT",
            "DOCKER_CONFIG",
            "DOCKER_TLS_VERIFY",
            "DOCKER_CERT_PATH",
        ] {
            copy_env_if_set(&mut command, key);
        }
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?.take(MAX_PROBE_OUTPUT_BYTES);
    let mut stderr = child.stderr.take()?.take(MAX_PROBE_OUTPUT_BYTES);
    let read = async {
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let (stdout_result, stderr_result, status_result) = tokio::join!(
            stdout.read_to_end(&mut stdout_bytes),
            stderr.read_to_end(&mut stderr_bytes),
            child.wait(),
        );
        stdout_result.ok()?;
        stderr_result.ok()?;
        let status = status_result.ok()?;
        if !stderr_bytes.is_empty() {
            stdout_bytes.push(b'\n');
            stdout_bytes.extend(stderr_bytes);
        }
        let raw = String::from_utf8_lossy(&stdout_bytes);
        Some(ProbeOutput {
            success: status.success(),
            text: sanitize_probe_output(&raw),
        })
    };
    tokio::time::timeout(PROBE_TIMEOUT, read)
        .await
        .ok()
        .flatten()
}

fn copy_env_if_set(command: &mut tokio::process::Command, key: &str) {
    if let Some(value) = std::env::var_os(key) {
        command.env(key, value);
    }
}

fn sanitize_probe_output(input: &str) -> String {
    let mut clean = String::with_capacity(input.len().min(MAX_SANITIZED_OUTPUT_CHARS));
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\x07' {
                            break;
                        }
                        if next == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            continue;
        }
        clean.push(ch);
    }
    let collapsed = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    static BEARER_HEADER: OnceLock<regex::Regex> = OnceLock::new();
    let bearer_header = BEARER_HEADER.get_or_init(|| {
        regex::Regex::new(r"(?i)(authorization\s*:\s*bearer\s+)[^\s]+")
            .expect("valid bearer header regex")
    });
    let without_bearer = bearer_header.replace_all(&collapsed, "${1}[REDACTED]");
    let redacted = crate::logging::redact_sensitive(&without_bearer);
    crate::truncate_utf8(&redacted, MAX_SANITIZED_OUTPUT_CHARS).to_string()
}

fn extract_version(input: &str) -> Option<String> {
    let pattern = regex::Regex::new(r"(?i)(?:^|[^0-9])v?(\d+(?:\.\d+){1,3})(?:[^0-9]|$)")
        .expect("version regex");
    pattern
        .captures(input)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
}

fn version_at_least(actual: &str, minimum: &str) -> bool {
    let parts = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let mut actual = parts(actual);
    let mut minimum = parts(minimum);
    let len = actual.len().max(minimum.len());
    actual.resize(len, 0);
    minimum.resize(len, 0);
    actual >= minimum
}

fn contains_prerelease_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["alpha", "beta", "preview", "nightly", "snapshot", "-rc"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn binary_facts(path: &Path) -> Vec<String> {
    let mut facts = Vec::new();
    if std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        facts.push("binary_symlink".to_string());
    }
    if path.starts_with(std::env::temp_dir()) {
        facts.push("temporary_wrapper".to_string());
    }
    let text = path.to_string_lossy().to_ascii_lowercase();
    let source = if text.contains("docker.app") {
        "source_docker_desktop"
    } else if text.contains("homebrew") || text.contains("/cellar/") {
        "source_homebrew"
    } else if dirs::home_dir().is_some_and(|home| path.starts_with(home)) {
        "source_user_managed"
    } else {
        "source_system"
    };
    facts.push(source.to_string());
    facts
}

fn docker_socket_facts() -> Vec<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if let Some(host) = std::env::var_os("DOCKER_HOST") {
            let host = host.to_string_lossy();
            if !host.is_empty() && !host.starts_with("unix://") {
                return vec!["socket_remote".to_string()];
            }
            if let Some(path) = host.strip_prefix("unix://") {
                return classify_socket(Path::new(path));
            }
        }
        let mut candidates = vec![PathBuf::from("/var/run/docker.sock")];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".docker/run/docker.sock"));
            candidates.push(home.join(".colima/default/docker.sock"));
        }
        for candidate in candidates {
            if candidate.exists() {
                return classify_socket(&candidate);
            }
        }
        fn classify_socket(path: &Path) -> Vec<String> {
            let Ok(metadata) = std::fs::metadata(path) else {
                return vec!["socket_unreadable".to_string()];
            };
            let owner = if metadata.uid() == unsafe { libc::geteuid() } {
                "socket_owner_current_user"
            } else if metadata.uid() == 0 {
                "socket_owner_root"
            } else {
                "socket_owner_other"
            };
            let mut facts = vec![owner.to_string()];
            if metadata.mode() & 0o002 != 0 {
                facts.push("socket_world_writable".to_string());
            }
            facts
        }
        vec!["socket_not_detected".to_string()]
    }
    #[cfg(not(unix))]
    {
        vec!["socket_not_applicable".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_removes_terminal_controls_and_secrets() {
        let value = sanitize_probe_output(
            "\u{1b}[31mtool 2.3.4\u{1b}[0m\u{0} Authorization: Bearer sk-live-secret",
        );
        assert!(!value.contains('\u{1b}'));
        assert!(!value.contains('\u{0}'));
        assert!(!value.contains("sk-live-secret"));
        assert_eq!(extract_version(&value).as_deref(), Some("2.3.4"));
    }

    #[test]
    fn version_comparison_handles_four_component_browser_versions() {
        assert!(version_at_least("151.0.7922.169", "151.0.7922.109"));
        assert!(!version_at_least("151.0.7922.100", "151.0.7922.109"));
        assert!(version_at_least("3.10", "3.10.0"));
    }

    #[test]
    fn prerelease_markers_are_degraded_inputs() {
        assert!(contains_prerelease_marker("LibreOffice 26.8 alpha1"));
        assert!(contains_prerelease_marker("server 1.0.0-rc1"));
        assert!(!contains_prerelease_marker("server 1.0.0"));
    }
}
