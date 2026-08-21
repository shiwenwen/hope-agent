//! Connection-boundary proxy for the isolated `web_fetch` renderer.
//!
//! CDP request interception is useful for method/resource policy, but it runs
//! before Chromium opens the socket. A hostname can therefore rebind between
//! an async SSRF check and Chromium's own DNS lookup. This loopback proxy owns
//! the actual connection: it resolves and checks each destination once, then
//! connects only to one of those exact socket addresses. Chromium is forced
//! through it for both HTTP and HTTPS.

use std::net::{IpAddr, SocketAddr};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ha_core::provider::{ProxyConfig, ProxyMode};
use ha_core::security::ssrf::{resolve_checked_destination, SsrfPolicy};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use url::Url;

const MAX_PROXY_HEADER_BYTES: usize = 64 * 1024;

trait AsyncIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}
type BoxedIo = Box<dyn AsyncIo>;

#[derive(Clone)]
enum ProxyRouting {
    Direct,
    Custom(String),
    System,
}

impl ProxyRouting {
    fn from_config(config: &ProxyConfig) -> Self {
        match config.mode {
            ProxyMode::None => Self::Direct,
            ProxyMode::Custom => config
                .url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| Self::Custom(value.trim().to_string()))
                .unwrap_or(Self::Direct),
            ProxyMode::System => Self::System,
        }
    }

    fn upstream_for(&self, target: &Url) -> Result<Option<UpstreamProxy>> {
        let raw = match self {
            Self::Direct => return Ok(None),
            Self::Custom(raw) => Some(raw.clone()),
            Self::System => system_proxy_for(target),
        };
        raw.map(|raw| UpstreamProxy::parse(&raw)).transpose()
    }
}

#[derive(Clone)]
struct UpstreamProxy {
    url: Url,
}

impl UpstreamProxy {
    fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let url = Url::parse(raw)
            .or_else(|_| Url::parse(&format!("http://{raw}")))
            .context("invalid configured proxy URL")?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
            bail!("unsupported configured proxy scheme");
        }
        if url.host_str().is_none() {
            bail!("configured proxy URL has no host");
        }
        Ok(Self { url })
    }

    fn is_socks(&self) -> bool {
        matches!(self.url.scheme(), "socks5" | "socks5h")
    }

    fn authority(&self) -> Result<(String, u16)> {
        let host = self
            .url
            .host_str()
            .ok_or_else(|| anyhow!("configured proxy URL has no host"))?;
        let port = self
            .url
            .port_or_known_default()
            .ok_or_else(|| anyhow!("configured proxy URL has no port"))?;
        Ok((host.to_string(), port))
    }

    fn basic_auth_header(&self) -> Option<String> {
        if self.url.username().is_empty() && self.url.password().is_none() {
            return None;
        }
        let credentials = format!(
            "{}:{}",
            self.url.username(),
            self.url.password().unwrap_or_default()
        );
        Some(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        ))
    }
}

fn first_nonempty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

fn system_proxy_for(target: &Url) -> Option<String> {
    if target_bypasses_system_proxy(target) {
        return None;
    }
    let scheme_specific = if target.scheme() == "https" {
        first_nonempty_env(&["HTTPS_PROXY", "https_proxy"])
    } else {
        first_nonempty_env(&["HTTP_PROXY", "http_proxy"])
    };
    let all = first_nonempty_env(&["ALL_PROXY", "all_proxy"]);
    let has_any_env_proxy = first_nonempty_env(&[
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ])
    .is_some();
    if has_any_env_proxy {
        scheme_specific.or(all)
    } else {
        ha_core::platform::detect_system_proxy()
    }
}

fn target_bypasses_system_proxy(target: &Url) -> bool {
    let Some(raw) = first_nonempty_env(&["NO_PROXY", "no_proxy"]) else {
        return false;
    };
    target_matches_no_proxy(target, &raw)
}

