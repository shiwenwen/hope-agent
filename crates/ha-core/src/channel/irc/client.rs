use anyhow::{anyhow, Result};
use base64::Engine as _;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::protocol::{extract_nick, parse_irc_line};
use crate::channel::types::*;
use crate::channel::ws::BACKOFF_SECS;

/// Wrapper for the write half of either a plain or TLS TCP stream.
enum IrcWriter {
    Plain(WriteHalf<TcpStream>),
    Tls(WriteHalf<tokio_native_tls::TlsStream<TcpStream>>),
}

impl IrcWriter {
    async fn write_all(&mut self, buf: &[u8]) -> tokio::io::Result<()> {
        match self {
            IrcWriter::Plain(w) => w.write_all(buf).await,
            IrcWriter::Tls(w) => w.write_all(buf).await,
        }
    }

    async fn flush(&mut self) -> tokio::io::Result<()> {
        match self {
            IrcWriter::Plain(w) => w.flush().await,
            IrcWriter::Tls(w) => w.flush().await,
        }
    }
}

/// Wrapper for the read half of either a plain or TLS TCP stream.
enum IrcReader {
    Plain(BufReader<ReadHalf<TcpStream>>),
    Tls(BufReader<ReadHalf<tokio_native_tls::TlsStream<TcpStream>>>),
}

impl IrcReader {
    async fn read_line(&mut self, buf: &mut String) -> tokio::io::Result<usize> {
        match self {
            IrcReader::Plain(r) => r.read_line(buf).await,
            IrcReader::Tls(r) => r.read_line(buf).await,
        }
    }
}

/// IRC client managing a TCP/TLS connection to an IRC server.
pub struct IrcClient {
    writer: Arc<Mutex<IrcWriter>>,
    reader_task: Option<JoinHandle<()>>,
    nick: String,
}

/// IRC connection credentials.
#[derive(Clone)]
pub struct IrcCredentials {
    pub server: String,
    pub port: u16,
    pub tls: bool,
    pub nick: String,
    pub username: String,
    pub realname: String,
    pub password: Option<String>,
    pub nickserv_password: Option<String>,
    pub sasl_username: Option<String>,
    pub sasl_password: Option<String>,
    pub channels: Vec<String>,
}

impl IrcCredentials {
    fn sasl_plain_credentials(&self) -> Option<(&str, &str)> {
        let password = self.sasl_password.as_deref()?;
        let username = self.sasl_username.as_deref().unwrap_or(self.nick.as_str());
        Some((username, password))
    }
}

struct RegistrationResult {
    nick: String,
    sasl_authenticated: bool,
}

struct Capability {
    name: String,
    value: Option<String>,
}

fn parse_capabilities(lines: &[String]) -> Vec<Capability> {
    lines
        .iter()
        .flat_map(|line| line.split_whitespace())
        .map(|token| {
            let (name, value) = token
                .split_once('=')
                .map(|(name, value)| (name, Some(value)))
                .unwrap_or((token, None));
            Capability {
                name: name.to_string(),
                value: value.map(str::to_string),
            }
        })
        .collect()
}

fn parse_cap_names(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|token| token.trim_start_matches('-'))
        .map(|token| token.split_once('=').map(|(name, _)| name).unwrap_or(token))
        .map(str::to_string)
        .collect()
}

impl IrcClient {
    /// Connect to an IRC server and perform registration.
    ///
    /// Returns the client and a reader that can be used to spawn the event loop.
    async fn connect_raw(
        server: &str,
        port: u16,
        tls: bool,
    ) -> Result<(Arc<Mutex<IrcWriter>>, IrcReader)> {
        let tcp = TcpStream::connect((server, port))
            .await
            .map_err(|e| anyhow!("IRC TCP connect to {}:{} failed: {}", server, port, e))?;

        if tls {
            let connector = native_tls::TlsConnector::new()
                .map_err(|e| anyhow!("TLS connector creation failed: {}", e))?;
            let connector = tokio_native_tls::TlsConnector::from(connector);
            let tls_stream = connector
                .connect(server, tcp)
                .await
                .map_err(|e| anyhow!("TLS handshake with {} failed: {}", server, e))?;

            let (read_half, write_half) = tokio::io::split(tls_stream);
            Ok((
                Arc::new(Mutex::new(IrcWriter::Tls(write_half))),
                IrcReader::Tls(BufReader::new(read_half)),
            ))
        } else {
            let (read_half, write_half) = tokio::io::split(tcp);
            Ok((
                Arc::new(Mutex::new(IrcWriter::Plain(write_half))),
                IrcReader::Plain(BufReader::new(read_half)),
            ))
        }
    }

