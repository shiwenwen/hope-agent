//! WeChat (微信) iLink Bot channel.
//!
//! - **Official API**: iLink Bot HTTP JSON API（私有协议，非微信官方公开）
//! - **SDK / Reference**: Tencent OpenClaw plugin (TS) —
//!   <https://github.com/Tencent/openclaw-weixin>
//!   该仓库是当前 hope-agent 实现的权威对照来源（auth header / UIN / version
//!   编码 / endpoint 路径全部按 OpenClaw 实装）
//! - **Protocol**: HTTPS long-polling `ilink/bot/getupdates` + AES-128-ECB
//!   媒体加密 + QR 码登录
//! - **Last reviewed**: 2026-05-05

pub mod api;
pub mod login;
pub(crate) mod media;
pub mod polling;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use self::api::WeChatApi;

struct RunningAccount {
    api: Arc<WeChatApi>,
}

const TYPING_TICKET_TTL: Duration = Duration::from_secs(24 * 60 * 60); // 24 hours
const TYPING_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const SESSION_PAUSE_DURATION: Duration = Duration::from_secs(3600); // 1 hour

#[derive(Default)]
pub(crate) struct WeChatSharedState {
    accounts: Mutex<HashMap<String, RunningAccount>>,
    context_tokens: Mutex<HashMap<String, HashMap<String, String>>>,
    /// Cache: account_id -> (user_id -> (typing_ticket, cached_at))
    typing_tickets: Mutex<HashMap<String, HashMap<String, (String, Instant)>>>,
    /// Active typing keepalive tasks: (account_id, user_id) -> CancellationToken
    typing_keepalives: Mutex<HashMap<(String, String), CancellationToken>>,
    /// When a session expires (errcode -14), API calls pause until this time
    paused_until: Mutex<HashMap<String, Instant>>,
}

