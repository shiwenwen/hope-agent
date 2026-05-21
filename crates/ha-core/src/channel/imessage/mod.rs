//! iMessage channel (macOS only, via `imsg` CLI).
//!
//! - **Official tool**: <https://github.com/steipete/imsg>
//!   (要求 macOS Full Disk Access + Automation→Messages 权限)
//! - **SDK / Reference**: imsg JSON-RPC over stdio 文档见仓库 README
//! - **Protocol**: 子进程托管 `imsg`，stdio NDJSON JSON-RPC，watch 订阅推送
//!   事件，send 单条命令；macOS 限定（依赖 Messages.app + chat.db）
//! - **Last reviewed**: 2026-05-20

pub mod client;
pub mod format;
pub mod media;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

/// Running account state for an iMessage account.
struct RunningAccount {
    client: client::IMessageClient,
}

/// iMessage channel plugin implementation.
///
/// Communicates with the local `imsg` CLI tool via JSON-RPC over stdio.
/// **macOS only** -- on other platforms all operations return errors.
pub struct IMessagePlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl IMessagePlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract the imsg binary path from credentials JSON.
    /// Defaults to "imsg" if not specified.
    #[cfg(target_os = "macos")]
    fn extract_imsg_path(credentials: &serde_json::Value) -> String {
        credentials
            .get("imsgPath")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "imsg".to_string())
    }

    /// Extract the optional database path from credentials JSON.
    #[cfg(target_os = "macos")]
    fn extract_db_path(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("dbPath")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

#[async_trait]
impl ChannelPlugin for IMessagePlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::IMessage,
            display_name: "iMessage".to_string(),
            description: "iMessage (macOS only)".to_string(),
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
            supports_reply: true,
            supports_threads: false,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
                MediaType::Sticker,
                MediaType::Voice,
                MediaType::Animation,
            ],
            supports_typing: false,
            supports_buttons: false,
            streaming_preview_max_bytes: None,
            supports_card_stream: false,
        }
    }

    #[cfg(target_os = "macos")]
    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let imsg_path = Self::extract_imsg_path(&account.credentials);
        let db_path = Self::extract_db_path(&account.credentials);

        // Verify the binary exists
        if crate::channel::process_manager::find_binary(&imsg_path).is_none() {
            return Err(anyhow::anyhow!(
                "imsg binary not found at '{}'. Please install imsg or set the correct path.",
                imsg_path
            ));
        }

        app_info!(
            "channel",
            "imessage",
            "Starting iMessage account '{}' with binary '{}'",
            account.id,
            imsg_path
        );

        // Start the RPC client
        let imsg_client = client::IMessageClient::start(&imsg_path, db_path.as_deref())?;

        // 顺序至关重要：先启动 stdout 读取 loop（spawn 内 ready_tx 就绪），
        // 再调 watch_subscribe。否则 watch_subscribe 的 RPC response 在 read
        // loop 启动前就到了，pending oneshot 没人接 → 10s timeout 失败 →
        // notification 订阅失败 → inbound 消息全丢。
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        imsg_client
            .run_notification_loop(account.id.clone(), inbound_tx, cancel, ready_tx)
            .await;

        // 等 read loop spawn 内部 ready 信号；最长 5s 兜底防止 spawn 失败时无限挂
        if tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
            .await
            .is_err()
        {
            app_warn!(
                "channel",
                "imessage",
                "Notification loop ready signal timed out (5s); subscribe may race"
            );
        }

        // Subscribe to watch notifications
        if let Err(e) = imsg_client.watch_subscribe().await {
            app_warn!(
                "channel",
                "imessage",
                "Failed to subscribe to watch notifications: {}",
                e
            );
        }

        // Store the running account
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    client: imsg_client,
                },
            );
        }

        app_info!(
            "channel",
            "imessage",
            "iMessage account '{}' started successfully",
            account.id
        );

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    async fn start_account(
        &self,
        _account: &ChannelAccountConfig,
        _inbound_tx: mpsc::Sender<InboundEvent>,
        _cancel: CancellationToken,
    ) -> Result<()> {
        Err(anyhow::anyhow!("iMessage is only supported on macOS"))
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        if let Some(running) = accounts.remove(account_id) {
            app_info!(
                "channel",
                "imessage",
                "Stopping iMessage account '{}'",
                account_id
            );
            running.client.stop().await;
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "iMessage account '{}' is not running",
                account_id
            ))
        }
    }

    #[cfg(target_os = "macos")]
    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let accounts = self.accounts.lock().await;
        let running = accounts
            .get(account_id)
            .ok_or_else(|| anyhow::anyhow!("iMessage account '{}' is not running", account_id))?;

        if !payload.media.is_empty() {
            let prepared = match media::prepare_imessage_attachments(&payload.media).await {
                Ok(prepared) => prepared,
                Err(e) => return Ok(DeliveryResult::err(e.to_string())),
            };
            let send_result: Result<String> = async {
                let mut last_msg_id = None;
                for (idx, attachment) in prepared.attachments().iter().enumerate() {
                    let text = media::attachment_text(
                        payload.text.as_deref(),
                        attachment.caption.as_deref(),
                        idx == 0,
                    );
                    let result = running
                        .client
                        .send_file(
                            chat_id,
                            text.as_deref(),
                            &attachment.path,
                            payload.reply_to_message_id.as_deref(),
                        )
                        .await?;
                    last_msg_id = Some(extract_message_id(&result).unwrap_or_else(|| {
                        if prepared.attachments().len() == 1 {
                            "sent".to_string()
                        } else {
                            format!("sent:{}", idx + 1)
                        }
                    }));
                }
                Ok(last_msg_id.unwrap_or_else(|| "no_content".to_string()))
            }
            .await;
            prepared.cleanup().await;

            return match send_result {
                Ok(msg_id) => Ok(DeliveryResult::ok(msg_id)),
                Err(e) => Ok(DeliveryResult::err(e.to_string())),
            };
        }

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let reply_to = payload.reply_to_message_id.as_deref();
            match running.client.send_message(chat_id, text, reply_to).await {
                Ok(result) => {
                    // Try to extract message ID from result
                    let msg_id = extract_message_id(&result).unwrap_or_else(|| "ok".to_string());
                    Ok(DeliveryResult::ok(msg_id))
                }
                Err(e) => Ok(DeliveryResult::err(e.to_string())),
            }
        } else {
            Ok(DeliveryResult::ok("no_content"))
        }
    }

    #[cfg(not(target_os = "macos"))]
    async fn send_message(
        &self,
        _account_id: &str,
        _chat_id: &str,
        _payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        Err(anyhow::anyhow!("iMessage is only supported on macOS"))
    }

    #[cfg(target_os = "macos")]
    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        // `imsg rpc` does not expose typing indicators; typing is an advanced
        // IMCore/bridge feature, so this channel does not advertise support.
        let _ = (account_id, chat_id);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        Err(anyhow::anyhow!("iMessage is only supported on macOS"))
    }

    #[cfg(target_os = "macos")]
    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let imsg_path = Self::extract_imsg_path(&account.credentials);
        let db_path = Self::extract_db_path(&account.credentials);

        // Check binary exists
        if crate::channel::process_manager::find_binary(&imsg_path).is_none() {
            return Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(format!("imsg binary not found at '{}'", imsg_path)),
                uptime_secs: None,
                bot_name: None,
            });
        }

        // Try to start a temporary client and list conversations
        match client::IMessageClient::start(&imsg_path, db_path.as_deref()) {
            Ok(temp_client) => {
                let result = temp_client.list_conversations().await;
                temp_client.stop().await;

                match result {
                    Ok(_) => Ok(ChannelHealth {
                        is_running: false,
                        last_probe: Some(chrono::Utc::now().to_rfc3339()),
                        probe_ok: Some(true),
                        error: None,
                        uptime_secs: None,
                        bot_name: Some("iMessage".to_string()),
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

    #[cfg(not(target_os = "macos"))]
    async fn probe(&self, _account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        Ok(ChannelHealth {
            is_running: false,
            last_probe: Some(chrono::Utc::now().to_rfc3339()),
            probe_ok: Some(false),
            error: Some("iMessage is only supported on macOS".to_string()),
            uptime_secs: None,
            bot_name: None,
        })
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        crate::channel::traits::default_check_access(account, msg, &[ChatType::Dm, ChatType::Group])
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_imessage(markdown)
    }

    #[cfg(target_os = "macos")]
    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let imsg_path = Self::extract_imsg_path(credentials);
        let db_path = Self::extract_db_path(credentials);

        // Check binary exists
        if crate::channel::process_manager::find_binary(&imsg_path).is_none() {
            return Err(anyhow::anyhow!(
                "imsg binary not found at '{}'. Install via: brew install steipete/tap/imsg",
                imsg_path
            ));
        }

        // Try to probe by listing conversations
        let temp_client = client::IMessageClient::start(&imsg_path, db_path.as_deref())?;
        let result = temp_client.list_conversations().await;
        temp_client.stop().await;

        match result {
            Ok(_) => Ok("iMessage".to_string()),
            Err(e) => Err(anyhow::anyhow!("Failed to connect to iMessage: {}", e)),
        }
    }

    #[cfg(not(target_os = "macos"))]
    async fn validate_credentials(&self, _credentials: &serde_json::Value) -> Result<String> {
        Err(anyhow::anyhow!("iMessage is only supported on macOS"))
    }
}

#[cfg(target_os = "macos")]
fn extract_message_id(result: &serde_json::Value) -> Option<String> {
    result
        .get("messageId")
        .or_else(|| result.get("message_id"))
        .or_else(|| result.get("id"))
        .or_else(|| result.get("guid"))
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
}
