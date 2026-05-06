//! Signal channel via signal-cli daemon.
//!
//! - **Official tool**: <https://github.com/AsamK/signal-cli>
//! - **JSON-RPC spec**:
//!   <https://github.com/AsamK/signal-cli/blob/master/man/signal-cli-jsonrpc.5.adoc>
//! - **Protocol**: 子进程托管 signal-cli `--http=<addr>`，HTTP JSON-RPC
//!   `/api/v1/rpc` 双向 + SSE `/api/v1/events` 推送实时事件
//! - **Last reviewed**: 2026-05-05

pub mod client;
pub mod daemon;
pub mod format;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use client::SignalClient;
use daemon::SignalDaemon;

/// Running account state for a Signal connection.
struct RunningAccount {
    client: Arc<SignalClient>,
    #[allow(dead_code)]
    account_phone: String,
    daemon: Option<SignalDaemon>,
    /// Cache of `inbound_msg_id (timestamp 字串) → sender_id` for quoteAuthor
    /// 拼装。signal-cli send 必须同时提供 quoteTimestamp + quoteAuthor 才会
    /// 真正生效，缺一即被忽略。MsgContext.sender_id 在 dispatch 阶段不再可
    /// 见，改为入站时缓存（LRU 由 cap 自然驱逐，避免无限增长）。
    quote_authors: Arc<tokio::sync::Mutex<lru::LruCache<String, String>>>,
}