fn target_matches_no_proxy(target: &Url, raw: &str) -> bool {
    let Some(host) = target.host_str() else {
        return false;
    };
    let port = target.port_or_known_default();
    raw.split(',').map(str::trim).any(|entry| {
        if entry.is_empty() {
            return false;
        }
        if entry == "*" {
            return true;
        }
        let (entry_host, entry_port) = split_no_proxy_entry(entry);
        if entry_port.is_some() && entry_port != port {
            return false;
        }
        if cidr_matches(host, entry_host) {
            return true;
        }
        let entry_host = entry_host
            .strip_prefix("*.")
            .unwrap_or(entry_host)
            .trim_start_matches('.');
        host.eq_ignore_ascii_case(entry_host)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", entry_host.to_ascii_lowercase()))
    })
}

fn cidr_matches(host: &str, cidr: &str) -> bool {
    let Some((network, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let (Ok(host), Ok(network), Ok(prefix)) = (
        host.parse::<IpAddr>(),
        network.parse::<IpAddr>(),
        prefix.parse::<u32>(),
    ) else {
        return false;
    };
    match (host, network) {
        (IpAddr::V4(host), IpAddr::V4(network)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(host) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(host), IpAddr::V6(network)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(host) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

fn split_no_proxy_entry(entry: &str) -> (&str, Option<u16>) {
    if let Some(bracketed) = entry.strip_prefix('[') {
        if let Some((host, port)) = bracketed.split_once("]:") {
            return port
                .parse()
                .map(|port| (host, Some(port)))
                .unwrap_or((entry, None));
        }
        return (bracketed.trim_end_matches(']'), None);
    }
    let Some((host, port)) = entry.rsplit_once(':') else {
        return (entry, None);
    };
    if host.contains(':') {
        (entry, None)
    } else {
        port.parse()
            .map(|port| (host, Some(port)))
            .unwrap_or((entry, None))
    }
}

#[derive(Clone)]
struct ProxyPolicy {
    ssrf_policy: SsrfPolicy,
    trusted_hosts: Vec<String>,
    routing: ProxyRouting,
}

pub struct RendererProxy {
    pub address: SocketAddr,
    pub task: JoinHandle<()>,
}

impl RendererProxy {
    pub async fn start(
        ssrf_policy: SsrfPolicy,
        trusted_hosts: Vec<String>,
        proxy_config: &ProxyConfig,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind isolated renderer proxy")?;
        let address = listener.local_addr()?;
        let policy = ProxyPolicy {
            ssrf_policy,
            trusted_hosts,
            routing: ProxyRouting::from_config(proxy_config),
        };
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let policy = policy.clone();
                        connections.spawn(async move {
                            let _ = handle_connection(stream, &policy).await;
                        });
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self { address, task })
    }
}

struct ParsedRequest {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
    remainder: Vec<u8>,
}

async fn read_request_head(stream: &mut TcpStream) -> Result<ParsedRequest> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if bytes.len() >= MAX_PROXY_HEADER_BYTES {
            bail!("renderer proxy request headers are too large");
        }
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("renderer proxy client closed before request headers");
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end])?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("renderer proxy request line is missing"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let target = parts.next().unwrap_or_default().to_string();
    let version = parts.next().unwrap_or_default().to_string();
    if method.is_empty()
        || target.is_empty()
        || !matches!(version.as_str(), "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
    {
        bail!("invalid renderer proxy request line");
    }
    let mut headers = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid renderer proxy header"))?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(ParsedRequest {
        method,
        target,
        version,
        headers,
        remainder: bytes[header_end + 4..].to_vec(),
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn handle_connection(mut downstream: TcpStream, policy: &ProxyPolicy) -> Result<()> {
    let request = match read_request_head(&mut downstream).await {
        Ok(request) => request,
        Err(error) => {
            write_proxy_error(&mut downstream, 400, "Bad Request").await;
            return Err(error);
        }
    };
    if request.method == "CONNECT" {
        return handle_connect(downstream, request, policy).await;
    }
    if !matches!(request.method.as_str(), "GET" | "HEAD" | "OPTIONS") || request_has_body(&request)
    {
        write_proxy_error(&mut downstream, 405, "Method Not Allowed").await;
        bail!("renderer proxy rejected a write-capable request");
    }
    handle_plain_http(downstream, request, policy).await
}

fn request_has_body(request: &ParsedRequest) -> bool {
    !request.remainder.is_empty()
        || request.headers.iter().any(|(name, value)| {
            (name.eq_ignore_ascii_case("content-length")
                && value.trim().parse::<u64>().unwrap_or(1) > 0)
                || name.eq_ignore_ascii_case("transfer-encoding")
        })
}

async fn handle_connect(
    mut downstream: TcpStream,
    request: ParsedRequest,
    policy: &ProxyPolicy,
) -> Result<()> {
    if request_has_body(&request) {
        write_proxy_error(&mut downstream, 400, "Bad Request").await;
        bail!("renderer proxy CONNECT request unexpectedly carried a body");
    }
    let target = parse_authority_url(&request.target, "https")?;
    let (addresses, upstream) = match checked_route(&target, policy).await {
        Ok(route) => route,
        Err(error) => {
            write_proxy_error(&mut downstream, 403, "Forbidden").await;
            return Err(error);
        }
    };
    let mut remote = match connect_tunnel(&addresses, upstream.as_ref()).await {
        Ok(remote) => remote,
        Err(error) => {
            write_proxy_error(&mut downstream, 502, "Bad Gateway").await;
            return Err(error);
        }
    };
    downstream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    let _ = tokio::io::copy_bidirectional(&mut downstream, &mut remote).await;
    Ok(())
}

async fn handle_plain_http(
    mut downstream: TcpStream,
    request: ParsedRequest,
    policy: &ProxyPolicy,
) -> Result<()> {
    let target = absolute_http_target(&request)?;
    if target.scheme() != "http" || !target.username().is_empty() || target.password().is_some() {
        write_proxy_error(&mut downstream, 400, "Bad Request").await;
        bail!("renderer proxy rejected an invalid HTTP target");
    }
    let (addresses, upstream) = match checked_route(&target, policy).await {
        Ok(route) => route,
        Err(error) => {
            write_proxy_error(&mut downstream, 403, "Forbidden").await;
            return Err(error);
        }
    };
    let address = *addresses
        .first()
        .ok_or_else(|| anyhow!("checked destination has no addresses"))?;
    let (mut remote, request_target, proxy_auth) = if let Some(upstream) = upstream.as_ref() {
        if upstream.is_socks() {
            let remote = connect_socks_destination(upstream, &addresses).await?;
            (remote, origin_form(&target), None)
        } else {
            let remote = connect_http_proxy(upstream).await?;
            (
                remote,
                pinned_absolute_url(&target, address)?,
                upstream.basic_auth_header(),
            )
        }
    } else {
        (
            Box::new(connect_approved_addresses(&addresses).await?) as BoxedIo,
            origin_form(&target),
            None,
        )
    };
    let rewritten = rewrite_request(&request, &target, &request_target, proxy_auth.as_deref());
    remote.write_all(rewritten.as_bytes()).await?;
    let _ = tokio::io::copy_bidirectional(&mut downstream, &mut remote).await;
    Ok(())
}

fn absolute_http_target(request: &ParsedRequest) -> Result<Url> {
    if let Ok(url) = Url::parse(&request.target) {
        return Ok(url);
    }
    if !request.target.starts_with('/') {
        bail!("renderer proxy HTTP target is not absolute");
    }
    let host = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| anyhow!("renderer proxy HTTP request has no Host header"))?;
    Url::parse(&format!("http://{host}{}", request.target)).context("invalid HTTP target")
}

fn parse_authority_url(authority: &str, scheme: &str) -> Result<Url> {
    let url = Url::parse(&format!("{scheme}://{authority}/"))?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        bail!("invalid renderer proxy authority");
    }
    Ok(url)
}

async fn checked_route(
    target: &Url,
    policy: &ProxyPolicy,
) -> Result<(Vec<SocketAddr>, Option<UpstreamProxy>)> {
    let host = target
        .host_str()
        .ok_or_else(|| anyhow!("renderer proxy target has no host"))?;
    let port = target
        .port_or_known_default()
        .ok_or_else(|| anyhow!("renderer proxy target has no port"))?;
    let addresses =
        resolve_checked_destination(host, port, policy.ssrf_policy, &policy.trusted_hosts).await?;
    let upstream = policy.routing.upstream_for(target)?;
    Ok((addresses, upstream))
}

async fn connect_approved_addresses(addresses: &[SocketAddr]) -> Result<TcpStream> {
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow!("checked destination has no addresses")))
}

