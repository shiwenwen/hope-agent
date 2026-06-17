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

/// Verified, extracted-but-not-yet-swapped artifact ready for the swap step.
struct StagedBuild {
    /// Path to the extracted new binary inside the staging dir.
    extracted: PathBuf,
    /// Bytes of the downloaded archive (0 if a verified archive was reused).
    archive_bytes: u64,
    /// Resolved target version (frontmatter-stripped).
    to_version: String,
    /// Version of the currently-running binary.
    from_version: String,
}

/// Download → verify → extract the new build into staging, **without** touching
/// the live binary. Reuses an already-downloaded archive when it still passes
/// signature verification (the silent pre-download path stages here first, so
/// the later install is a no-network swap). Shared by [`install`] and
/// [`stage_only`].
async fn download_and_extract(
    job_id: &str,
    target_version: Option<&str>,
    preloaded_manifest: Option<Manifest>,
) -> Result<StagedBuild> {
    emit_phase(job_id, Phase::Checking);
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

    // Drop staging dirs for other versions before we (re)stage this one.
    super::staging::prune(Some(&to_version));

    let staging = crate::paths::updater_staging_dir(&to_version)?;
    fs::create_dir_all(&staging)
        .with_context(|| format!("create staging dir {}", staging.display()))?;

    let archive_path = staging.join(archive_filename(entry));

    // Reuse a previously-staged archive iff it still verifies — this is what
    // makes the silent pre-download pay off (install becomes download-free).
    let mut archive_bytes = 0u64;
    let reused = archive_path.is_file()
        && match fs::read(&archive_path) {
            Ok(buf) => signature::verify_bytes(&buf, &entry.signature).is_ok(),
            Err(_) => false,
        };
    if reused {
        app_info!(
            "self_update",
            "stage",
            "reusing verified staged archive for {} ({})",
            to_version,
            archive_path.display()
        );
        emit_phase(job_id, Phase::Verifying);
    } else {
        emit_phase(job_id, Phase::Downloading);
        archive_bytes = download::download_to(&entry.url, &archive_path, job_id, "archive").await?;

        emit_phase(job_id, Phase::Verifying);
        // Read once into a local buffer to feed the verifier — minisign-verify
        // needs the whole payload.
        let archive_buf = fs::read(&archive_path).with_context(|| {
            format!(
                "read archive {} for signature verify",
                archive_path.display()
            )
        })?;
        let verify = signature::verify_bytes(&archive_buf, &entry.signature);
        drop(archive_buf);
        if let Err(e) = verify {
            // A bad signature means a corrupt / tampered download — never keep
            // it around to be "reused" next time.
            let _ = fs::remove_file(&archive_path);
            return Err(e);
        }
    }

    emit_phase(job_id, Phase::Staging);
    let extracted = extract_binary(&archive_path, &entry.archive, &entry.binary_path, &staging)?;

    Ok(StagedBuild {
        extracted,
        archive_bytes,
        to_version,
        from_version,
    })
}

/// Pre-download + verify + extract the new build into staging without swapping.
/// Used by the headless auto-update loop's silent-download step so the eventual
/// install is instant. Returns the resolved target version.
pub async fn stage_only(
    job_id: &str,
    target_version: Option<&str>,
    preloaded_manifest: Option<Manifest>,
) -> Result<String> {
    let staged = download_and_extract(job_id, target_version, preloaded_manifest).await?;
    emit_phase(job_id, Phase::Done);
    Ok(staged.to_version)
}

