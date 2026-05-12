//! Self-contained binary swap path.
//!
//! Used when [`super::source_detector`] returns `Manual` or when the
//! package-manager path failed (stale tap, sudo blocked, network refused
//! the apt mirror, etc.). The pipeline:
//!
//! 1. Resolve current binary from `current_exe()` and current version
//!    from `app_init::app_version()` (registered by each binary entrypoint).
//! 2. Pull the manifest; find the bare-binary entry for this platform.
//! 3. Download the archive to `~/.hope-agent/updater/staging/<version>/`
//!    with progress events on `app_update:progress`.
//! 4. Verify the archive bytes against the manifest signature
//!    ([`super::signature`]). Hard fail on any verify error.
//! 5. Extract the named binary out of the archive.
//! 6. Back the current binary up under `~/.hope-agent/updater/backup/<old>/`.
//! 7. Atomically swap the new binary into place via
//!    [`crate::platform::atomic_replace_binary`].
//! 8. Trigger `service restart` via [`super::service_control`].
//! 9. Prune backups so disk usage stays bounded.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use super::manifest::{ArchiveKind, BareBinaryEntry, Manifest};
use super::{backup, download, manifest as manifest_mod, service_control, signature};

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub from_version: String,
    pub to_version: String,
    pub archive_bytes: u64,
    pub binary_swapped: bool,
    pub service_restart: Option<String>,
    /// `Some(msg)` when binary swap succeeded but the relaunch step
    /// failed (no installed service AND `lifecycle::restart()` refused,
    /// or the supervisor's kick returned an error). The caller is
    /// expected to surface this to the user — the new binary is on disk
    /// but the running process is still the old one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_failure: Option<String>,
}

/// Phase events emitted on `app_update:progress` so the UI / status tool
/// can render coarse progress beyond the byte-level download bar. Failure
/// is signaled via the separate `app_update:completed` topic (with
/// `status: "failed"`), not a phase frame.
#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Checking,
    Downloading,
    Verifying,
    Staging,
    Backing,
    Swapping,
    Restarting,
    /// Binary swap succeeded but the relaunch step did NOT (no service
    /// installed and no respawn path available, or supervisor refused).
    /// Used by `install()` to distinguish "fully done" from "the binary
    /// is in place but the running process is still the old one" — the
    /// app_update tool surfaces this to the user.
    SwapDone,
    Done,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Staging => "staging",
            Self::Backing => "backing",
            Self::Swapping => "swapping",
            Self::Restarting => "restarting",
            Self::SwapDone => "swap_done",
            Self::Done => "done",
        }
    }
}

pub fn emit_phase(job_id: &str, phase: Phase) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "app_update:progress",
            serde_json::json!({
                "job_id": job_id,
                "phase": phase.as_str(),
                "label": "lifecycle",
            }),
        );
    }
}