async fn connect_proxy_socket(proxy: &UpstreamProxy) -> Result<TcpStream> {
    let (host, port) = proxy.authority()?;
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .context("resolve configured proxy")?
        .collect::<Vec<_>>();
    connect_approved_addresses(&addresses)
        .await
        .context("connect configured proxy")
}

async fn connect_http_proxy(proxy: &UpstreamProxy) -> Result<BoxedIo> {
    let stream = connect_proxy_socket(proxy).await?;
    if proxy.url.scheme() == "https" {
        let (host, _) = proxy.authority()?;
        let connector = tokio_native_tls::native_tls::TlsConnector::builder()
            .build()
            .context("build configured proxy TLS connector")?;
        let stream = tokio_native_tls::TlsConnector::from(connector)
            .connect(&host, stream)
            .await
            .context("establish configured HTTPS proxy TLS")?;
        Ok(Box::new(stream))
    } else {
        Ok(Box::new(stream))
    }
}

async fn connect_tunnel(
    addresses: &[SocketAddr],
    upstream: Option<&UpstreamProxy>,
) -> Result<BoxedIo> {
    let Some(upstream) = upstream else {
        return Ok(Box::new(connect_approved_addresses(addresses).await?));
    };
    if upstream.is_socks() {
        return connect_socks_destination(upstream, addresses).await;
    }
    let mut last_error = None;
    for address in addresses {
        match connect_http_tunnel(upstream, *address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("checked destination has no addresses")))
}