impl WeChatSharedState {
    fn account_state_dir() -> Result<PathBuf> {
        let dir = crate::paths::channel_dir("wechat")?;
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn sync_buf_path(account_id: &str) -> Result<PathBuf> {
        Ok(Self::account_state_dir()?.join(format!("{}.sync.json", account_id)))
    }

    fn context_token_path(account_id: &str) -> Result<PathBuf> {
        Ok(Self::account_state_dir()?.join(format!("{}.context_tokens.json", account_id)))
    }

    pub fn load_sync_buf(&self, account_id: &str) -> Option<String> {
        let path = Self::sync_buf_path(account_id).ok()?;
        let raw = fs::read_to_string(path).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        parsed
            .get("getUpdatesBuf")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    }

    pub fn save_sync_buf(&self, account_id: &str, get_updates_buf: &str) -> Result<()> {
        let path = Self::sync_buf_path(account_id)?;
        fs::write(
            path,
            serde_json::to_string(&serde_json::json!({
                "getUpdatesBuf": get_updates_buf,
            }))?,
        )?;
        Ok(())
    }

    pub async fn restore_context_tokens(&self, account_id: &str) -> Result<()> {
        let path = Self::context_token_path(account_id)?;
        if !path.exists() {
            return Ok(());
        }

        let raw = fs::read_to_string(&path)?;
        let tokens: HashMap<String, String> =
            serde_json::from_str(&raw).context("Failed to parse WeChat context token store")?;

        self.context_tokens
            .lock()
            .await
            .insert(account_id.to_string(), tokens);
        Ok(())
    }

    pub async fn set_context_token(
        &self,
        account_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<()> {
        let snapshot = {
            let mut store = self.context_tokens.lock().await;
            let entry = store.entry(account_id.to_string()).or_default();
            entry.insert(user_id.to_string(), token.to_string());
            entry.clone()
        };
        self.persist_context_tokens(account_id, &snapshot)
    }

    pub async fn get_context_token(&self, account_id: &str, user_id: &str) -> Option<String> {
        let store = self.context_tokens.lock().await;
        store
            .get(account_id)
            .and_then(|value| value.get(user_id))
            .cloned()
    }

    fn persist_context_tokens(
        &self,
        account_id: &str,
        tokens: &HashMap<String, String>,
    ) -> Result<()> {
        let path = Self::context_token_path(account_id)?;
        fs::write(path, serde_json::to_string(tokens)?)?;
        Ok(())
    }

    pub async fn get_or_fetch_typing_ticket(
        &self,
        account_id: &str,
        user_id: &str,
        api: &WeChatApi,
    ) -> Result<String> {
        // Check cache — respect TTL
        {
            let cache = self.typing_tickets.lock().await;
            if let Some((ticket, cached_at)) = cache.get(account_id).and_then(|m| m.get(user_id)) {
                if cached_at.elapsed() < TYPING_TICKET_TTL {
                    return Ok(ticket.clone());
                }
            }
        }

        // Fetch from API with exponential backoff (2s, 4s, 8s)
        let context_token = self.get_context_token(account_id, user_id).await;
        let mut last_err = None;
        for attempt in 0..3u32 {
            match api.get_config(user_id, context_token.as_deref()).await {
                Ok(config) => {
                    if let Some(ticket) = config.typing_ticket.filter(|v| !v.trim().is_empty()) {
                        let mut cache = self.typing_tickets.lock().await;
                        cache
                            .entry(account_id.to_string())
                            .or_default()
                            .insert(user_id.to_string(), (ticket.clone(), Instant::now()));
                        return Ok(ticket);
                    }
                    return Err(anyhow::anyhow!(
                        "No typing_ticket returned for user '{}'",
                        user_id
                    ));
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(2u64 << attempt)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("getConfig failed")))
    }

    pub async fn invalidate_typing_ticket(&self, account_id: &str, user_id: &str) {
        let mut cache = self.typing_tickets.lock().await;
        if let Some(map) = cache.get_mut(account_id) {
            map.remove(user_id);
        }
    }

    pub async fn start_typing_keepalive(
        &self,
        account_id: &str,
        user_id: &str,
        api: Arc<WeChatApi>,
        ticket: String,
    ) {
        self.stop_typing_keepalive(account_id, user_id).await;

        let cancel = CancellationToken::new();
        {
            let mut keepalives = self.typing_keepalives.lock().await;
            keepalives.insert(
                (account_id.to_string(), user_id.to_string()),
                cancel.clone(),
            );
        }

        let user_id = user_id.to_string();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(TYPING_KEEPALIVE_INTERVAL) => {
                        if let Err(err) = api.send_typing(&user_id, &ticket, api::TYPING_STATUS_TYPING).await {
                            app_debug!("channel", "wechat", "Typing keepalive failed for '{}': {}", user_id, err);
                            break;
                        }
                    }
                }
            }
        });
    }

    pub async fn stop_typing_keepalive(&self, account_id: &str, user_id: &str) {
        let mut keepalives = self.typing_keepalives.lock().await;
        if let Some(cancel) = keepalives.remove(&(account_id.to_string(), user_id.to_string())) {
            cancel.cancel();
        }
    }

    pub async fn pause_account(&self, account_id: &str) {
        let mut paused = self.paused_until.lock().await;
        paused.insert(
            account_id.to_string(),
            Instant::now() + SESSION_PAUSE_DURATION,
        );
        // Clear typing tickets for paused account
        let mut tickets = self.typing_tickets.lock().await;
        tickets.remove(account_id);
    }

    pub async fn is_paused(&self, account_id: &str) -> bool {
        let paused = self.paused_until.lock().await;
        paused
            .get(account_id)
            .map(|until| Instant::now() < *until)
            .unwrap_or(false)
    }

    pub async fn clear_pause(&self, account_id: &str) {
        let mut paused = self.paused_until.lock().await;
        paused.remove(account_id);
    }

    pub fn clear_persisted_account_state(account_id: &str) -> Result<()> {
        for path in [
            Self::sync_buf_path(account_id)?,
            Self::context_token_path(account_id)?,
        ] {
            if path.exists() {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }
}

pub fn clear_persisted_account_state(account_id: &str) -> Result<()> {
    WeChatSharedState::clear_persisted_account_state(account_id)
}

pub struct WeChatPlugin {
    shared: Arc<WeChatSharedState>,
}

impl WeChatPlugin {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(WeChatSharedState::default()),
        }
    }

    fn extract_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("token")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("Missing 'token' in WeChat credentials"))
    }

    fn extract_base_url(settings: &serde_json::Value, credentials: &serde_json::Value) -> String {
        settings
            .get("baseUrl")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                credentials
                    .get("baseUrl")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or(api::DEFAULT_WECHAT_BASE_URL)
            .trim()
            .to_string()
    }

    fn display_name(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("userId")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                credentials
                    .get("remoteAccountId")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
            })
    }

    async fn get_api(&self, account_id: &str) -> Result<Arc<WeChatApi>> {
        let accounts = self.shared.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|account| account.api.clone())
            .ok_or_else(|| anyhow::anyhow!("WeChat account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for WeChatPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::WeChat,
            display_name: "WeChat".to_string(),
            description: "WeChat direct messages via the iLink HTTP API".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm],
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: false,
            supports_unsend: false,
            supports_reply: false,
            supports_threads: false,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Document,
                MediaType::Voice,
            ],
            supports_typing: true,
            supports_buttons: false,
            max_message_length: Some(4000),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: tokio::sync::mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let token = Self::extract_token(&account.credentials)?;
        let base_url = Self::extract_base_url(&account.settings, &account.credentials);
        let api = Arc::new(WeChatApi::new(Some(token), base_url.clone()));

        api.probe().await?;
        self.shared.restore_context_tokens(&account.id).await?;

        {
            let mut accounts = self.shared.accounts.lock().await;
            accounts.insert(account.id.clone(), RunningAccount { api: api.clone() });
        }

        app_info!(
            "channel",
            "wechat",
            "WeChat account '{}' authenticated against {}",
            account.label,
            base_url
        );

        tokio::spawn(polling::run_polling_loop(
            api,
            self.shared.clone(),
            account.id.clone(),
            inbound_tx,
            cancel,
        ));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        self.shared.accounts.lock().await.remove(account_id);
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        if self.shared.is_paused(account_id).await {
            return Err(anyhow::anyhow!(
                "WeChat account '{}' is paused due to session expiry",
                account_id
            ));
        }

        // Cancel typing keepalive and send typing cancel (status=2)
        self.shared.stop_typing_keepalive(account_id, chat_id).await;
        if let Ok(api) = self.get_api(account_id).await {
            if let Ok(ticket) = self
                .shared
                .get_or_fetch_typing_ticket(account_id, chat_id, &api)
                .await
            {
                let _ = api
                    .send_typing(chat_id, &ticket, api::TYPING_STATUS_CANCEL)
                    .await;
            }
        }

        let api = self.get_api(account_id).await?;
        let context_token = self.shared.get_context_token(account_id, chat_id).await;

        // Send media attachments first (each as a separate message)
        let mut last_media_id = None;
        for m in &payload.media {
            let mid = media::send_outbound_media(
                &api,
                m,
                chat_id,
                None, // caption is handled inside send_outbound_media
                context_token.as_deref(),
                None,
            )
            .await?;
            last_media_id = Some(mid);
        }

        // Send text (if any)
        let text = payload.text.as_deref().map(str::trim).unwrap_or("");
        if !text.is_empty() {
            let message_id = api
                .send_text(chat_id, text, context_token.as_deref())
                .await?;
            return Ok(DeliveryResult::ok(message_id));
        }

        // If only media was sent, return the last media message id
        if let Some(mid) = last_media_id {
            return Ok(DeliveryResult::ok(mid));
        }

        Ok(DeliveryResult::ok("empty"))
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        if self.shared.is_paused(account_id).await {
            return Ok(());
        }

        let api = self.get_api(account_id).await?;
        let ticket = match self
            .shared
            .get_or_fetch_typing_ticket(account_id, chat_id, &api)
            .await
        {
            Ok(t) => t,
            Err(err) => {
                app_debug!(
                    "channel",
                    "wechat",
                    "Failed to get typing ticket for '{}': {}",
                    chat_id,
                    err
                );
                return Ok(());
            }
        };

        match api
            .send_typing(chat_id, &ticket, api::TYPING_STATUS_TYPING)
            .await
        {
            Ok(()) => {
                // Start keepalive: re-send typing every 5 seconds until cancelled
                self.shared
                    .start_typing_keepalive(account_id, chat_id, api, ticket)
                    .await;
                Ok(())
            }
            Err(err) => {
                // Ticket may have expired; invalidate cache so next call re-fetches
                self.shared
                    .invalidate_typing_ticket(account_id, chat_id)
                    .await;
                app_debug!(
                    "channel",
                    "wechat",
                    "send_typing failed for '{}': {}",
                    chat_id,
                    err
                );
                Ok(())
            }
        }
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let token = Self::extract_token(&account.credentials)?;
        let base_url = Self::extract_base_url(&account.settings, &account.credentials);
        let api = WeChatApi::new(Some(token), base_url);

        match api.probe().await {
            Ok(()) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Self::display_name(&account.credentials),
            }),
            Err(err) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(err.to_string()),
                uptime_secs: None,
                bot_name: Self::display_name(&account.credentials),
            }),
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        if msg.chat_type != ChatType::Dm {
            return false;
        }

        match account.security.dm_policy {
            DmPolicy::Open => true,
            DmPolicy::Allowlist | DmPolicy::Pairing => {
                account.security.user_allowlist.contains(&msg.sender_id)
                    || account.security.admin_ids.contains(&msg.sender_id)
            }
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        markdown_to_wechat_text(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        crate::channel::traits::chunk_text(text, 4000)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let token = Self::extract_token(credentials)?;
        let base_url = credentials
            .get("baseUrl")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(api::DEFAULT_WECHAT_BASE_URL);
        let api = WeChatApi::new(Some(token), base_url.to_string());
        api.probe().await?;
        Ok(Self::display_name(credentials).unwrap_or_else(|| "WeChat".to_string()))
    }
}

fn markdown_to_wechat_text(markdown: &str) -> String {
    let mut text = markdown.replace("\r\n", "\n");
    text = regex_replace(code_block_regex(), &text, "$1");
    text = regex_replace(image_regex(), &text, "");
    text = regex_replace(link_regex(), &text, "$1");
    text = regex_replace(heading_regex(), &text, "$1");
    text = regex_replace(quote_regex(), &text, "$1");
    text = text
        .replace("**", "")
        .replace("__", "")
        .replace("~~", "")
        .replace('`', "");
    text = regex_replace(blank_line_regex(), &text, "\n\n");
    text.trim().to_string()
}

fn regex_replace(regex: &Regex, input: &str, replacement: &str) -> String {
    regex.replace_all(input, replacement).to_string()
}

fn code_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"```[^\n]*\n?([\s\S]*?)```").expect("valid regex"))
}

fn image_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("valid regex"))
}

fn link_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").expect("valid regex"))
}

fn heading_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?m)^#{1,6}\s+(.*)$").expect("valid regex"))
}

fn quote_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?m)^>\s?(.*)$").expect("valid regex"))
}

fn blank_line_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\n{3,}").expect("valid regex"))
}
