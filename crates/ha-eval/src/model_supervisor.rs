//! Authenticated, loopback-only Hope Server supervisor for registered
//! process-restart evaluation faults.
//!
//! The supervisor owns the one-shot Provider secret bundle and re-injects it
//! only into a freshly spawned Hope process. The Agent/tool environment never
//! receives the control token or Provider bundle. No manifest command string
//! reaches this module: the executable and fixed `server start` argv are
//! validated here.

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PROVIDER_SECRETS_ENV: &str = "HA_MODEL_EVAL_PROVIDER_SECRETS_B64";
const SERVER_TOKEN_ENV: &str = "HA_MODEL_EVAL_SERVER_TOKEN";
const SUPERVISOR_TOKEN_ENV: &str = "HA_MODEL_EVAL_SUPERVISOR_TOKEN";
#[cfg(target_os = "linux")]
const ISOLATED_PID_NAMESPACE_ENV: &str = "HA_MODEL_EVAL_ISOLATED_PID_NAMESPACE";
const MAX_CREDENTIAL_BYTES: u64 = 1_250_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupervisorCredentials {
    #[serde(default)]
    provider_secrets_b64: Option<String>,
    server_token: String,
    supervisor_token: String,
}

pub async fn run(
    root: &Path,
    server_bin: &Path,
    bind: &str,
    control_bind: &str,
    credentials_stdin: bool,
) -> Result<()> {
    let server_bin = server_bin
        .canonicalize()
        .with_context(|| format!("canonicalizing Hope server {}", server_bin.display()))?;
    let name = server_bin
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "hope-agent-server"
            | "hope-agent-server.exe"
            | "hope-agent"
            | "hope-agent.exe"
            | "hope agent"
    ) {
        bail!("model supervisor only launches a registered Hope product binary");
    }
    let supervisor = std::env::current_exe()
        .context("resolving model supervisor executable")?
        .canonicalize()?;
    if !server_bin.starts_with(root)
        && server_bin.parent() != supervisor.parent()
        && !same_macos_app_bundle(&server_bin, &supervisor)
    {
        bail!("model supervisor product binary must be in the checkout or installed Hope bundle");
    }
    let server_addr = parse_loopback(bind, "server bind")?;
    let control_addr = parse_loopback(control_bind, "supervisor control bind")?;
    if server_addr == control_addr {
        bail!("model supervisor control and server binds must differ");
    }
    let credentials = load_credentials(credentials_stdin)?;
    harden_secret_holder()?;
    let provider_secrets = credentials.provider_secrets_b64;
    let server_token = credentials.server_token;
    let supervisor_token = credentials.supervisor_token;

    let listener = TcpListener::bind(control_addr)
        .await
        .context("binding model supervisor control listener")?;
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("building supervisor health client")?;
    let health_url = format!("http://{server_addr}/api/health");
    let mut child = spawn_server(
        &server_bin,
        bind,
        provider_secrets.as_deref(),
        &server_token,
    )?;
    wait_healthy(&client, &health_url, &mut child).await?;
    println!("model-eval supervisor ready on {control_addr} for Hope {server_addr}");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted.context("accepting supervisor request")?;
                if !peer.ip().is_loopback() {
                    continue;
                }
                match read_request(stream, &supervisor_token).await? {
                    SupervisorRequest::Health(mut stream) => {
                        let running = child.try_wait().context("checking Hope server process")?.is_none();
                        write_response(&mut stream, if running { 200 } else { 503 }, if running { "ok" } else { "server_exited" }).await?;
                    }
                    SupervisorRequest::Restart(mut stream) => {
                        terminate_child(&mut child)?;
                        child = spawn_server(&server_bin, bind, provider_secrets.as_deref(), &server_token)?;
                        match wait_healthy(&client, &health_url, &mut child).await {
                            Ok(()) => write_response(&mut stream, 200, "restarted").await?,
                            Err(error) => {
                                let _ = write_response(&mut stream, 503, "restart_failed").await;
                                return Err(error);
                            }
                        }
                    }
                    SupervisorRequest::Shutdown(mut stream) => {
                        write_response(&mut stream, 200, "shutting_down").await?;
                        terminate_child(&mut child)?;
                        return Ok(());
                    }
                    SupervisorRequest::Unauthorized(mut stream) => {
                        write_response(&mut stream, 401, "unauthorized").await?;
                    }
                    SupervisorRequest::NotFound(mut stream) => {
                        write_response(&mut stream, 404, "not_found").await?;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if let Some(status) = child.try_wait().context("checking Hope server process")? {
                    bail!("supervised Hope server exited unexpectedly with {status}");
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn same_macos_app_bundle(left: &Path, right: &Path) -> bool {
    fn bundle(path: &Path) -> Option<&Path> {
        path.ancestors()
            .find(|part| part.extension().and_then(|value| value.to_str()) == Some("app"))
    }
    matches!((bundle(left), bundle(right)), (Some(left), Some(right)) if left == right)
}

#[cfg(not(target_os = "macos"))]
fn same_macos_app_bundle(_left: &Path, _right: &Path) -> bool {
    false
}

fn load_credentials(from_stdin: bool) -> Result<SupervisorCredentials> {
    let credentials = if from_stdin {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_CREDENTIAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("reading model supervisor credentials from stdin")?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
            bail!("model supervisor credential envelope has an invalid size");
        }
        decode_credentials(&bytes)?
    } else {
        SupervisorCredentials {
            provider_secrets_b64: optional_secret(PROVIDER_SECRETS_ENV)?,
            server_token: required_secret(SERVER_TOKEN_ENV)?,
            supervisor_token: required_secret(SUPERVISOR_TOKEN_ENV)?,
        }
    };
    validate_secret_value(SERVER_TOKEN_ENV, &credentials.server_token, 4_096)?;
    validate_secret_value(SUPERVISOR_TOKEN_ENV, &credentials.supervisor_token, 4_096)?;
    if credentials.server_token == credentials.supervisor_token {
        bail!("model supervisor and Hope server tokens must differ");
    }
    if let Some(provider_secrets) = credentials.provider_secrets_b64.as_deref() {
        validate_secret_value(PROVIDER_SECRETS_ENV, provider_secrets, 1_000_000)?;
    }
    Ok(credentials)
}

fn decode_credentials(bytes: &[u8]) -> Result<SupervisorCredentials> {
    serde_json::from_slice::<SupervisorCredentials>(bytes)
        .context("decoding model supervisor credential envelope")
}

fn validate_secret_value(name: &str, value: &str, maximum: usize) -> Result<()> {
    if value.len() < 24 || value.len() > maximum || value.contains(['\r', '\n']) {
        bail!("{name} has an invalid length or encoding");
    }
    Ok(())
}

fn required_secret(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    std::env::remove_var(name);
    if value.len() < 24 || value.len() > 1_000_000 || value.contains(['\r', '\n']) {
        bail!("{name} has an invalid length or encoding");
    }
    Ok(value)
}

fn optional_secret(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            std::env::remove_var(name);
            if value.len() < 24 || value.len() > 1_000_000 || value.contains(['\r', '\n']) {
                bail!("{name} has an invalid length or encoding");
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {name}")),
    }
}

fn harden_secret_holder() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // The supervisor retains the Provider bundle in memory so it can
        // restart Hope. Make the process non-dumpable before any Agent tool can
        // execute, preventing same-UID descendants from reading /proc memory or
        // the original environment block.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("disabling model supervisor process dumps");
        }
    }
    Ok(())
}