async fn connect_http_tunnel(proxy: &UpstreamProxy, address: SocketAddr) -> Result<BoxedIo> {
    let mut stream = connect_http_proxy(proxy).await?;
    let authority = socket_authority(address);
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(auth) = proxy.basic_auth_header() {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("Proxy-Connection: Keep-Alive\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let response = read_boxed_response_head(&mut stream).await?;
    let status = response
        .split("\r\n")
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        bail!("configured HTTP proxy rejected the tunnel");
    }
    Ok(stream)
}

async fn read_boxed_response_head(stream: &mut BoxedIo) -> Result<String> {
    let mut bytes = Vec::with_capacity(1024);
    loop {
        if bytes.len() >= MAX_PROXY_HEADER_BYTES {
            bail!("configured proxy response headers are too large");
        }
        let mut byte = [0_u8; 1];
        if stream.read(&mut byte).await? == 0 {
            bail!("configured proxy closed before tunnel response");
        }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes).context("configured proxy returned invalid headers");
        }
    }
}

async fn connect_socks_destination(
    proxy: &UpstreamProxy,
    addresses: &[SocketAddr],
) -> Result<BoxedIo> {
    let mut last_error = None;
    for address in addresses {
        match connect_one_socks_destination(proxy, *address).await {
            Ok(stream) => return Ok(Box::new(stream)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("checked destination has no addresses")))
}

async fn connect_one_socks_destination(
    proxy: &UpstreamProxy,
    address: SocketAddr,
) -> Result<TcpStream> {
    let mut stream = connect_proxy_socket(proxy).await?;
    let has_auth = !proxy.url.username().is_empty() || proxy.url.password().is_some();
    let methods: &[u8] = if has_auth { &[0x00, 0x02] } else { &[0x00] };
    let mut greeting = vec![0x05, methods.len() as u8];
    greeting.extend_from_slice(methods);
    stream.write_all(&greeting).await?;
    let mut selection = [0_u8; 2];
    stream.read_exact(&mut selection).await?;
    if selection[0] != 0x05 || selection[1] == 0xff {
        bail!("configured SOCKS5 proxy rejected authentication methods");
    }
    if selection[1] == 0x02 {
        let username = proxy.url.username().as_bytes();
        let password = proxy.url.password().unwrap_or_default().as_bytes();
        if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
            bail!("configured SOCKS5 proxy credentials are too long");
        }
        let mut auth = vec![0x01, username.len() as u8];
        auth.extend_from_slice(username);
        auth.push(password.len() as u8);
        auth.extend_from_slice(password);
        stream.write_all(&auth).await?;
        let mut reply = [0_u8; 2];
        stream.read_exact(&mut reply).await?;
        if reply != [0x01, 0x00] {
            bail!("configured SOCKS5 proxy authentication failed");
        }
    } else if selection[1] != 0x00 {
        bail!("configured SOCKS5 proxy selected an unsupported authentication method");
    }
    let mut connect = vec![0x05, 0x01, 0x00];
    match address.ip() {
        IpAddr::V4(ip) => {
            connect.push(0x01);
            connect.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            connect.push(0x04);
            connect.extend_from_slice(&ip.octets());
        }
    }
    connect.extend_from_slice(&address.port().to_be_bytes());
    stream.write_all(&connect).await?;
    let mut reply = [0_u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != 0x05 || reply[1] != 0x00 {
        bail!("configured SOCKS5 proxy rejected the destination");
    }
    let address_bytes = match reply[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            length[0] as usize
        }
        _ => bail!("configured SOCKS5 proxy returned an invalid address type"),
    };
    let mut ignored = vec![0_u8; address_bytes + 2];
    stream.read_exact(&mut ignored).await?;
    Ok(stream)
}