/// Signal channel plugin implementation.
///
/// Manages signal-cli daemon processes and communicates via JSON-RPC + SSE.
/// Credentials JSON: `{ "account": "+1234567890", "signalCliPath": null, "httpPort": null }`
pub struct SignalPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl SignalPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract the account phone number from credentials.
    fn extract_account(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("account")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.starts_with('+'))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing or invalid 'account' in Signal credentials (expected E.164 phone number like +1234567890)"
                )
            })
    }

    /// Extract optional signal-cli binary path from credentials.
    fn extract_cli_path(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("signalCliPath")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract optional HTTP port from credentials.
    fn extract_port(credentials: &serde_json::Value) -> Option<u16> {
        credentials
            .get("httpPort")
            .and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok())
    }

    /// Get the client for a running account.
    async fn get_client(&self, account_id: &str) -> Result<Arc<SignalClient>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.client.clone())
            .ok_or_else(|| anyhow::anyhow!("Signal account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for SignalPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Signal,
            display_name: "Signal".to_string(),
            description: "Signal via signal-cli".to_string(),
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
            supports_unsend: true,
            supports_reply: true,
            supports_threads: false,
            // TODO: native Signal media (signal-cli `--attachment`)
            // not yet implemented. Dispatcher falls back to a download-link
            // text for now.
            supports_media: Vec::new(),
            supports_typing: true,
            supports_buttons: false,
            max_message_length: None,
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let phone = Self::extract_account(&account.credentials)?;
        let cli_path = Self::extract_cli_path(&account.credentials);
        let port = Self::extract_port(&account.credentials);

        // Check that the signal-cli binary exists
        let binary_name = cli_path.as_deref().unwrap_or("signal-cli");
        if crate::channel::process_manager::find_binary(binary_name).is_none() {
            anyhow::bail!(
                "signal-cli binary not found: '{}'. Please install signal-cli or provide the full path in credentials.",
                binary_name
            );
        }

        // Start the daemon process
        let mut daemon = SignalDaemon::start(&phone, cli_path.as_deref(), port)?;
        let daemon_port = daemon.port();

        // Wait briefly for the daemon to initialize
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        if !daemon.is_running() {
            anyhow::bail!("signal-cli daemon exited immediately after start");
        }

        app_info!(
            "channel",
            "signal",
            "signal-cli daemon started for {} on port {}",
            phone,
            daemon_port
        );

        // Create the client
        let client = Arc::new(SignalClient::new(daemon_port, phone.clone()));

        let quote_authors = Arc::new(tokio::sync::Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(1024).expect("1024 is non-zero"),
        )));

        // Store the running account
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    client: client.clone(),
                    account_phone: phone.clone(),
                    daemon: Some(daemon),
                    quote_authors: quote_authors.clone(),
                },
            );
        }

        // Spawn the SSE event loop. Wrap inbound_tx so we cache sender_id of
        // each inbound MsgContext keyed by message_id, used later for
        // signal-cli's quoteAuthor field on outbound replies.
        let account_id = account.id.clone();
        let (intercept_tx, mut intercept_rx) = mpsc::channel::<MsgContext>(64);
        let cache = quote_authors.clone();
        let outbound_tx = inbound_tx.clone();
        tokio::spawn(async move {
            while let Some(ctx) = intercept_rx.recv().await {
                if !ctx.message_id.is_empty() && !ctx.sender_id.is_empty() {
                    let mut map = cache.lock().await;
                    map.put(ctx.message_id.clone(), ctx.sender_id.clone());
                }
                if outbound_tx.send(ctx).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            client.run_sse_loop(account_id, intercept_tx, cancel).await;
        });

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        if let Some(mut running) = accounts.remove(account_id) {
            if let Some(ref mut daemon) = running.daemon {
                daemon.stop().await;
            }
            app_info!(
                "channel",
                "signal",
                "Stopped Signal account '{}'",
                account_id
            );
        }
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let (client, quote_cache) = {
            let accounts = self.accounts.lock().await;
            let acc = accounts
                .get(account_id)
                .ok_or_else(|| anyhow::anyhow!("Signal account '{}' is not running", account_id))?;
            (acc.client.clone(), acc.quote_authors.clone())
        };

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let quote_ts = payload
                .reply_to_message_id
                .as_deref()
                .and_then(|id| id.parse::<i64>().ok());
            // signal-cli reply 必须 timestamp + author 配对，缺一不发 quote
            let quote_author = if let Some(reply_id) = payload.reply_to_message_id.as_deref() {
                let mut cache = quote_cache.lock().await;
                cache.get(reply_id).cloned()
            } else {
                None
            };

            match client
                .send_message(chat_id, text, &[], quote_ts, quote_author.as_deref())
                .await
            {
                Ok(result) => {
                    // signal-cli send returns the timestamp as message ID
                    let msg_id = result
                        .get("timestamp")
                        .and_then(|v| v.as_i64())
                        .map(|ts| ts.to_string())
                        .unwrap_or_else(|| "sent".to_string());
                    Ok(DeliveryResult::ok(msg_id))
                }
                Err(e) => Ok(DeliveryResult::err(e.to_string())),
            }
        } else {
            Ok(DeliveryResult::ok("no_content"))
        }
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        let client = self.get_client(account_id).await?;
        client.send_typing(chat_id).await
    }

    async fn delete_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let client = self.get_client(account_id).await?;
        let timestamp: i64 = message_id.parse().map_err(|_| {
            anyhow::anyhow!(
                "Invalid Signal message ID (expected timestamp): {}",
                message_id
            )
        })?;
        client.delete_message(chat_id, timestamp).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let phone = Self::extract_account(&account.credentials)?;
        let cli_path = Self::extract_cli_path(&account.credentials);
        let binary_name = cli_path.as_deref().unwrap_or("signal-cli");

        // Check if binary exists
        if crate::channel::process_manager::find_binary(binary_name).is_none() {
            return Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(format!("signal-cli binary not found: '{}'", binary_name)),
                uptime_secs: None,
                bot_name: None,
            });
        }

        // If the account is running, try to list identities
        let accounts = self.accounts.lock().await;
        if let Some(running) = accounts.get(&account.id) {
            match running.client.list_identities().await {
                Ok(_) => Ok(ChannelHealth {
                    is_running: true,
                    last_probe: Some(chrono::Utc::now().to_rfc3339()),
                    probe_ok: Some(true),
                    error: None,
                    uptime_secs: None,
                    bot_name: Some(phone),
                }),
                Err(e) => Ok(ChannelHealth {
                    is_running: true,
                    last_probe: Some(chrono::Utc::now().to_rfc3339()),
                    probe_ok: Some(false),
                    error: Some(e.to_string()),
                    uptime_secs: None,
                    bot_name: Some(phone),
                }),
            }
        } else {
            Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(phone),
            })
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        crate::channel::traits::default_check_access(account, msg, &[ChatType::Dm, ChatType::Group])
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_signal(markdown)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let phone = Self::extract_account(credentials)?;
        let cli_path = Self::extract_cli_path(credentials);
        let binary_name = cli_path.as_deref().unwrap_or("signal-cli");

        // Check that signal-cli binary exists
        if crate::channel::process_manager::find_binary(binary_name).is_none() {
            anyhow::bail!(
                "signal-cli binary not found: '{}'. Please install signal-cli or provide the full path.",
                binary_name
            );
        }

        Ok(phone)
    }
}
