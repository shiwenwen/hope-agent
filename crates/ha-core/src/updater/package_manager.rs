//! Package-manager upgrade path for installs that came from `brew` /
//! `scoop` / `apt` / `dnf` / `pacman` (via the AUR build).
//!
//! Each helper builds an exact `Command` with no shell interpolation —
//! the only inputs we accept are the install source enum and (for brew)
//! the detected prefix, so we never get to compose arbitrary tokens. The
//! caller (`app_update install`) feeds the result of [`source_detector`]
//! straight in.
//!
//! The runtime command is intentionally **not** routed through the
//! `exec` tool. `exec` would re-prompt the user under non-YOLO modes,
//! which is redundant on top of the `ask_user_question` confirmation the
//! tool already showed. We invoke the package manager directly with a
//! captured stdout/stderr so the result can be surfaced cleanly via the
//! async job.

use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use super::source_detector::InstallSource;

#[derive(Debug, Clone, Serialize)]
pub struct PackageManagerOutcome {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// Run the upgrade command associated with `source`. Returns the captured
/// outcome. The caller is responsible for restarting the service after a
/// successful run.
pub fn upgrade(source: &InstallSource) -> Result<PackageManagerOutcome> {
    let mut cmd = build_command(source)
        .with_context(|| format!("no upgrade command for install source {:?}", source))?;
    let output = cmd
        .output()
        .with_context(|| format!("spawn {:?}", cmd.get_program()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok(PackageManagerOutcome {
        command: format_command(&cmd),
        stdout,
        stderr,
        success: output.status.success(),
    })
}

fn build_command(source: &InstallSource) -> Option<Command> {
    match source {
        InstallSource::Brew { prefix } => {
            // Pin `HOMEBREW_PREFIX` so the upgrade hits the same brew
            // installation the binary was loaded from — relevant when the
            // user has both `/opt/homebrew` and `/usr/local` brews.
            let mut c = Command::new(format!("{}/bin/brew", prefix.display()));
            c.env("HOMEBREW_PREFIX", prefix);
            c.args(["upgrade", "--cask", "hope-agent"]);
            Some(c)
        }
        InstallSource::Scoop => {
            // `scoop` on Windows is a Powershell shim — `platform`'s
            // `default_shell_command` already wraps `cmd /C` correctly so
            // we don't have to locate scoop.ps1 ourselves.
            Some(crate::platform::default_shell_command(
                "scoop update hope-agent",
            ))
        }
        InstallSource::Aur => {
            // AUR builds aren't in the official repo, so we can't use
            // pacman directly. The command list is hardcoded; we don't
            // accept extra args.
            let mut c = Command::new("yay");
            c.args(["-S", "--noconfirm", "hope-agent-bin"]);
            Some(c)
        }
        InstallSource::Apt => {
            // `apt update` then `install --only-upgrade` so we don't
            // accidentally pull a fresh install onto a system where the
            // package was removed mid-session.
            Some(crate::platform::default_shell_command(
                "sudo apt-get update && sudo apt-get install --only-upgrade -y hope-agent",
            ))
        }
        InstallSource::Dnf => Some(crate::platform::default_shell_command(
            "sudo dnf upgrade -y hope-agent",
        )),
        InstallSource::TauriBundle | InstallSource::Manual => None,
    }
}

fn format_command(cmd: &Command) -> String {
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    for a in cmd.get_args() {
        parts.push(a.to_string_lossy().into_owned());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn brew_command_targets_detected_prefix() {
        let cmd = build_command(&InstallSource::Brew {
            prefix: PathBuf::from("/opt/homebrew"),
        })
        .unwrap();
        let rendered = format_command(&cmd);
        assert!(
            rendered.contains("/opt/homebrew/bin/brew"),
            "rendered: {rendered}"
        );
        assert!(rendered.contains("upgrade"));
        assert!(rendered.contains("--cask"));
        assert!(rendered.contains("hope-agent"));
    }

    #[test]
    fn tauri_bundle_returns_no_command() {
        assert!(build_command(&InstallSource::TauriBundle).is_none());
    }

    #[test]
    fn manual_returns_no_command() {
        assert!(build_command(&InstallSource::Manual).is_none());
    }
}
