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
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PROVIDER_SECRETS_ENV: &str = "HA_MODEL_EVAL_PROVIDER_SECRETS_B64";
const SERVER_TOKEN_ENV: &str = "HA_MODEL_EVAL_SERVER_TOKEN";
const SUPERVISOR_TOKEN_ENV: &str = "HA_MODEL_EVAL_SUPERVISOR_TOKEN";

pub async fn run(root: &Path, server_bin: &Path, bind: &str, control_bind: &str) -> Result<()> {
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
    let provider_secrets = optional_secret(PROVIDER_SECRETS_ENV)?;
    let server_token = required_secret(SERVER_TOKEN_ENV)?;
    let supervisor_token = required_secret(SUPERVISOR_TOKEN_ENV)?;

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
                        terminate_child(&mut child);
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
                        terminate_child(&mut child);
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

fn required_secret(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("{name} is required"))?;
    if value.len() < 24 || value.len() > 1_000_000 || value.contains(['\r', '\n']) {
        bail!("{name} has an invalid length or encoding");
    }
    Ok(value)
}

fn optional_secret(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            if value.len() < 24 || value.len() > 1_000_000 || value.contains(['\r', '\n']) {
                bail!("{name} has an invalid length or encoding");
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {name}")),
    }
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

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
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
}
