#[cfg(windows)]
use std::fs::File;
use std::io::{stdin, stdout, Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use ha_browser_host::protocol::{read_native_message, write_native_message, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerDiscovery {
    protocol_version: u32,
    endpoint: String,
    token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrokerEndpoint {
    Tcp(String),
    Unix(PathBuf),
    #[cfg(windows)]
    Pipe(String),
}

enum BrokerStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Pipe(File),
}

impl BrokerStream {
    fn try_clone(&self) -> Result<Self> {
        match self {
            Self::Tcp(stream) => Ok(Self::Tcp(stream.try_clone()?)),
            #[cfg(unix)]
            Self::Unix(stream) => Ok(Self::Unix(stream.try_clone()?)),
            #[cfg(windows)]
            Self::Pipe(stream) => Ok(Self::Pipe(stream.try_clone()?)),
        }
    }
}

impl Read for BrokerStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
            #[cfg(windows)]
            Self::Pipe(stream) => stream.read(buf),
        }
    }
}

impl Write for BrokerStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
            #[cfg(windows)]
            Self::Pipe(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Pipe(stream) => stream.flush(),
        }
    }
}

fn main() -> Result<()> {
    let mut native_in = stdin().lock();
    let native_out = Arc::new(Mutex::new(stdout()));

    // The host's lifetime is tied to a LIVE broker connection. If the broker is
    // unavailable at startup or drops later (e.g. the desktop app restarts), the
    // host EXITS so Chrome closes the native port — the extension's onDisconnect
    // then drives a reconnect, which respawns a fresh host against the (possibly
    // restarted) broker. Staying alive in a degraded local-only mode would wedge
    // the extension "connected" to a zombie host that can never reach the new
    // broker, so the user would have to reload the extension by hand after every
    // app restart. Exiting is what makes auto-recovery work.
    let broker = match connect_broker() {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("ha-browser-host: broker unavailable at startup: {e:#}");
            return Ok(());
        }
    };
    start_broker_to_native(broker.try_clone()?, native_out);
    let broker_writer = Arc::new(Mutex::new(broker));

    loop {
        let message = match read_native_message(&mut native_in) {
            Ok(Some(message)) => message,
            Ok(None) => break, // clean EOF: Chrome closed the port
            Err(e) => {
                // A malformed/oversized frame from Chrome corrupts the stream;
                // log to stderr (stdout is the native-messaging channel) and
                // exit this host instance cleanly instead of aborting with `?`.
                // Chrome relaunches the host on the next message.
                eprintln!("ha-browser-host: native read error: {e:#}");
                break;
            }
        };
        let write_result = broker_writer
            .lock()
            .map_err(|_| anyhow::anyhow!("broker writer mutex poisoned"))
            .and_then(|mut stream| write_native_message(&mut *stream, &message));
        if let Err(e) = write_result {
            // Broker connection lost mid-session (app restart). Exit so the
            // extension reconnects against a fresh host instead of dropping
            // commands into a dead socket and faking connectivity.
            eprintln!("ha-browser-host: broker write failed, exiting: {e:#}");
            break;
        }
    }

    Ok(())
}

fn connect_broker() -> Result<BrokerStream> {
    let discovery = read_discovery().context("reading broker discovery")?;
    if discovery.protocol_version != PROTOCOL_VERSION {
        anyhow::bail!(
            "broker protocol mismatch: host={} broker={}",
            PROTOCOL_VERSION,
            discovery.protocol_version
        );
    }
    let endpoint = parse_endpoint(&discovery.endpoint)?;
    let mut stream = connect_endpoint(&endpoint)
        .with_context(|| format!("connecting broker {}", discovery.endpoint))?;
    let hello = json!({
        "id": "host-hello",
        "method": "host.hello",
        "token": discovery.token,
        "payload": {
            "host": "ha-browser-host",
            "hostVersion": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "protocolVersion": PROTOCOL_VERSION
        }
    });
    write_native_message(&mut stream, &hello)?;
    Ok(stream)
}

