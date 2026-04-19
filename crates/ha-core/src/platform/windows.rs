use std::os::windows::process::CommandExt;
use std::path::PathBuf;
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
