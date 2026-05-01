use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use super::traits::ChannelPlugin;
use super::types::*;

/// Handle to a running channel account worker.
pub struct ChannelWorkerHandle {
    pub account_id: String,
    pub channel_id: ChannelId,
    cancel: CancellationToken,
    started_at: chrono::DateTime<chrono::Utc>,
}

impl ChannelWorkerHandle {
    /// Elapsed uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        (chrono::Utc::now() - self.started_at).num_seconds().max(0) as u64
    }
}

/// Central registry for all channel plugins and running account workers.
pub struct ChannelRegistry {
    plugins: HashMap<ChannelId, Arc<dyn ChannelPlugin>>,
    workers: Mutex<HashMap<String, ChannelWorkerHandle>>,
    inbound_tx: mpsc::Sender<MsgContext>,
}

impl ChannelRegistry {
    /// Create a new registry. Returns the registry and the inbound message receiver.
    ///
    /// Call `register_plugin()` on the returned registry before wrapping in `Arc`.
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<MsgContext>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        let registry = Self {
            plugins: HashMap::new(),
            workers: Mutex::new(HashMap::new()),
            inbound_tx: tx,
        };
        (registry, rx)
    }

    /// Register a channel plugin. Must be called during initialization
    /// before the registry is wrapped in `Arc`.
    pub fn register_plugin(&mut self, plugin: Arc<dyn ChannelPlugin>) {
        let meta = plugin.meta();
        app_info!(
            "channel",
            "registry",
            "Registered channel plugin: {} ({})",
            meta.display_name,
            meta.id
        );
        self.plugins.insert(meta.id, plugin);
    }

    /// Get a plugin by channel ID.
    pub fn get_plugin(&self, channel_id: &ChannelId) -> Option<&Arc<dyn ChannelPlugin>> {
        self.plugins.get(channel_id)
    }

    /// List all registered plugins' metadata.
    pub fn list_plugins(&self) -> Vec<(ChannelMeta, ChannelCapabilities)> {
        self.plugins
            .values()
            .map(|p| (p.meta(), p.capabilities()))
            .collect()
    }

    /// Start a channel account. Spawns the plugin's background worker.
    pub async fn start_account(&self, account: &ChannelAccountConfig) -> Result<()> {
        let plugin = self.plugins.get(&account.channel_id).ok_or_else(|| {
            anyhow::anyhow!("No plugin registered for channel: {}", account.channel_id)
        })?;

        // Check if already running
        {
            let workers = self.workers.lock().await;
            if workers.contains_key(&account.id) {
                return Err(anyhow::anyhow!(
                    "Account '{}' is already running",
                    account.id
                ));
            }
        }

        let cancel = CancellationToken::new();

        // Start the plugin's account listener
        plugin
            .start_account(account, self.inbound_tx.clone(), cancel.clone())
            .await?;

        // Record the worker handle
        let handle = ChannelWorkerHandle {
            account_id: account.id.clone(),
            channel_id: account.channel_id.clone(),
            cancel,
            started_at: chrono::Utc::now(),
        };

        {
            let mut workers = self.workers.lock().await;
            workers.insert(account.id.clone(), handle);
        }
        // Clear any queued retry so a manual Start / UI Restart doesn't
        // race with the watchdog firing a redundant attempt.
        super::start_watchdog::mark_success(&account.id).await;

        app_info!(
            "channel",
            "registry",
            "Started account '{}' on channel {}",
            account.label,
            account.channel_id
        );
        Ok(())
    }

    /// Stop a running channel account. Also cancels any queued
    /// watchdog retry — user intent always overrides the watchdog.
    pub async fn stop_account(&self, account_id: &str) -> Result<()> {
        super::start_watchdog::cancel_pending(account_id).await;

        let handle = {
            let mut workers = self.workers.lock().await;
            workers.remove(account_id)
        };

        if let Some(handle) = handle {
            handle.cancel.cancel();
            // Also notify the plugin to clean up
            if let Some(plugin) = self.plugins.get(&handle.channel_id) {
                let _ = plugin.stop_account(account_id).await;
            }
            app_info!("channel", "registry", "Stopped account '{}'", account_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Account '{}' is not running", account_id))
        }
    }

    /// Restart a channel account (stop then start).
    pub async fn restart_account(&self, account: &ChannelAccountConfig) -> Result<()> {
        let _ = self.stop_account(&account.id).await; // ignore error if not running
        self.start_account(account).await
    }

    /// Send a reply message through a channel.
    pub async fn send_reply(
        &self,
        account: &ChannelAccountConfig,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let plugin = self
            .plugins
            .get(&account.channel_id)
            .ok_or_else(|| anyhow::anyhow!("No plugin for channel: {}", account.channel_id))?;
        plugin.send_message(&account.id, chat_id, payload).await
    }

    /// Get health status for a running account.
    pub async fn health(&self, account_id: &str) -> ChannelHealth {
        let workers = self.workers.lock().await;
        if let Some(handle) = workers.get(account_id) {
            ChannelHealth {
                is_running: true,
                uptime_secs: Some(handle.uptime_secs()),
                ..Default::default()
            }
        } else {
            ChannelHealth::default()
        }
    }

    /// List all running accounts with their health.
    pub async fn list_running(&self) -> Vec<(String, ChannelHealth)> {
        let workers = self.workers.lock().await;
        workers
            .iter()
            .map(|(id, handle)| {
                (
                    id.clone(),
                    ChannelHealth {
                        is_running: true,
                        uptime_secs: Some(handle.uptime_secs()),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// Re-sync slash command menus for a single running account. Returns 1 on
    /// success, 0 if the account isn't running, the config row is missing, or
    /// the plugin call failed (warn-logged). Re-sync is best-effort —
    /// `Err` is reserved for "no plugin registered for this channel id"
    /// invariant violations the caller should propagate.
    pub async fn sync_commands_for_account(&self, account_id: &str) -> Result<usize> {
        let channel_id = {
            let workers = self.workers.lock().await;
            match workers.get(account_id) {
                Some(h) => h.channel_id.clone(),
                None => return Ok(0),
            }
        };

        let account_cfg = {
            let cfg = crate::config::cached_config();
            cfg.channels.find_account(account_id).cloned()
        };
        let Some(account_cfg) = account_cfg else {
            app_warn!(
                "channel",
                "registry",
                "sync_commands: account '{}' is running but missing from config",
                account_id
            );
            return Ok(0);
        };

        let plugin = self
            .plugins
            .get(&channel_id)
            .ok_or_else(|| anyhow::anyhow!("No plugin registered for channel: {}", channel_id))?
            .clone();

        match plugin.sync_commands(&account_cfg).await {
            Ok(()) => Ok(1),
            Err(e) => {
                app_warn!(
                    "channel",
                    "registry",
                    "sync_commands failed for account '{}': {}",
                    account_id,
                    e
                );
                Ok(0)
            }
        }
    }

    /// Re-sync slash command menus for every running account. Each account is
    /// attempted independently so a stale Telegram connection doesn't block
    /// Discord from picking up the change. Sequential because a typical user
    /// only has 1-3 IM accounts and matches the `stop_all` shape.
    pub async fn sync_commands_for_all(&self) -> usize {
        let account_ids: Vec<String> = {
            let workers = self.workers.lock().await;
            workers.keys().cloned().collect()
        };

        let mut synced = 0usize;
        for account_id in account_ids {
            match self.sync_commands_for_account(&account_id).await {
                Ok(n) => synced += n,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "registry",
                        "sync_commands_for_account('{}') errored: {}",
                        account_id,
                        e
                    );
                }
            }
        }
        synced
    }

    /// Unified entry-point that callers (Tauri / HTTP / event listener) can use
    /// without branching themselves: `Some(id)` → sync that single account,
    /// `None` → sync every running account.
    pub async fn sync_commands(&self, account_id: Option<&str>) -> Result<usize> {
        match account_id {
            Some(id) => self.sync_commands_for_account(id).await,
            None => Ok(self.sync_commands_for_all().await),
        }
    }

    /// Stop all running accounts. Called during app shutdown.
    pub async fn stop_all(&self) {
        let account_ids: Vec<String> = {
            let workers = self.workers.lock().await;
            workers.keys().cloned().collect()
        };

        for account_id in account_ids {
            if let Err(e) = self.stop_account(&account_id).await {
                app_warn!(
                    "channel",
                    "registry",
                    "Failed to stop account '{}': {}",
                    account_id,
                    e
                );
            }
        }
    }
}