pub async fn install(
    job_id: &str,
    target_version: Option<&str>,
    preloaded_manifest: Option<Manifest>,
) -> Result<InstallOutcome> {
    emit_phase(job_id, Phase::Checking);
    let current_exe = std::env::current_exe().context("resolve current_exe")?;
    let from_version = crate::app_init::app_version().to_string();

    let manifest = match preloaded_manifest {
        Some(m) => m,
        None => manifest_mod::fetch_manifest().await?,
    };
    let to_version = target_version
        .map(|s| s.trim_start_matches('v').to_string())
        .unwrap_or_else(|| manifest.version.clone());

    if !manifest_mod::is_newer(&to_version, &from_version) {
        anyhow::bail!(
            "target version {to_version} is not newer than running version {from_version}"
        );
    }

    let platform_key = manifest_mod::current_platform_key();
    let entry = manifest_mod::select_bare_binary(&manifest, platform_key).ok_or_else(|| {
        anyhow::anyhow!(
            "manifest has no bare_binary entry for platform '{platform_key}' \
             — fall back to the package-manager path or download the installer manually"
        )
    })?;

    let staging = crate::paths::updater_staging_dir(&to_version)?;
    fs::create_dir_all(&staging)
        .with_context(|| format!("create staging dir {}", staging.display()))?;

    emit_phase(job_id, Phase::Downloading);
    let archive_path = staging.join(archive_filename(entry));
    let archive_bytes = download::download_to(&entry.url, &archive_path, job_id, "archive").await?;

    emit_phase(job_id, Phase::Verifying);
    // Read once into a local buffer to feed the verifier — minisign-verify
    // needs the whole payload. Skipping this and re-deriving from the
    // streaming download would require buffering in `download_to`, which
    // would conflict with the disk-spool path used by extraction below.
    let archive_buf = fs::read(&archive_path).with_context(|| {
        format!(
            "read archive {} for signature verify",
            archive_path.display()
        )
    })?;
    signature::verify_bytes(&archive_buf, &entry.signature)?;
    drop(archive_buf);

    emit_phase(job_id, Phase::Staging);
    let extracted = extract_binary(&archive_path, &entry.archive, &entry.binary_path, &staging)?;

    emit_phase(job_id, Phase::Backing);
    backup::store(&current_exe, &from_version)?;

    emit_phase(job_id, Phase::Swapping);
    crate::platform::atomic_replace_binary(&current_exe, &extracted).with_context(|| {
        format!(
            "atomic swap {} → {}",
            extracted.display(),
            current_exe.display()
        )
    })?;

    emit_phase(job_id, Phase::Restarting);
    // Pick the relaunch strategy by formfactor:
    //   - installed service: let the supervisor (launchctl / systemctl /
    //     schtasks) kill us and start the new binary. `restart_service`
    //     does an atomic stop+start so we don't need a separate SIGTERM.
    //   - foreground server with no installed service: hand off to
    //     `lifecycle::restart`, which spawns a detached child running the
    //     captured launch argv and schedules self-exit. Without this the
    //     newly-swapped binary would never actually load.
    //   - desktop / acp: we shouldn't be reaching this code path
    //     (self_contained is only routed for headless), but if we do, we
    //     leave it to the caller to relaunch and report the gap.
    let restart_status = if crate::service_install::is_service_installed() {
        match service_control::restart_service() {
            Ok(msg) => RestartStatus::Service(msg),
            Err(e) => RestartStatus::ServiceFailed(e.to_string()),
        }
    } else {
        match crate::lifecycle::restart() {
            Ok(outcome) => RestartStatus::Lifecycle(outcome.detail),
            Err(e) => RestartStatus::ManualRequired(e.to_string()),
        }
    };

    backup::prune();
    let failure_msg = restart_status.failure_msg().map(str::to_string);
    let phase_after_restart = if failure_msg.is_some() {
        Phase::SwapDone
    } else {
        Phase::Done
    };
    emit_phase(job_id, phase_after_restart);

    Ok(InstallOutcome {
        from_version,
        to_version,
        archive_bytes,
        binary_swapped: true,
        service_restart: restart_status.into_label(),
        restart_failure: failure_msg,
    })
}

#[derive(Debug)]
enum RestartStatus {
    /// `service_control::restart_service` returned Ok — supervisor accepted
    /// the kick. The string is the supervisor's ack.
    Service(String),
    /// `service_control::restart_service` returned Err — supervisor exists
    /// but the kick failed. New binary is on disk, but the running process
    /// is still the old one.
    ServiceFailed(String),
    /// No installed service — `lifecycle::restart()` handled the relaunch
    /// (`Respawn` for foreground server, etc.). String is the ack detail.
    Lifecycle(String),
    /// No installed service AND `lifecycle::restart()` refused (e.g. ACP /
    /// unknown role). Caller / user must manually relaunch.
    ManualRequired(String),
}

impl RestartStatus {
    fn into_label(self) -> Option<String> {
        match self {
            Self::Service(s) | Self::Lifecycle(s) => Some(s),
            Self::ServiceFailed(s) => Some(format!("service restart failed: {s}")),
            Self::ManualRequired(s) => Some(format!("manual restart required: {s}")),
        }
    }