pub async fn install(
    job_id: &str,
    target_version: Option<&str>,
    preloaded_manifest: Option<Manifest>,
) -> Result<InstallOutcome> {
    let StagedBuild {
        extracted,
        archive_bytes,
        to_version,
        from_version,
    } = download_and_extract(job_id, target_version, preloaded_manifest).await?;

    let current_exe = std::env::current_exe().context("resolve current_exe")?;

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

    // Smoke-test the freshly-swapped binary before we hand control to it via a
    // service restart. A binary that can't even print its version (wrong arch,
    // truncated, missing shared lib) would otherwise leave the service dead
    // with no working image. On failure, roll the backup back into place.
    emit_phase(job_id, Phase::Verifying);
    if let Err(e) = smoke_test(&current_exe, &to_version).await {
        app_error!(
            "self_update",
            "install",
            "new binary failed smoke test: {e}; rolling back"
        );
        if let Some(backup_path) = backup::most_recent() {
            if let Err(re) = crate::platform::atomic_replace_binary(&current_exe, &backup_path) {
                anyhow::bail!(
                    "new binary failed smoke test ({e}) AND rollback failed ({re}) — \
                     manual recovery required: restore {} from {}",
                    current_exe.display(),
                    backup_path.display()
                );
            }
            anyhow::bail!("new binary failed smoke test ({e}); rolled back to previous version");
        }
        anyhow::bail!("new binary failed smoke test ({e}); no backup available to roll back");
    }

    emit_phase(job_id, Phase::Restarting);
    // NOTE: do NOT call `stop_if_running` here — `restart_service`
    // already does atomic kill+restart on every platform
    // (launchctl kickstart -k / systemctl --user restart / schtasks
    // /End + /Run). When self-update runs inside the daemon itself,
    // a separate SIGTERM would trigger our own signal handler's
    // `exit(0)`, and systemd's `Restart=on-failure` would NOT pull
    // us back up after a clean exit — the service would stay
    // stopped with the new binary in place.
    let restart = service_control::restart_service().ok();

    backup::prune();
    // The swap consumed the staged build; drop it so it isn't "reused" later.
    super::staging::prune(None);
    emit_phase(job_id, Phase::Done);

    Ok(InstallOutcome {
        from_version,
        to_version,
        archive_bytes,
        binary_swapped: true,
        service_restart: restart,
    })
}

/// Run `<exe> --version` with a hard timeout and confirm it reports
/// `expected_version`. The whole point is to catch a binary that won't even
/// start before we restart the service onto it.
async fn smoke_test(exe: &Path, expected_version: &str) -> Result<()> {
    use std::time::Duration;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--version");
    crate::platform::hide_console_tokio(&mut cmd);
    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .context("smoke test `--version` timed out after 5s")?
        .context("spawn new binary for smoke test")?;

    if !output.status.success() {
        anyhow::bail!("new binary `--version` exited with {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Match on the `major.minor.patch` core, not a raw substring: a substring
    // check both false-passes ("0.8.1" ⊂ "0.8.10") and false-fails (manifest
    // "0.8.1+build5" vs binary "0.8.1"). Compare cores so build / pre-release
    // suffixes on either side don't trigger a spurious rollback.
    let want = semver_core(expected_version);
    let matched = if want.is_empty() {
        // Expected version isn't a clean N.N.N — fall back to a substring check
        // rather than matching every empty-core token.
        let needle = expected_version.trim().trim_start_matches('v');
        !needle.is_empty() && (stdout.contains(needle) || stderr.contains(needle))
    } else {
        stdout
            .split_whitespace()
            .chain(stderr.split_whitespace())
            .any(|tok| semver_core(tok) == want)
    };
    if matched {
        Ok(())
    } else {
        anyhow::bail!(
            "new binary version output {:?} does not report expected version {} (core {})",
            stdout.trim(),
            expected_version,
            want
        )
    }
}

/// Leading `major.minor.patch` of a version-ish token, with a `v` prefix and
/// any `+build` / `-prerelease` suffix stripped. Returns `""` for tokens that
/// don't start with a numeric core (so non-version words never match).
fn semver_core(s: &str) -> String {
    let s = s.trim().trim_start_matches('v');
    let core = s.split(['+', '-']).next().unwrap_or(s);
    // Require the shape N.N.N (all-numeric components) so a stray word like
    // "hope-agent" or "build" can't be mistaken for a version.
    let comps: Vec<&str> = core.split('.').collect();
    if comps.len() == 3
        && comps
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
    {
        core.to_string()
    } else {
        String::new()
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

    #[test]
    fn semver_core_strips_prefix_and_suffixes() {
        assert_eq!(semver_core("0.8.1"), "0.8.1");
        assert_eq!(semver_core("v0.8.1"), "0.8.1");
        assert_eq!(semver_core("0.8.1+build5"), "0.8.1");
        assert_eq!(semver_core("0.8.1-rc1"), "0.8.1");
        assert_eq!(semver_core(" 0.8.1 "), "0.8.1");
    }

    #[test]
    fn semver_core_distinguishes_patch_lengths() {
        // The bug a raw substring check would miss: 0.8.1 must NOT equal 0.8.10.
        assert_ne!(semver_core("0.8.1"), semver_core("0.8.10"));
        assert_eq!(semver_core("0.8.10"), "0.8.10");
    }

    #[test]
    fn semver_core_rejects_non_version_tokens() {
        assert_eq!(semver_core("hope-agent"), "");
        assert_eq!(semver_core("0.8"), "");
        assert_eq!(semver_core("0.8.x"), "");
        assert_eq!(semver_core(""), "");
    }
}
