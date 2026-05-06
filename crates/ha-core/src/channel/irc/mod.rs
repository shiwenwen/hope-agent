//! IRC channel (RFC 2812 + IRCv3).
//!
//! - **Official spec**: <https://www.rfc-editor.org/rfc/rfc2812>（base IRC）+
//!   <https://ircv3.net/specs/>（modern extensions: CAP / SASL / message-tags）
//! - **SDK / Reference**: 无统一 SDK；参考实现
//!   <https://ircv3.net/specs/extensions/sasl-3.1.html>
//! - **Protocol**: TCP / TLS 直连，`\r\n` 分隔，行 ≤ 512 字节（含 CRLF），
//!   PING/PONG 心跳，CAP LS 协商可选 SASL PLAIN
//! - **Last reviewed**: 2026-05-05

pub mod client;
pub mod format;
pub mod protocol;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::{chunk_text, ChannelPlugin};
use crate::channel::types::*;
use client::{IrcClient, IrcCredentials};

/// Running account state for an IRC connection.
struct RunningAccount {
    client: IrcClient,
    #[allow(dead_code)]
    bot_nick: String,
}

/// IRC channel plugin implementation.
pub struct IrcPlugin {
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl IrcPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract IRC credentials from the account config's credentials JSON.
    fn extract_credentials(credentials: &serde_json::Value) -> Result<IrcCredentials> {
        let server = credentials
            .get("server")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'server' in IRC credentials"))?;

        let port = credentials
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(6697) as u16;

        let tls = credentials
            .get("tls")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let nick = credentials
            .get("nick")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'nick' in IRC credentials"))?;

        let username = credentials
            .get("username")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| nick.clone());

        let realname = credentials
            .get("realname")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| nick.clone());

        let password = credentials
            .get("password")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let nickserv_password = credentials
            .get("nickservPassword")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let channels: Vec<String> = credentials
            .get("channels")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(IrcCredentials {
            server,
            port,
            tls,
            nick,
            username,
            realname,
            password,
            nickserv_password,
            channels,
        })
    }
}

#[async_trait]
impl ChannelPlugin for IrcPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Irc,
            display_name: "IRC".to_string(),
            description: "IRC (Internet Relay Chat)".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group],
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: false,
            supports_unsend: false,
            supports_reply: false,
            supports_threads: false,
            supports_media: Vec::new(),
            supports_typing: false,
            supports_buttons: false,
            max_message_length: Some(512),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let creds = Self::extract_credentials(&account.credentials)?;
        let account_id = account.id.clone();
        let nick = creds.nick.clone();

        let client =
            IrcClient::connect_and_run(creds, account_id.clone(), inbound_tx, cancel).await?;

        let confirmed_nick = client.nick().to_string();

        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account_id,
                RunningAccount {
                    client,
                    bot_nick: confirmed_nick.clone(),
                },
            );
        }

        app_info!(
            "channel",
            "irc",
            "Started IRC account '{}' as nick '{}'",
            account.label,
            confirmed_nick
        );

        // Suppress unused variable warning for the original nick
        let _ = nick;

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        if let Some(mut running) = accounts.remove(account_id) {
            running.client.close().await;
            app_info!("channel", "irc", "Stopped IRC account '{}'", account_id);
        }
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let accounts = self.accounts.lock().await;
        let running = accounts
            .get(account_id)
            .ok_or_else(|| anyhow::anyhow!("IRC account '{}' is not running", account_id))?;

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            running.client.send_privmsg(chat_id, text).await?;

            return Ok(DeliveryResult::ok(uuid::Uuid::new_v4().to_string()));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // IRC does not support typing indicators
        Ok(())
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let creds = Self::extract_credentials(&account.credentials)?;

        match IrcClient::probe(&creds).await {
            Ok(nick) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(nick),
            }),
            Err(e) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(e.to_string()),
                uptime_secs: None,
                bot_name: None,
            }),
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        crate::channel::traits::default_check_access(account, msg, &[ChatType::Dm, ChatType::Group])
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_irc(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        chunk_text(text, 512)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let creds = Self::extract_credentials(credentials)?;
        let nick = IrcClient::probe(&creds).await?;
        Ok(nick)
    }
}