fn start_broker_to_native(
    mut broker_reader: BrokerStream,
    native_out: Arc<Mutex<std::io::Stdout>>,
) {
    thread::spawn(move || {
        loop {
            match read_native_message(&mut broker_reader) {
                Ok(Some(message)) => {
                    let Ok(mut out) = native_out.lock() else {
                        break;
                    };
                    if write_native_message(&mut *out, &message).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        // The broker connection ended (app stopped/restarted). Terminate the
        // whole host so Chrome closes the native port and the extension
        // reconnects against a fresh host — instead of lingering as a zombie
        // that holds the port open with no broker behind it.
        std::process::exit(0);
    });
}

fn parse_endpoint(endpoint: &str) -> Result<BrokerEndpoint> {
    if let Some(path) = endpoint.strip_prefix("unix:") {
        if path.is_empty() {
            anyhow::bail!("broker unix endpoint path is empty");
        }
        return Ok(BrokerEndpoint::Unix(PathBuf::from(path)));
    }
    if let Some(addr) = endpoint.strip_prefix("tcp:") {
        if addr.is_empty() {
            anyhow::bail!("broker tcp endpoint address is empty");
        }
        return Ok(BrokerEndpoint::Tcp(addr.to_string()));
    }
    if let Some(pipe) = endpoint.strip_prefix("pipe:") {
        if pipe.is_empty() {
            anyhow::bail!("broker pipe endpoint path is empty");
        }
        return parse_pipe_endpoint(pipe);
    }
    // Backward compatibility with early MVP discovery files that stored a
    // bare loopback address such as `127.0.0.1:54321`.
    Ok(BrokerEndpoint::Tcp(endpoint.to_string()))
}

fn connect_endpoint(endpoint: &BrokerEndpoint) -> Result<BrokerStream> {
    match endpoint {
        BrokerEndpoint::Tcp(addr) => TcpStream::connect(addr)
            .map(BrokerStream::Tcp)
            .with_context(|| format!("connecting TCP broker {addr}")),
        BrokerEndpoint::Unix(path) => connect_unix_endpoint(path),
        #[cfg(windows)]
        BrokerEndpoint::Pipe(path) => connect_pipe_endpoint(path),
    }
}

#[cfg(unix)]
fn connect_unix_endpoint(path: &std::path::Path) -> Result<BrokerStream> {
    UnixStream::connect(path)
        .map(BrokerStream::Unix)
        .with_context(|| format!("connecting Unix broker {}", path.display()))
}

#[cfg(windows)]
fn parse_pipe_endpoint(path: &str) -> Result<BrokerEndpoint> {
    Ok(BrokerEndpoint::Pipe(path.to_string()))
}

#[cfg(not(windows))]
fn parse_pipe_endpoint(path: &str) -> Result<BrokerEndpoint> {
    anyhow::bail!("Windows broker pipe endpoints are not supported on this platform: {path}")
}

#[cfg(windows)]
fn connect_pipe_endpoint(path: &str) -> Result<BrokerStream> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
        .open(path)
        .with_context(|| format!("connecting Windows named pipe broker {path}"))?;
    Ok(BrokerStream::Pipe(file))
}

#[cfg(not(unix))]
fn connect_unix_endpoint(path: &std::path::Path) -> Result<BrokerStream> {
    anyhow::bail!(
        "Unix broker endpoints are not supported on this platform: {}",
        path.display()
    )
}

fn read_discovery() -> Result<BrokerDiscovery> {
    let path = discovery_path()?;
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading broker discovery {}", path.display()))?;
    serde_json::from_slice(&bytes).context("decoding broker discovery JSON")
}

fn discovery_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HOPE_AGENT_BROWSER_BROKER_DISCOVERY") {
        return Ok(PathBuf::from(path));
    }
    let root = if let Some(path) = std::env::var_os("HA_DATA_DIR") {
        PathBuf::from(path)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?
            .join(".hope-agent")
    };
    Ok(root.join("browser-extension").join("broker.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tcp_endpoint_with_scheme() {
        assert_eq!(
            parse_endpoint("tcp:127.0.0.1:1234").unwrap(),
            BrokerEndpoint::Tcp("127.0.0.1:1234".to_string())
        );
    }

    #[test]
    fn parses_legacy_bare_tcp_endpoint() {
        assert_eq!(
            parse_endpoint("127.0.0.1:1234").unwrap(),
            BrokerEndpoint::Tcp("127.0.0.1:1234".to_string())
        );
    }

    #[test]
    fn parses_unix_endpoint() {
        assert_eq!(
            parse_endpoint("unix:/tmp/hope-agent.sock").unwrap(),
            BrokerEndpoint::Unix(PathBuf::from("/tmp/hope-agent.sock"))
        );
    }

    #[test]
    fn rejects_empty_unix_endpoint() {
        let err = parse_endpoint("unix:").unwrap_err().to_string();
        assert!(err.contains("path is empty"));
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_pipe_endpoint() {
        assert_eq!(
            parse_endpoint(r"pipe:\\.\pipe\hope-agent-browser-extension-42").unwrap(),
            BrokerEndpoint::Pipe(r"\\.\pipe\hope-agent-browser-extension-42".to_string())
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn rejects_windows_pipe_endpoint_off_windows() {
        let err = parse_endpoint(r"pipe:\\.\pipe\hope-agent-browser-extension-42")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not supported"));
    }
}