fn origin_form(target: &Url) -> String {
    let mut value = target.path().to_string();
    if value.is_empty() {
        value.push('/');
    }
    if let Some(query) = target.query() {
        value.push('?');
        value.push_str(query);
    }
    value
}

fn pinned_absolute_url(target: &Url, address: SocketAddr) -> Result<String> {
    let mut pinned = target.clone();
    pinned.set_host(Some(&address.ip().to_string()))?;
    pinned
        .set_port(Some(address.port()))
        .map_err(|_| anyhow!("invalid checked destination port"))?;
    pinned.set_fragment(None);
    Ok(pinned.to_string())
}

fn rewrite_request(
    request: &ParsedRequest,
    target: &Url,
    request_target: &str,
    proxy_auth: Option<&str>,
) -> String {
    let mut rewritten = format!(
        "{} {} {}\r\nHost: {}\r\nConnection: close\r\n",
        request.method,
        request_target,
        request.version,
        url_authority(target)
    );
    for (name, value) in &request.headers {
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("proxy-authorization")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        rewritten.push_str(name);
        rewritten.push_str(": ");
        rewritten.push_str(value);
        rewritten.push_str("\r\n");
    }
    if let Some(auth) = proxy_auth {
        rewritten.push_str("Proxy-Authorization: ");
        rewritten.push_str(auth);
        rewritten.push_str("\r\n");
    }
    rewritten.push_str("\r\n");
    rewritten
}

