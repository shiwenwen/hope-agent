//! Direct WeCom AI-bot text channel. Credentials and callback anchors never
//! leave this adapter; ordinary turns still enter the shared channel worker.
mod connection;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use ha_core::channel::traits::{default_check_access, ChannelPlugin};
use ha_core::channel::types::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

use connection::{Actor, ConnectError, Credentials, Timing};

pub(super) const MAX_TEXT: usize = 20_480;
type Rejections = Arc<Mutex<HashMap<String, [u8; 32]>>>;

#[derive(Default)]
pub struct WeComPlugin {
    accounts: Mutex<HashMap<String, Running>>,
    rejected: Rejections,
    lifecycle: AsyncMutex<()>,
}

struct Running {
    bot_id: String,
    fingerprint: [u8; 32],
    commands: mpsc::Sender<Outbound>,
    health: Arc<Mutex<ChannelHealth>>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

pub(super) struct Outbound {
    chat: String,
    message: String,
    text: String,
    result: oneshot::Sender<std::result::Result<RichReplyReceipt, ReplyStreamError>>,
}

pub(super) fn error(kind: ReplyStreamErrorKind) -> ReplyStreamError {
    let message = match kind {
        ReplyStreamErrorKind::InvalidContent => "WeCom supports text only, up to 20480 UTF-8 bytes",
        ReplyStreamErrorKind::InvalidTarget | ReplyStreamErrorKind::Expired =>
            "WeCom requires this account's exact, unexpired message callback; proactive sends are unavailable",
        ReplyStreamErrorKind::Ambiguous =>
            "WeCom reply acknowledgement is unknown; do not resend automatically",
        ReplyStreamErrorKind::Rejected => "WeCom rejected this reply",
        _ => "WeCom is not ready to accept a reply",
    };
    ReplyStreamError::new(kind, message)
}

pub(super) fn allowed(account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
    if msg.account_id != account.id || msg.channel_id != ChannelId::WeCom {
        return false;
    }
    match msg.chat_type {
        ChatType::Dm => {
            if msg.chat_id != format!("single:{}", msg.sender_id)
                || account.security.dm_policy == DmPolicy::Pairing
            {
                return false;
            }
        }
        ChatType::Group => {
            if !msg.chat_id.starts_with("group:") {
                return false;
            }
            let security = &account.security;
            let group = security
                .groups
                .get(&msg.chat_id)
                .or_else(|| security.groups.get("*"));
            // The SDK supplies no reliable mention flag. Explicit mention-only
            // policy remains closed, including the inherited default.
            if group.and_then(|g| g.require_mention) != Some(false) {
                return false;
            }
            if group.and_then(|g| g.group_policy.as_ref()) == Some(&GroupPolicy::Disabled) {
                return false;
            }
            if security.group_policy == GroupPolicy::Allowlist
                && group.is_none()
                && !security.group_allowlist.contains(&msg.chat_id)
            {
                return false;
            }
        }
        _ => return false,
    }
    default_check_access(account, msg, &[ChatType::Dm, ChatType::Group])
}

impl WeComPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    async fn reply(
        &self,
        target: &ReplyStreamTarget,
        reply: &RichReply,
    ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
        if reply.markdown.trim().is_empty()
            || reply.markdown.len() > MAX_TEXT
            || !reply.media.is_empty()
            || !reply.buttons.is_empty()
        {
            return Err(error(ReplyStreamErrorKind::InvalidContent));
        }
        if !matches!(target.chat_type, ChatType::Dm | ChatType::Group)
            || target.thread_id.is_some()
            || (target.chat_type == ChatType::Dm && !target.chat_id.starts_with("single:"))
            || (target.chat_type == ChatType::Group && !target.chat_id.starts_with("group:"))
        {
            return Err(error(ReplyStreamErrorKind::InvalidTarget));
        }
        let message = target
            .reply_to_message_id
            .as_deref()
            .filter(|value| connection::valid_id(value))
            .ok_or_else(|| error(ReplyStreamErrorKind::InvalidTarget))?;
        let commands = {
            let accounts = self.accounts.lock().expect("WeCom accounts");
            let runtime = accounts
                .get(&target.account_id)
                .ok_or_else(|| error(ReplyStreamErrorKind::InvalidTarget))?;
            if runtime.health.lock().expect("WeCom health").probe_ok != Some(true) {
                return Err(error(ReplyStreamErrorKind::Transient));
            }
            runtime.commands.clone()
        };
        let (result, receipt) = oneshot::channel();
        commands
            .try_send(Outbound {
                chat: target.chat_id.clone(),
                message: message.into(),
                text: reply.markdown.clone(),
                result,
            })
            .map_err(|_| error(ReplyStreamErrorKind::Transient))?;
        // Once enqueued, a dropped caller cannot establish non-delivery. The
        // actor checks a closed result channel before starting any mutation.
        match tokio::time::timeout(Duration::from_secs(10), receipt).await {
            Ok(Ok(result)) => result,
            _ => Err(error(ReplyStreamErrorKind::Ambiguous)),
        }
    }