    /// Send a raw IRC line (appends \r\n).
    async fn send_raw_with(writer: &Mutex<IrcWriter>, line: &str) -> Result<()> {
        let cleaned = line.replace(['\r', '\n'], "");
        let mut w = writer.lock().await;
        w.write_all(format!("{}\r\n", cleaned).as_bytes()).await?;
        w.flush().await?;
        Ok(())
    }

    /// Send a raw IRC command.
    pub async fn send_raw(&self, line: &str) -> Result<()> {
        Self::send_raw_with(&self.writer, line).await
    }

    async fn register_connection(
        writer: &Mutex<IrcWriter>,
        reader: &mut IrcReader,
        creds: &IrcCredentials,
    ) -> Result<RegistrationResult> {
        if let Some(ref pass) = creds.password {
            if !pass.is_empty() {
                Self::send_raw_with(writer, &format!("PASS {}", pass)).await?;
            }
        }

        Self::send_raw_with(writer, "CAP LS 302").await?;
        Self::send_raw_with(writer, &format!("NICK {}", creds.nick)).await?;
        Self::send_raw_with(
            writer,
            &format!("USER {} 0 * :{}", creds.username, creds.realname),
        )
        .await?;

        Self::wait_for_registration(writer, reader, creds).await
    }

    async fn send_sasl_plain(
        writer: &Mutex<IrcWriter>,
        username: &str,
        password: &str,
    ) -> Result<()> {
        let mut payload = Vec::with_capacity(username.len() * 2 + password.len() + 2);
        payload.extend_from_slice(username.as_bytes());
        payload.push(0);
        payload.extend_from_slice(username.as_bytes());
        payload.push(0);
        payload.extend_from_slice(password.as_bytes());

        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);

        // IRCv3 SASL caps each AUTHENTICATE parameter at 400 bytes. A final
        // exactly-400-byte chunk must be followed by AUTHENTICATE +.
        for chunk in encoded.as_bytes().chunks(400) {
            let chunk = std::str::from_utf8(chunk)?;
            Self::send_raw_with(writer, &format!("AUTHENTICATE {}", chunk)).await?;
        }
        if encoded.len() % 400 == 0 {
            Self::send_raw_with(writer, "AUTHENTICATE +").await?;
        }

