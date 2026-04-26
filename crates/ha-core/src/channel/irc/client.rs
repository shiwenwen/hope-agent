use anyhow::{anyhow, Result};
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
    pub channels: Vec<String>,
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

    /// Send PRIVMSG to a target (channel or nick).
    pub async fn send_privmsg(&self, target: &str, text: &str) -> Result<()> {
        // IRC messages have a max of ~512 bytes including the protocol overhead.
        // Split long messages into chunks.
        let sanitized = text.replace(['\r', '\n'], " ");
        let max_text_len = 400; // Leave room for PRIVMSG command + target + overhead
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
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let (writer, mut reader) = Self::connect_raw(&creds.server, creds.port, creds.tls).await?;

        // Register with the server
        if let Some(ref pass) = creds.password {
            if !pass.is_empty() {
                Self::send_raw_with(&writer, &format!("PASS {}", pass)).await?;
            }
        }
        Self::send_raw_with(&writer, &format!("NICK {}", creds.nick)).await?;
        Self::send_raw_with(
            &writer,
            &format!("USER {} 0 * :{}", creds.username, creds.realname),
        )
        .await?;

        // Wait for RPL_WELCOME (001) or error
        let confirmed_nick = Self::wait_for_welcome(&writer, &mut reader, &creds.nick).await?;

        // NickServ identification
        if let Some(ref ns_pass) = creds.nickserv_password {
            if !ns_pass.is_empty() {
                Self::send_raw_with(&writer, &format!("PRIVMSG NickServ :IDENTIFY {}", ns_pass))
                    .await?;
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

    /// Wait for RPL_WELCOME (001) from the server.
    async fn wait_for_welcome(
        writer: &Mutex<IrcWriter>,
        reader: &mut IrcReader,
        desired_nick: &str,
    ) -> Result<String> {
        let mut nick = desired_nick.to_string();
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
                "PING" => {
                    let payload = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                    let _ = Self::send_raw_with(writer, &format!("PONG :{}", payload)).await;
                }
                // RPL_WELCOME
                "001" => {
                    if let Some(param) = msg.params.first() {
                        let param = param.trim();
                        if !param.is_empty() {
                            nick = param.to_string();
                        }
                    }
                    return Ok(nick);
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

    /// Main event loop: reads IRC lines, handles PING, dispatches PRIVMSG.
    /// On disconnect, performs exponential backoff reconnection.
    async fn event_loop(
        mut reader: IrcReader,
        writer: Arc<Mutex<IrcWriter>>,
        creds: IrcCredentials,
        account_id: String,
        mut current_nick: String,
        inbound_tx: mpsc::Sender<MsgContext>,
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
                    // Re-register
                    if let Some(ref pass) = creds.password {
                        if !pass.is_empty() {
                            let _ =
                                Self::send_raw_with(&new_writer, &format!("PASS {}", pass)).await;
                        }
                    }
                    let _ = Self::send_raw_with(&new_writer, &format!("NICK {}", creds.nick)).await;
                    let _ = Self::send_raw_with(
                        &new_writer,
                        &format!("USER {} 0 * :{}", creds.username, creds.realname),
                    )
                    .await;

                    reader = new_reader;

                    // Wait for welcome on new connection
                    match Self::wait_for_welcome(&new_writer, &mut reader, &creds.nick).await {
                        Ok(nick) => {
                            current_nick = nick;

                            // NickServ re-identify
                            if let Some(ref ns_pass) = creds.nickserv_password {
                                if !ns_pass.is_empty() {
                                    let _ = Self::send_raw_with(
                                        &new_writer,
                                        &format!("PRIVMSG NickServ :IDENTIFY {}", ns_pass),
                                    )
                                    .await;
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
        inbound_tx: &mpsc::Sender<MsgContext>,
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
                        timestamp: chrono::Utc::now(),
                        was_mentioned,
                        raw: serde_json::json!({ "line": line }),
                    };

                    if inbound_tx.send(msg_ctx).await.is_err() {
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

        if let Some(ref pass) = creds.password {
            if !pass.is_empty() {
                Self::send_raw_with(&writer, &format!("PASS {}", pass)).await?;
            }
        }
        Self::send_raw_with(&writer, &format!("NICK {}", creds.nick)).await?;
        Self::send_raw_with(
            &writer,
            &format!("USER {} 0 * :{}", creds.username, creds.realname),
        )
        .await?;

        let nick = Self::wait_for_welcome(&writer, &mut reader, &creds.nick).await?;

        // Send QUIT
        let _ = Self::send_raw_with(&writer, "QUIT :probe").await;

        Ok(nick)
    }
}