    async fn validate_with<C, F>(&self, credentials: &Credentials, connect: C) -> Result<String>
    where
        C: FnOnce() -> F,
        F: std::future::Future<Output = std::result::Result<connection::Socket, ConnectError>>,
    {
        let _guard = self.lifecycle.lock().await;
        let fingerprint = credentials.fingerprint();
        let cached = {
            let accounts = self.accounts.lock().expect("WeCom accounts");
            let mut found = false;
            let mut healthy = false;
            for runtime in accounts
                .values()
                .filter(|runtime| runtime.bot_id == credentials.bot_id())
            {
                // WeCom replaces a bot's previous client when another socket
                // subscribes. A draft Secret must not disconnect a running bot.
                if runtime.fingerprint != fingerprint {
                    return Err(anyhow!("Stop this WeCom bot's running account before validating changed credentials"));
                }
                found = true;
                healthy |= runtime.health.lock().expect("WeCom health").probe_ok == Some(true);
            }
            found.then_some(healthy)
        };
        if let Some(ready) = cached {
            return if ready {
                Ok("WeCom".into())
            } else {
                Err(anyhow!("WeCom account is running but disconnected; inspect its status or stop/restart it before validating"))
            };
        }
        let mut socket = connect().await.map_err(|_| {
            anyhow!("WeCom credentials or authentication acknowledgement rejected/unavailable")
        })?;
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
        self.rejected
            .lock()
            .expect("WeCom rejections")
            .retain(|_, rejected| *rejected != fingerprint);
        Ok("WeCom".into())
    }
}