        Ok(())
    }

    /// Send PRIVMSG to a target (channel or nick).
    pub async fn send_privmsg(&self, target: &str, text: &str) -> Result<()> {
        // RFC 2812 整行 ≤ 512 字节（含 CRLF）。服务端附加的 prefix
        // `:nick!user@host PRIVMSG <target> :` ≈ 100 + 9 + target.len() + 4
        // 字节；剩下的才是 text 上限。最少留 64 字节兜底（CJK 21 字符）。
        let sanitized = text.replace(['\r', '\n'], " ");
        let overhead = 100 + 9 + target.len() + 2 + 2;
        let max_text_len = 512usize.saturating_sub(overhead).max(64);
        let mut remaining = sanitized.as_str();

        while !remaining.is_empty() {
            let chunk = crate::truncate_utf8(remaining, max_text_len);
            if chunk.is_empty() {
                break;
            }
            self.send_raw(&format!("PRIVMSG {} :{}", target, chunk))
                .await?;
            remaining = remaining[chunk.len()..].trim_start();
        }
        Ok(())
    }

    /// Close the connection gracefully.
    pub async fn close(&mut self) {
        let _ = self.send_raw("QUIT :Goodbye").await;
        if let Some(task) = self.reader_task.take() {
            task.abort();
        }
    }

    /// Connect to an IRC server, register, and spawn the event loop.
    ///
    /// The event loop reads lines, handles PING/PONG, converts PRIVMSG
    /// into MsgContext, and reconnects on disconnect with exponential backoff.
    pub async fn connect_and_run(
        creds: IrcCredentials,
        account_id: String,
        inbound_tx: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let (writer, mut reader) = Self::connect_raw(&creds.server, creds.port, creds.tls).await?;

        let registration = Self::register_connection(&writer, &mut reader, &creds).await?;
        let confirmed_nick = registration.nick;

        // NickServ fallback for networks without SASL support.
        if !registration.sasl_authenticated {
            if let Some(ref ns_pass) = creds.nickserv_password {
                if !ns_pass.is_empty() {
                    Self::send_raw_with(
                        &writer,
                        &format!("PRIVMSG NickServ :IDENTIFY {}", ns_pass),
                    )
                    .await?;
                }
            }
        }

        // Join channels
        for channel in &creds.channels {
            let trimmed = channel.trim();
            if !trimmed.is_empty() {
                Self::send_raw_with(&writer, &format!("JOIN {}", trimmed)).await?;
            }
        }

        app_info!(
            "channel",
            "irc",
            "Connected to {}:{} as {}",
            creds.server,
            creds.port,
            confirmed_nick
        );

        let nick = confirmed_nick.clone();
        let writer_clone = writer.clone();

        // Spawn the event loop
        let reader_task = tokio::spawn(Self::event_loop(
            reader,
            writer_clone,
            creds,
            account_id,
            confirmed_nick,
            inbound_tx,
            cancel,
        ));

        Ok(Self {
            writer,
            reader_task: Some(reader_task),
            nick,
        })
    }

    /// Wait for CAP negotiation, optional SASL, and RPL_WELCOME (001).
    async fn wait_for_registration(
        writer: &Mutex<IrcWriter>,
        reader: &mut IrcReader,
        creds: &IrcCredentials,
    ) -> Result<RegistrationResult> {
        let mut nick = creds.nick.clone();
        let mut cap_ls = Vec::new();
        let mut requested_sasl = false;
        let mut sasl_authenticated = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "IRC registration timed out (no RPL_WELCOME within 30s)"
                ));
            }

            let mut line_buf = String::new();
            let read_result =
                tokio::time::timeout_at(deadline, reader.read_line(&mut line_buf)).await;

            match read_result {
                Ok(Ok(0)) => {
                    return Err(anyhow!("IRC connection closed during registration"));
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    return Err(anyhow!("IRC read error during registration: {}", e));
                }
                Err(_) => {
                    return Err(anyhow!("IRC registration timed out"));
                }
            }

            let line = line_buf.trim_end().to_string();
            if line.is_empty() {
                continue;
            }

            let Some(msg) = parse_irc_line(&line) else {
                continue;
            };

            match msg.command.as_str() {
                "CAP" => {
                    let Some(subcommand) = msg.params.get(1).map(|s| s.to_uppercase()) else {
                        continue;
                    };
                    let is_continuation = msg.params.get(2).is_some_and(|param| param == "*");
                    let payload_idx = if is_continuation { 3 } else { 2 };
                    let payload = msg
                        .params
                        .get(payload_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");

                    match subcommand.as_str() {
                        "LS" => {
                            cap_ls.push(payload.to_string());
                            if is_continuation {
                                continue;
                            }

                            let requested_caps = Self::desired_capabilities(&cap_ls, creds);
                            requested_sasl = requested_caps.iter().any(|cap| cap == "sasl");

                            if requested_caps.is_empty() {
                                let _ = Self::send_raw_with(writer, "CAP END").await;
                            } else {
                                let cap_list = requested_caps.join(" ");
                                let _ =
                                    Self::send_raw_with(writer, &format!("CAP REQ :{}", cap_list))
                                        .await;
                            }
                        }
                        "ACK" => {
                            let acked_caps = parse_cap_names(payload);
                            if requested_sasl && acked_caps.iter().any(|cap| cap == "sasl") {
                                let _ = Self::send_raw_with(writer, "AUTHENTICATE PLAIN").await;
                            } else {
                                let _ = Self::send_raw_with(writer, "CAP END").await;
                            }
                        }
                        "NAK" => {
                            let _ = Self::send_raw_with(writer, "CAP END").await;
                            if requested_sasl {
                                return Err(anyhow!("IRC server rejected SASL capability request"));
                            }
                        }
                        _ => {}
                    }
                }
                "AUTHENTICATE" => {
                    if !requested_sasl {
                        continue;
                    }
                    let challenge = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    if challenge != "+" {
                        let _ = Self::send_raw_with(writer, "AUTHENTICATE *").await;
                        let _ = Self::send_raw_with(writer, "CAP END").await;
                        return Err(anyhow!("IRC server sent unexpected SASL PLAIN challenge"));
                    }

                    if let Some((username, password)) = creds.sasl_plain_credentials() {
                        Self::send_sasl_plain(writer, username, password).await?;
                    } else {
                        let _ = Self::send_raw_with(writer, "AUTHENTICATE *").await;
                        let _ = Self::send_raw_with(writer, "CAP END").await;
                        return Err(anyhow!("IRC SASL requested without credentials"));
                    }
                }
                "PING" => {
                    let payload = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    let _ = Self::send_raw_with(writer, &format!("PONG :{}", payload)).await;
                }
                // RPL_LOGGEDIN
                "900" => {}
                // RPL_SASLSUCCESS
                "903" => {
                    sasl_authenticated = true;
                    let _ = Self::send_raw_with(writer, "CAP END").await;
                }
                // ERR_NICKLOCKED, ERR_SASLFAIL, ERR_SASLTOOLONG, ERR_SASLABORTED
                "902" | "904" | "905" | "906" => {
                    let detail = msg
                        .params
                        .last()
                        .cloned()
                        .unwrap_or_else(|| "SASL authentication failed".to_string());
                    let _ = Self::send_raw_with(writer, "CAP END").await;
                    return Err(anyhow!("IRC SASL failed ({}): {}", msg.command, detail));
                }
                // ERR_SASLALREADY: treat as authenticated and continue registration.
                "907" => {
                    sasl_authenticated = true;
                    let _ = Self::send_raw_with(writer, "CAP END").await;
                }
                // RPL_WELCOME
                "001" => {
                    if let Some(param) = msg.params.first() {
                        let param = param.trim();
                        if !param.is_empty() {
                            nick = param.to_string();
                        }
                    }
                    return Ok(RegistrationResult {
                        nick,
                        sasl_authenticated,
                    });
                }
                // ERR_ERRONEUSNICKNAME, ERR_PASSWDMISMATCH, ERR_YOUREBANNEDCREEP
                "432" | "464" | "465" => {
                    let detail = msg
                        .params
                        .last()
                        .cloned()
                        .unwrap_or_else(|| "login rejected".to_string());
                    return Err(anyhow!("IRC login failed ({}): {}", msg.command, detail));
                }
                // ERR_NICKNAMEINUSE
                "433" => {
                    // Try fallback nick
                    let fallback = format!("{}_", nick);
                    app_warn!(
                        "channel",
                        "irc",
                        "Nick '{}' in use, trying fallback '{}'",
                        nick,
                        fallback
                    );
                    nick = fallback.clone();
                    let _ = Self::send_raw_with(writer, &format!("NICK {}", fallback)).await;
                }
                _ => {
                    // Ignore other messages during registration
                }
            }
        }
    }

    fn desired_capabilities(cap_lines: &[String], creds: &IrcCredentials) -> Vec<String> {
        let mut caps = Vec::new();
        let available = parse_capabilities(cap_lines);

        if available.iter().any(|cap| cap.name == "message-tags") {
            caps.push("message-tags".to_string());
        }

        if creds.sasl_plain_credentials().is_some()
            && available.iter().any(|cap| {
                cap.name == "sasl"
                    && match cap.value.as_deref() {
                        Some(value) => value
                            .split(',')
                            .any(|mechanism| mechanism.eq_ignore_ascii_case("PLAIN")),
                        None => true,
                    }
            })
        {
            caps.push("sasl".to_string());
        }

        caps
    }

    /// Main event loop: reads IRC lines, handles PING, dispatches PRIVMSG.
    /// On disconnect, performs exponential backoff reconnection.
    async fn event_loop(
        mut reader: IrcReader,
        writer: Arc<Mutex<IrcWriter>>,
        creds: IrcCredentials,
        account_id: String,
        mut current_nick: String,
        inbound_tx: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
    ) {
        let mut attempt: usize = 0;

        loop {
            // Read lines from the current connection
            let disconnect_reason = Self::read_loop(
                &mut reader,
                &writer,
                &account_id,
                &mut current_nick,
                &inbound_tx,
                &cancel,
            )
            .await;

            if cancel.is_cancelled() {
                app_info!(
                    "channel",
                    "irc",
                    "Event loop cancelled for account '{}'",
                    account_id
                );
                return;
            }

            app_warn!(
                "channel",
                "irc",
                "Disconnected from IRC (account '{}'): {}",
                account_id,
                disconnect_reason
            );

            // Exponential backoff reconnect
            let delay_secs = BACKOFF_SECS
                .get(attempt)
                .copied()
                .unwrap_or_else(|| BACKOFF_SECS.last().copied().unwrap_or(60));
            attempt += 1;

            app_info!(
                "channel",
                "irc",
                "Reconnecting in {}s (attempt {}) for account '{}'",
                delay_secs,
                attempt,
                account_id
            );

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay_secs)) => {}
                _ = cancel.cancelled() => {
                    app_info!("channel", "irc", "Reconnect cancelled for account '{}'", account_id);
                    return;
                }
            }

            // Attempt reconnection
            match Self::connect_raw(&creds.server, creds.port, creds.tls).await {
                Ok((new_writer, new_reader)) => {
                    reader = new_reader;

                    match Self::register_connection(&new_writer, &mut reader, &creds).await {
                        Ok(registration) => {
                            current_nick = registration.nick;

                            // NickServ fallback for networks without SASL support.
                            if !registration.sasl_authenticated {
                                if let Some(ref ns_pass) = creds.nickserv_password {
                                    if !ns_pass.is_empty() {
                                        let _ = Self::send_raw_with(
                                            &new_writer,
                                            &format!("PRIVMSG NickServ :IDENTIFY {}", ns_pass),
                                        )
                                        .await;
                                    }
                                }
                            }

                            // Re-join channels
                            for channel in &creds.channels {
                                let trimmed = channel.trim();
                                if !trimmed.is_empty() {
                                    let _ = Self::send_raw_with(
                                        &new_writer,
                                        &format!("JOIN {}", trimmed),
                                    )
                                    .await;
                                }
                            }

                            // Replace the writer
                            {
                                let mut w = writer.lock().await;
                                let mut new_w = new_writer.lock().await;
                                std::mem::swap(&mut *w, &mut *new_w);
                            }

                            attempt = 0; // Reset backoff on successful reconnect
                            app_info!(
                                "channel",
                                "irc",
                                "Reconnected to {}:{} as {} (account '{}')",
                                creds.server,
                                creds.port,
                                current_nick,
                                account_id
                            );
                        }
                        Err(e) => {
                            app_error!(
                                "channel",
                                "irc",
                                "Reconnect registration failed for '{}': {}",
                                account_id,
                                e
                            );
                            // Will retry on next loop iteration
                        }
                    }
                }
                Err(e) => {
                    app_error!(
                        "channel",
                        "irc",
                        "Reconnect TCP failed for '{}': {}",
                        account_id,
                        e
                    );
                }
            }
        }
    }

    /// Inner read loop: processes lines until disconnection.
    /// Returns the reason for disconnection.
    async fn read_loop(
        reader: &mut IrcReader,
        writer: &Mutex<IrcWriter>,
        account_id: &str,
        current_nick: &mut String,
        inbound_tx: &mpsc::Sender<InboundEvent>,
        cancel: &CancellationToken,
    ) -> String {
        loop {
            let mut line_buf = String::new();
            let read_result = tokio::select! {
                result = reader.read_line(&mut line_buf) => result,
                _ = cancel.cancelled() => {
                    return "cancelled".to_string();
                }
            };

            match read_result {
                Ok(0) => {
                    return "connection closed (EOF)".to_string();
                }
                Ok(_) => {}
                Err(e) => {
                    return format!("read error: {}", e);
                }
            }

            let line = line_buf.trim_end().to_string();
            if line.is_empty() {
                continue;
            }

            let Some(msg) = parse_irc_line(&line) else {
                continue;
            };

            match msg.command.as_str() {
                "PING" => {
                    let payload = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    let _ = Self::send_raw_with(writer, &format!("PONG :{}", payload)).await;
                }
                "PRIVMSG" => {
                    let target = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    let text = msg.params.get(1).map(|s| s.as_str()).unwrap_or("");
                    let sender_prefix = msg.prefix.as_deref().unwrap_or("");
                    let sender_nick = extract_nick(sender_prefix);

                    if sender_nick.is_empty() || target.is_empty() || text.trim().is_empty() {
                        continue;
                    }

                    // Skip messages from ourselves
                    if sender_nick.eq_ignore_ascii_case(current_nick) {
                        continue;
                    }

                    // Skip CTCP messages (except ACTION)
                    if text.starts_with('\x01') && !text.starts_with("\x01ACTION") {
                        continue;
                    }

                    // Determine chat type and chat_id
                    let (chat_type, chat_id) = if target.starts_with('#') || target.starts_with('&')
                    {
                        (ChatType::Group, target.to_string())
                    } else {
                        // DM: chat_id is the sender's nick
                        (ChatType::Dm, sender_nick.to_string())
                    };

                    // Check if bot was mentioned
                    let was_mentioned = text.to_lowercase().contains(&current_nick.to_lowercase());
                    let timestamp = msg
                        .tags
                        .get("time")
                        .and_then(|value| value.as_deref())
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now);

                    let msg_ctx = MsgContext {
                        channel_id: ChannelId::Irc,
                        account_id: account_id.to_string(),
                        sender_id: sender_nick.to_string(),
                        sender_name: Some(sender_nick.to_string()),
                        sender_username: Some(sender_nick.to_string()),
                        chat_id,
                        chat_type,
                        chat_title: if target.starts_with('#') || target.starts_with('&') {
                            Some(target.to_string())
                        } else {
                            None
                        },
                        thread_id: None,
                        message_id: uuid::Uuid::new_v4().to_string(),
                        text: Some(text.to_string()),
                        media: Vec::new(),
                        reply_to_message_id: None,
                        timestamp,
                        was_mentioned,
                        raw: serde_json::json!({ "line": line, "tags": msg.tags }),
                    };

                    if inbound_tx
                        .send(InboundEvent::Message(msg_ctx))
                        .await
                        .is_err()
                    {
                        app_warn!(
                            "channel",
                            "irc",
                            "Inbound channel closed for account '{}'",
                            account_id
                        );
                        return "inbound channel closed".to_string();
                    }
                }
                "NICK" => {
                    // Track our own nick changes
                    if let Some(ref prefix) = msg.prefix {
                        let old_nick = extract_nick(prefix);
                        if old_nick.eq_ignore_ascii_case(current_nick) {
                            if let Some(new_nick) = msg.params.first() {
                                let new_nick = new_nick.trim();
                                if !new_nick.is_empty() {
                                    *current_nick = new_nick.to_string();
                                    app_info!(
                                        "channel",
                                        "irc",
                                        "Nick changed to '{}' for account '{}'",
                                        current_nick,
                                        account_id
                                    );
                                }
                            }
                        }
                    }
                }
                // Error codes that indicate severe issues
                "432" | "433" | "464" | "465" => {
                    let detail = msg
                        .params
                        .last()
                        .cloned()
                        .unwrap_or_else(|| "unknown error".to_string());
                    app_error!(
                        "channel",
                        "irc",
                        "IRC error {} for account '{}': {}",
                        msg.command,
                        account_id,
                        detail
                    );
                }
                "ERROR" => {
                    let detail = msg
                        .params
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    return format!("server ERROR: {}", detail);
                }
                _ => {
                    // Ignore other messages (JOIN, PART, MODE, NOTICE, numerics, etc.)
                }
            }
        }
    }

    /// Get the current nick.
    pub fn nick(&self) -> &str {
        &self.nick
    }

    /// Probe an IRC server by connecting and waiting for RPL_WELCOME.
    /// Returns the confirmed nick on success.
    pub async fn probe(creds: &IrcCredentials) -> Result<String> {
        let (writer, mut reader) = Self::connect_raw(&creds.server, creds.port, creds.tls).await?;

        let registration = Self::register_connection(&writer, &mut reader, creds).await?;
        let nick = registration.nick;

        // Send QUIT
        let _ = Self::send_raw_with(&writer, "QUIT :probe").await;

        Ok(nick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds_with_sasl() -> IrcCredentials {
        IrcCredentials {
            server: "irc.example.test".to_string(),
            port: 6697,
            tls: true,
            nick: "hopebot".to_string(),
            username: "hopebot".to_string(),
            realname: "Hope Bot".to_string(),
            password: None,
            nickserv_password: Some("secret".to_string()),
            sasl_username: None,
            sasl_password: Some("secret".to_string()),
            channels: vec!["#hope".to_string()],
        }
    }

    #[test]
    fn desired_capabilities_request_tags_and_sasl_plain() {
        let caps = IrcClient::desired_capabilities(
            &[String::from(
                "multi-prefix message-tags sasl=PLAIN,EXTERNAL",
            )],
            &creds_with_sasl(),
        );

        assert_eq!(caps, vec!["message-tags", "sasl"]);
    }

    #[test]
    fn desired_capabilities_skip_sasl_without_plain() {
        let caps = IrcClient::desired_capabilities(
            &[String::from("message-tags sasl=EXTERNAL")],
            &creds_with_sasl(),
        );

        assert_eq!(caps, vec!["message-tags"]);
    }

    #[test]
    fn parse_capabilities_handles_values_and_multiline() {
        let caps = parse_capabilities(&[
            String::from("multi-prefix sasl=PLAIN,EXTERNAL"),
            String::from("message-tags"),
        ]);

        assert!(caps
            .iter()
            .any(|cap| cap.name == "sasl" && cap.value.as_deref() == Some("PLAIN,EXTERNAL")));
        assert!(caps.iter().any(|cap| cap.name == "message-tags"));
    }
}
