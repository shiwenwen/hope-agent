//! ACP Control Plane — Health checks and diagnostics.

use super::types::AcpHealthStatus;

/// Build a health status from binary probing results.
pub fn build_health_status(
    available: bool,
    binary_path: Option<String>,
    version: Option<String>,
    error: Option<String>,
) -> AcpHealthStatus {
    AcpHealthStatus {
        available,
        binary_path,
        version,
        error,
        last_checked: chrono::Utc::now().to_rfc3339(),
    }
}

/// Check if a binary is executable by running `<binary> --version`.
pub async fn probe_binary(binary_path: &str) -> AcpHealthStatus {
    match tokio::process::Command::new(binary_path)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            match tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
                .await
            {
                Ok(Ok(output)) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let version_text = if stdout.is_empty() { &stderr } else { &stdout };
                    // Try to extract a version-like substring
                    let version = extract_version(version_text);
                    build_health_status(
                        output.status.success(),
                        Some(binary_path.to_string()),
                        version,
                        if output.status.success() {
                            None
                        } else {
                            Some(format!("Exit code {}", output.status.code().unwrap_or(-1)))
                        },
                    )
                }
                Ok(Err(e)) => build_health_status(
                    false,
                    Some(binary_path.to_string()),
                    None,
                    Some(format!("Failed to run: {}", e)),
                ),
                Err(_) => build_health_status(
                    false,
                    Some(binary_path.to_string()),
                    None,
                    Some("Timed out after 10s".into()),
                ),
            }
        }
        Err(e) => build_health_status(
            false,
            Some(binary_path.to_string()),
            None,
            Some(format!("Failed to spawn: {}", e)),
        ),
    }
}

/// Extract a semver-like version from a string (e.g. "claude v1.2.3" → "1.2.3").
fn extract_version(text: &str) -> Option<String> {
    // Look for patterns like X.Y.Z, vX.Y.Z, or X.Y
    let re = regex::Regex::new(r"v?(\d+\.\d+(?:\.\d+)?)").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}