#[async_trait]
impl ChannelPlugin for WeComPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::WeCom,
            display_name: "WeCom".into(),
            description: "Direct AI-bot DM/group text".into(),
            version: env!("CARGO_PKG_VERSION").into(),
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
            supports_media: vec![],
            supports_typing: false,
            supports_buttons: false,
            streaming_preview_max_bytes: None,
            supports_card_stream: false,
            native_reply: Some(NativeReplyCapabilities {
                preview_chat_types: vec![],
                final_chat_types: vec![ChatType::Dm, ChatType::Group],
                update_mode: ReplyStreamUpdateMode::Snapshot,
                preview_persistence: ReplyStreamPreviewPersistence::Persistent,
                requires_reply_anchor: true,
                supports_thread_anchor: false,
                requires_recipient_user_id: false,
                requires_recipient_tenant_id: false,
                supports_task_updates: false,
                supports_plan_updates: false,
                supports_blocks: false,
                embedded_media_types: vec![],
                max_embedded_media_items: None,
                refresh_after_secs: None,
                max_delta_chars: None,
            }),
        }
    }
    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound: mpsc::Sender<InboundEvent>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let _guard = self.lifecycle.lock().await;
        let credentials = Credentials::parse(&account.credentials)?;
        let fingerprint = credentials.fingerprint();
        let bot_id = credentials.bot_id().to_owned();
        if self
            .accounts
            .lock()
            .expect("WeCom accounts")
            .contains_key(&account.id)
        {
            return Err(anyhow!("WeCom account is already running"));
        }
        if self
            .rejected
            .lock()
            .expect("WeCom rejections")
            .get(&account.id)
            == Some(&fingerprint)
        {
            return Err(anyhow!(
                "WeCom authentication rejected; validate corrected credentials before retrying"
            ));
        }
        let socket = match connection::connect(&credentials, &cancel).await {
            Ok(socket) => socket,
            Err(ConnectError::Auth) => {
                self.rejected
                    .lock()
                    .expect("WeCom rejections")
                    .insert(account.id.clone(), fingerprint);
                return Err(anyhow!(
                    "WeCom authentication rejected; validate corrected credentials before retrying"
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "WeCom connection or authentication acknowledgement unavailable"
                ))
            }
        };
        let (commands, receiver) = mpsc::channel(16);
        let cancel = cancel.child_token();
        let health = Arc::new(Mutex::new(ChannelHealth::default()));
        let actor = Actor::new(
            account.clone(),
            inbound,
            receiver,
            health.clone(),
            cancel.clone(),
            self.rejected.clone(),
            fingerprint,
        );
        let reconnect_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            actor
                .run(
                    socket,
                    || connection::connect(&credentials, &reconnect_cancel),
                    Timing::default(),
                )
                .await;
        });
        self.accounts.lock().expect("WeCom accounts").insert(
            account.id.clone(),
            Running {
                bot_id,
                fingerprint,
                commands,
                health,
                cancel,
                task,
            },
        );
        Ok(())
    }
    async fn stop_account(&self, id: &str) -> Result<()> {
        let _guard = self.lifecycle.lock().await;
        let runtime = self.accounts.lock().expect("WeCom accounts").remove(id);
        if let Some(mut runtime) = runtime {
            runtime.cancel.cancel();
            if tokio::time::timeout(Duration::from_secs(3), &mut runtime.task)
                .await
                .is_err()
            {
                runtime.task.abort();
                let _ = runtime.task.await;
            }
        }
        Ok(())
    }
    async fn send_rich_reply(
        &self,
        target: &ReplyStreamTarget,
        reply: &RichReply,
    ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
        self.reply(target, reply).await
    }
    async fn send_message(
        &self,
        account: &str,
        chat: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let chat_type = if chat.starts_with("single:") {
            ChatType::Dm
        } else {
            ChatType::Group
        };
        let target = ReplyStreamTarget {
            account_id: account.into(),
            chat_id: chat.into(),
            chat_type,
            thread_id: payload.thread_id.clone(),
            reply_to_message_id: payload.reply_to_message_id.clone(),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let receipt = self
            .reply(
                &target,
                &RichReply {
                    markdown: payload.text.clone().unwrap_or_default(),
                    media: payload.media.clone(),
                    buttons: payload.buttons.clone(),
                },
            )
            .await?;
        Ok(DeliveryResult::ok(receipt.message_id))
    }
    async fn send_typing(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn chunk_message(&self, text: &str) -> Vec<String> {
        vec![text.into()]
    }
    fn markdown_to_native(&self, text: &str) -> String {
        text.into()
    }
    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        allowed(account, msg)
    }
    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        Ok(self
            .probe_running_cached(account)
            .await?
            .unwrap_or_else(|| ChannelHealth {
                probe_ok: Some(false),
                error: Some("WeCom account is stopped".into()),
                ..Default::default()
            }))
    }
    async fn probe_running_cached(
        &self,
        account: &ChannelAccountConfig,
    ) -> Result<Option<ChannelHealth>> {
        Ok(self
            .accounts
            .lock()
            .expect("WeCom accounts")
            .get(&account.id)
            .map(|runtime| runtime.health.lock().expect("WeCom health").clone()))
    }
    async fn validate_credentials(&self, value: &Value) -> Result<String> {
        let credentials = Credentials::parse(value)?;
        let cancel = CancellationToken::new();
        self.validate_with(&credentials, || connection::connect(&credentials, &cancel))
            .await
    }
}
