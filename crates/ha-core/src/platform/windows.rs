use std::fs;
use std::io::{self, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) fn terminate_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub(super) fn send_graceful_stop(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub(super) fn detect_system_proxy() -> Option<String> {
    // Cache once per process: winreg access is cheap but callers
    // (provider/proxy, docker/proxy, …) would otherwise each re-read
    // on every client build.
    use std::sync::OnceLock;
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED.get_or_init(probe_system_proxy).clone()
}

fn probe_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;

    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }

    let server: String = settings.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }

    // ProxyServer can be either a single "host:port" or a protocol-specific
    // list like "http=127.0.0.1:1082;https=127.0.0.1:1082;ftp=...".
    // Prefer https, fall back to http, fall back to the bare form.
    if server.contains('=') {
        let mut http: Option<&str> = None;
        let mut https: Option<&str> = None;
        for part in server.split(';') {
            let part = part.trim();
            if let Some(rest) = part.strip_prefix("https=") {
                https = Some(rest);
            } else if let Some(rest) = part.strip_prefix("http=") {
                http = Some(rest);
            }
        }
        let pick = https.or(http)?;
        return Some(format!("http://{}", pick));
    }

    Some(format!("http://{}", server))
}

pub(super) fn default_shell_command(cmdline: &str) -> Command {
    // `cmd /C` consumes the *rest* of the command line verbatim, so we use
    // `raw_arg` to avoid std's automatic quoting rewriting the user payload.
    let mut cmd = Command::new("cmd");
    cmd.raw_arg("/C").raw_arg(cmdline);
    cmd
}

pub(super) fn default_shell_command_tokio(cmdline: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.raw_arg("/C").raw_arg(cmdline);
    cmd
}

pub(super) fn find_chrome_executable() -> Option<PathBuf> {
    // Use env vars rather than hard-coding `C:\Program Files` so we
    // handle localized / ARM / alternate-drive installs. %LOCALAPPDATA%
    // covers per-user installs.
    let relatives: &[&str] = &[
        r"Google\Chrome\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"Chromium\Application\chrome.exe",
    ];

    for env_var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Ok(root) = std::env::var(env_var) else {
            continue;
        };
        for rel in relatives {
            let full = PathBuf::from(&root).join(rel);
            if full.is_file() {
                return Some(full);
            }
        }
    }

    None
}

pub(super) fn try_acquire_exclusive_lock(path: &Path) -> io::Result<Option<fs::File>> {
    use std::io::ErrorKind;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // FILE_SHARE_READ keeps the open kernel-exclusive against other
    // *writers* — any second process trying to open the same path with
    // write access gets ERROR_SHARING_VIOLATION, which is the only
    // exclusion we need (the holder body is the only thing written). We
    // can't use FILE_SHARE_NONE: it would also block read-only opens from
    // the same process, breaking `current_holder()`'s diagnostic read.
    // The handle is released automatically when the process exits or
    // panics, matching flock(LOCK_EX) semantics. FILE_FLAG_NO_INHERIT_HANDLE
    // keeps spawned children from holding the handle alive past their
    // parent's death.
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_NO_INHERIT_HANDLE: u32 = 0x0000_0080;
    let result = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_NO_INHERIT_HANDLE)
        .open(path);

    match result {
        Ok(file) => Ok(Some(file)),
        Err(e) => {
            // ERROR_SHARING_VIOLATION (32) — another process owns it.
            // PermissionDenied is what `io::Error::kind` maps it to.
            if matches!(e.kind(), ErrorKind::PermissionDenied) || e.raw_os_error() == Some(32) {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

pub(super) fn write_secure_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    {
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // NTFS inherits a DACL from the parent directory — `~/.hope-agent/`
    // lives under the user profile so by default only the owning user
    // and SYSTEM/Administrators can read. Hardening to an explicit DACL
    // (strip inherited ACEs, grant only the owner) is a future pass.
    // Windows rename fails if the destination exists; remove first.
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub(super) fn run_hidden(cmd: &str, args: &[&str]) -> Option<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
}

pub(super) fn detect_dedicated_gpu_fallback() -> Option<super::DetectedGpu> {
    // `Win32_VideoController.AdapterRAM` is a 32-bit field that wraps at
    // 4 GiB. We surface 4096 MiB as a conservative floor so the recommender
    // doesn't think a high-end GPU has tiny memory; the GUI surfaces the
    // raw name so users can sanity-check.
    let script = "Get-CimInstance Win32_VideoController | \
                  Where-Object { $_.AdapterRAM -gt 0 } | \
                  Sort-Object AdapterRAM -Descending | \
                  Select-Object -First 1 | \
                  ForEach-Object { \"$($_.Name)|$($_.AdapterRAM)\" }";
    let output = run_hidden(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(2, '|');
    let name = parts.next()?.trim().to_string();
    let bytes = parts.next()?.trim().parse::<u64>().ok()?;
    let mut vram_mb = bytes / (1024 * 1024);
    if (4090..=4100).contains(&vram_mb) {
        vram_mb = 4096;
    }
    Some(super::DetectedGpu {
        name,
        vram_mb: Some(vram_mb),
    })
}

pub(super) fn os_version_string() -> String {
    let long = sysinfo::System::long_os_version();
    let kernel = sysinfo::System::kernel_version();
    match (long, kernel) {
        (Some(name), Some(build)) => format!("{} ({})", name, build),
        (Some(name), None) => name,
        (None, Some(build)) => format!("Windows ({})", build),
        (None, None) => "Windows (unknown build)".to_string(),
    }
}