fn url_authority(url: &Url) -> String {
    let host = match url.host() {
        Some(url::Host::Ipv6(ip)) => format!("[{ip}]"),
        Some(host) => host.to_string(),
        None => String::new(),
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn socket_authority(address: SocketAddr) -> String {
    address.to_string()
}

async fn write_proxy_error(stream: &mut TcpStream, status: u16, reason: &str) {
    let response =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_matches_exact_suffix_and_port() {
        let exact = Url::parse("https://api.example.com:8443/").unwrap();
        assert!(target_matches_no_proxy(
            &exact,
            "localhost,.example.com:8443"
        ));
        let other_port = Url::parse("https://api.example.com:9443/").unwrap();
        assert!(!target_matches_no_proxy(
            &other_port,
            "localhost,.example.com:8443"
        ));
        let private = Url::parse("http://10.2.3.4/").unwrap();
        assert!(target_matches_no_proxy(&private, "10.0.0.0/8"));
    }

    #[test]
    fn request_rewrite_keeps_origin_host_while_pinning_proxy_target() {
        let target = Url::parse("http://example.com/path?q=1").unwrap();
        let request = ParsedRequest {
            method: "GET".into(),
            target: target.to_string(),
            version: "HTTP/1.1".into(),
            headers: vec![
                ("Host".into(), "attacker.invalid".into()),
                ("Accept".into(), "text/html".into()),
                ("Proxy-Authorization".into(), "untrusted".into()),
            ],
            remainder: Vec::new(),
        };
        let pinned = pinned_absolute_url(&target, "203.0.113.7:80".parse().unwrap()).unwrap();
        let rewritten = rewrite_request(&request, &target, &pinned, Some("Basic safe"));
        assert!(rewritten.starts_with("GET http://203.0.113.7/path?q=1 HTTP/1.1\r\n"));
        assert!(rewritten.contains("\r\nHost: example.com\r\n"));
        assert!(rewritten.contains("\r\nProxy-Authorization: Basic safe\r\n"));
        assert!(!rewritten.contains("attacker.invalid"));
        assert!(!rewritten.contains("untrusted"));
    }

    #[tokio::test]
    async fn strict_policy_blocks_before_the_destination_socket_is_opened() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination_address = destination.local_addr().unwrap();
        let config = ProxyConfig {
            mode: ProxyMode::None,
            url: None,
        };
        let proxy = RendererProxy::start(SsrfPolicy::Strict, Vec::new(), &config)
            .await
            .unwrap();
        let mut client = TcpStream::connect(proxy.address).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://{destination_address}/private HTTP/1.1\r\nHost: {destination_address}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = [0_u8; 128];
        let read = client.read(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response[..read]).contains("403 Forbidden"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), destination.accept())
                .await
                .is_err(),
            "blocked target must not receive a connection"
        );
        proxy.task.abort();
    }

    #[tokio::test]
    async fn custom_proxy_receives_a_pinned_target_and_original_host() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                bytes.extend_from_slice(&chunk[..read]);
                if read == 0 || find_header_end(&bytes).is_some() {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let config = ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(format!("http://user:pass@{upstream_address}")),
        };
        let proxy =
            RendererProxy::start(SsrfPolicy::Strict, vec!["127.0.0.1:4317".into()], &config)
                .await
                .unwrap();
        let mut client = TcpStream::connect(proxy.address).await.unwrap();
        client
            .write_all(
                b"GET http://127.0.0.1:4317/metrics?q=1 HTTP/1.1\r\nHost: attacker.invalid\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        let response_text = String::from_utf8_lossy(&response);
        assert!(
            response_text.contains("200 OK"),
            "unexpected renderer proxy response: {response_text}"
        );

        let forwarded = upstream_task.await.unwrap();
        let request_line = forwarded.lines().next().unwrap_or_default();
        assert!(request_line.starts_with("GET http://127.0.0.1:4317/metrics?q=1 HTTP/1.1"));
        assert!(forwarded.contains("\r\nHost: 127.0.0.1:4317\r\n"));
        assert!(forwarded.contains("\r\nProxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
        assert!(!forwarded.contains("attacker.invalid"));
        proxy.task.abort();
    }
}