    fn failure_msg(&self) -> Option<&str> {
        match self {
            Self::ServiceFailed(s) | Self::ManualRequired(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

pub fn rollback(job_id: &str) -> Result<InstallOutcome> {
    emit_phase(job_id, Phase::Staging);
    let current_exe = std::env::current_exe().context("resolve current_exe")?;
    let from_version = crate::app_init::app_version().to_string();
    let backup_path = backup::most_recent().ok_or_else(|| {
        anyhow::anyhow!("no backup binary found under ~/.hope-agent/updater/backup/")
    })?;

    emit_phase(job_id, Phase::Swapping);
    crate::platform::atomic_replace_binary(&current_exe, &backup_path)?;

    emit_phase(job_id, Phase::Restarting);
    let restart = service_control::restart_service().ok();
    emit_phase(job_id, Phase::Done);

    Ok(InstallOutcome {
        from_version,
        to_version: "<restored from backup>".into(),
        archive_bytes: 0,
        binary_swapped: true,
        service_restart: restart,
        restart_failure: None,
    })
}

fn archive_filename(entry: &BareBinaryEntry) -> &'static str {
    match entry.archive {
        ArchiveKind::TarGz => "archive.tar.gz",
        ArchiveKind::Zip => "archive.zip",
    }
}

fn extract_binary(
    archive_path: &Path,
    kind: &ArchiveKind,
    binary_path: &str,
    staging: &Path,
) -> Result<PathBuf> {
    let out = staging.join(binary_basename(binary_path));
    match kind {
        ArchiveKind::TarGz => extract_tar_gz(archive_path, binary_path, &out)?,
        ArchiveKind::Zip => extract_zip(archive_path, binary_path, &out)?,
    }
    if !out.is_file() {
        anyhow::bail!(
            "extracted binary missing at {} after extraction",
            out.display()
        );
    }
    Ok(out)
}

fn binary_basename(declared: &str) -> String {
    Path::new(declared)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "hope-agent.exe".into()
            } else {
                "hope-agent".into()
            }
        })
}

fn extract_tar_gz(archive_path: &Path, binary_path: &str, out: &Path) -> Result<()> {
    let f = fs::File::open(archive_path)
        .with_context(|| format!("open archive {}", archive_path.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path_matches(&path, binary_path) {
            let mut out_file = fs::File::create(out)
                .with_context(|| format!("create extracted output {}", out.display()))?;
            std::io::copy(&mut entry, &mut out_file)?;
            return Ok(());
        }
    }
    anyhow::bail!(
        "binary '{binary_path}' not found in tar.gz {}",
        archive_path.display()
    )
}

fn extract_zip(archive_path: &Path, binary_path: &str, out: &Path) -> Result<()> {
    let f = fs::File::open(archive_path)
        .with_context(|| format!("open archive {}", archive_path.display()))?;
    let mut zip = zip::ZipArchive::new(f)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if path_matches(Path::new(&name), binary_path) {
            let mut out_file = fs::File::create(out)
                .with_context(|| format!("create extracted output {}", out.display()))?;
            std::io::copy(&mut entry, &mut out_file)?;
            return Ok(());
        }
    }
    anyhow::bail!(
        "binary '{binary_path}' not found in zip {}",
        archive_path.display()
    )
}

fn path_matches(entry: &Path, declared: &str) -> bool {
    // Manifest always declares forward-slash paths; normalize the archive
    // entry path the same way so a Windows-built zip with backslashes
    // still matches a Unix-declared `hope-agent` entry.
    let normalized = entry.to_string_lossy().replace('\\', "/");
    normalized == declared
        || normalized.ends_with(&format!("/{declared}"))
        || normalized == format!("./{declared}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_matches_handles_simple_basename() {
        assert!(path_matches(Path::new("hope-agent"), "hope-agent"));
    }

    #[test]
    fn path_matches_handles_nested_archive_layout() {
        assert!(path_matches(
            Path::new("hope-agent-0.2.1-linux-x86_64/hope-agent"),
            "hope-agent"
        ));
    }

    #[test]
    fn path_matches_normalizes_windows_separators() {
        // ZipArchive on a Windows-built archive can hand back entries with
        // backslashes — make sure we match against the slash-canonical
        // manifest declaration.
        let p = PathBuf::from("hope-agent-0.2.1-windows-x86_64\\hope-agent.exe");
        assert!(path_matches(&p, "hope-agent.exe"));
    }

    #[test]
    fn binary_basename_picks_trailing_segment() {
        assert_eq!(binary_basename("hope-agent"), "hope-agent");
        assert_eq!(
            binary_basename("hope-agent-0.2.1-linux-x86_64/hope-agent"),
            "hope-agent"
        );
        assert_eq!(binary_basename("hope-agent.exe"), "hope-agent.exe");
    }
}