fn parse_loopback(value: &str, label: &str) -> Result<SocketAddr> {
    let address = value
        .parse::<SocketAddr>()
        .with_context(|| format!("parsing {label}"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        bail!("{label} must be a non-zero loopback TCP address");
    }
    Ok(address)
}

fn spawn_server(
    server_bin: &Path,
    bind: &str,
    provider_secrets: Option<&str>,
    server_token: &str,
) -> Result<Child> {
    let mut command = Command::new(server_bin);
    command
        .args(["server", "start", "--bind", bind])
        .env("HA_MODEL_EVAL_MODE", "1")
        .env(SERVER_TOKEN_ENV, server_token)
        .env_remove(SUPERVISOR_TOKEN_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            // If the supervisor exits unexpectedly, ensure at least the direct
            // Hope process cannot outlive it. Normal restart/shutdown performs
            // the stronger recursive /proc tree termination below.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    if let Some(provider_secrets) = provider_secrets {
        command.env(PROVIDER_SECRETS_ENV, provider_secrets);
    }
    command.spawn().context("spawning supervised Hope server")
}

async fn wait_healthy(client: &Client, health_url: &str, child: &mut Child) -> Result<()> {
    for _ in 0..120 {
        if let Some(status) = child.try_wait().context("checking Hope server startup")? {
            bail!("supervised Hope server exited during startup with {status}");
        }
        if client
            .get(health_url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("supervised Hope server did not become healthy within 30 seconds")
}

fn terminate_child(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .context("checking supervised Hope process")?
        .is_some()
    {
        return Ok(());
    }
    let root_pid = child.id();
    #[cfg(target_os = "linux")]
    let terminated = terminate_linux_process_tree(root_pid)?;
    #[cfg(all(unix, not(target_os = "linux")))]
    unsafe {
        libc::kill(-(root_pid as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = child.kill();
    }
    let _ = child.kill();
    let _ = child.wait();
    #[cfg(target_os = "linux")]
    wait_for_linux_processes_to_exit(&terminated)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn terminate_linux_process_tree(root_pid: u32) -> Result<HashSet<u32>> {
    let mut targets = HashSet::from([root_pid]);
    // Freeze each discovered generation before rescanning. This closes the
    // race where a tool process forks while the supervisor is walking /proc,
    // including exec children that deliberately created another process group.
    for _ in 0..16 {
        for pid in &targets {
            unsafe {
                libc::kill(*pid as libc::pid_t, libc::SIGSTOP);
            }
        }
        let discovered = if std::env::var(ISOLATED_PID_NAMESPACE_ENV).as_deref() == Ok("1") {
            linux_namespace_workload_processes()?
        } else {
            linux_descendants(root_pid)?
        };
        let before = targets.len();
        targets.extend(discovered);
        if targets.len() == before {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for pid in &targets {
        unsafe {
            libc::kill(*pid as libc::pid_t, libc::SIGKILL);
        }
    }
    Ok(targets)
}

#[cfg(target_os = "linux")]
fn linux_descendants(root_pid: u32) -> Result<HashSet<u32>> {
    let process_table = linux_process_table()?;
    let mut children = HashMap::<u32, Vec<u32>>::new();
    for (pid, parent) in process_table {
        children.entry(parent).or_default().push(pid);
    }
    let mut descendants = HashSet::new();
    let mut queue = VecDeque::from([root_pid]);
    while let Some(parent) = queue.pop_front() {
        for child in children.remove(&parent).unwrap_or_default() {
            if descendants.insert(child) {
                queue.push_back(child);
            }
        }
    }
    Ok(descendants)
}

#[cfg(target_os = "linux")]
fn linux_namespace_workload_processes() -> Result<HashSet<u32>> {
    let process_table = linux_process_table()?;
    let mut protected = HashSet::new();
    let mut current = std::process::id();
    while current != 0 && protected.insert(current) {
        current = process_table.get(&current).copied().unwrap_or(0);
    }
    Ok(process_table
        .keys()
        .copied()
        .filter(|pid| !protected.contains(pid))
        .collect())
}

#[cfg(target_os = "linux")]
fn linux_process_table() -> Result<HashMap<u32, u32>> {
    let mut processes = HashMap::new();
    for entry in std::fs::read_dir("/proc").context("reading /proc process table")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, fields)) = stat.rsplit_once(") ") else {
            continue;
        };
        let mut fields = fields.split_whitespace();
        let _state = fields.next();
        let Some(parent) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        processes.insert(pid, parent);
    }
    Ok(processes)
}

#[cfg(target_os = "linux")]
fn wait_for_linux_processes_to_exit(processes: &HashSet<u32>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = processes
            .iter()
            .copied()
            .filter(|pid| linux_process_is_live(*pid))
            .collect::<Vec<_>>();
        if alive.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("supervised Hope process tree did not exit: {alive:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(target_os = "linux")]
fn linux_process_is_live(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().next())
        .is_some_and(|state| state != "Z" && state != "X")
}

enum SupervisorRequest {
    Health(TcpStream),
    Restart(TcpStream),
    Shutdown(TcpStream),
    Unauthorized(TcpStream),
    NotFound(TcpStream),
}

async fn read_request(mut stream: TcpStream, token: &str) -> Result<SupervisorRequest> {
    let mut bytes = vec![0u8; 16 * 1024];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut bytes))
        .await
        .context("timing out supervisor request read")??;
    bytes.truncate(read);
    let request = std::str::from_utf8(&bytes).context("supervisor request is not UTF-8")?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let authorized = lines.any(|line| {
        line.strip_prefix("Authorization: Bearer ")
            .is_some_and(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes()))
    });
    if !authorized {
        return Ok(SupervisorRequest::Unauthorized(stream));
    }
    Ok(match request_line {
        "GET /health HTTP/1.1" => SupervisorRequest::Health(stream),
        "POST /restart HTTP/1.1" => SupervisorRequest::Restart(stream),
        "POST /shutdown HTTP/1.1" => SupervisorRequest::Shutdown(stream),
        _ => SupervisorRequest::NotFound(stream),
    })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Service Unavailable",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .context("writing supervisor response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_only_accepts_nonzero_loopback_addresses() {
        assert!(parse_loopback("127.0.0.1:19420", "test").is_ok());
        assert!(parse_loopback("[::1]:19420", "test").is_ok());
        assert!(parse_loopback("0.0.0.0:19420", "test").is_err());
        assert!(parse_loopback("127.0.0.1:0", "test").is_err());
    }

    #[test]
    fn control_token_comparison_is_exact() {
        assert!(constant_time_eq(
            b"012345678901234567890123",
            b"012345678901234567890123"
        ));
        assert!(!constant_time_eq(
            b"012345678901234567890123",
            b"012345678901234567890124"
        ));
        assert!(!constant_time_eq(b"short", b"different-length"));
    }

    #[test]
    fn credential_values_require_distinct_bounded_secrets() {
        assert!(validate_secret_value(SERVER_TOKEN_ENV, "012345678901234567890123", 4_096).is_ok());
        assert!(validate_secret_value(SERVER_TOKEN_ENV, "short", 4_096).is_err());
        assert!(
            validate_secret_value(SERVER_TOKEN_ENV, "01234567890123456789012\n", 4_096).is_err()
        );
        let credentials = decode_credentials(
            br#"{"providerSecretsB64":"abcdefghijklmnopqrstuvwxyz","serverToken":"012345678901234567890123","supervisorToken":"abcdefghijklmnopqrstuvwx"}"#,
        )
        .unwrap();
        assert_eq!(credentials.server_token, "012345678901234567890123");
        assert!(decode_credentials(br#"{"serverToken":"missing-fields"}"#).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn termination_reaps_descendants_that_change_process_group() {
        use std::os::unix::process::CommandExt;

        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("orphan-marker");
        let script = format!(
            "setsid sh -c 'sleep 1; printf leaked > {}' & sleep 5",
            marker.display()
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", &script])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        terminate_child(&mut child).unwrap();
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(
            !marker.exists(),
            "orphan descendant survived supervisor stop"
        );
    }
}
