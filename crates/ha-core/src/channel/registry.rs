use anyhow::Result;
use futures_util::{stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use super::traits::ChannelPlugin;
use super::types::*;

fn should_probe_in_health_list(channel_id: &ChannelId, is_running: bool) -> bool {
    matches!(channel_id, ChannelId::Signal | ChannelId::WhatsApp)
        || (is_running && matches!(channel_id, ChannelId::IMessage))
}

fn should_probe_account(
    channel_id: &ChannelId,
    is_running: bool,
    include_stopped_imessage: bool,
) -> bool {
    if matches!(channel_id, ChannelId::IMessage) && !is_running && !include_stopped_imessage {
        return false;
    }
    matches!(
        channel_id,
        ChannelId::Signal | ChannelId::WhatsApp | ChannelId::IMessage
    ) || !is_running
}

pub(crate) const DELIVERY_SURFACE_STATE_CHANGED_EVENT: &str =
    "channel:delivery_surface_state_changed";

/// Notify the Primary delivery coordinator that resolving this account's IM
/// surface may now produce a different result. Successful starts can turn
/// `Unavailable` into `Attached`; a persisted disable/removal can turn it into
/// `Absent`. Consumers re-resolve from live state rather than trusting an event
/// subtype, so duplicate notifications are harmless.
pub(crate) fn emit_delivery_surface_state_changed(account_id: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            DELIVERY_SURFACE_STATE_CHANGED_EVENT,
            serde_json::json!({ "accountId": account_id }),
        );
    }
}

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
    inbound_tx: mpsc::Sender<InboundEvent>,
}

impl ChannelRegistry {
    /// Create a new registry. Returns the registry and the inbound event receiver.
    ///
    /// Call `register_plugin()` on the returned registry before wrapping in `Arc`.
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<InboundEvent>) {
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
        crate::channel_hooks::start_watchdog_mark_success(&account.id).await;

        app_info!(
            "channel",
            "registry",
            "Started account '{}' on channel {}",
            account.label,
            account.channel_id
        );
        // A queued ParentInjection may have inspected this exact binding while
        // the account was unavailable. EventBus keeps ChannelRegistry
        // independent of the injection implementation; the Primary listener
        // re-resolves this account's queued delivery surfaces.
        emit_delivery_surface_state_changed(&account.id);
        Ok(())
    }

    /// Stop a running channel account. Also cancels any queued
    /// watchdog retry — user intent always overrides the watchdog.
    pub async fn stop_account(&self, account_id: &str) -> Result<()> {
        crate::channel_hooks::start_watchdog_cancel_pending(account_id).await;

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

    /// Merge worker liveness with bounded adapter-owned runtime discovery.
    /// Signal and WhatsApp need bounded external discovery. A running
    /// iMessage account serves its already negotiated in-process status;
    /// other network adapters stay out of the Settings poll.
    pub async fn health_with_probe(&self, account_id: &str) -> ChannelHealth {
        self.health_with_probe_mode(account_id, true).await
    }

    async fn health_with_probe_mode(
        &self,
        account_id: &str,
        include_stopped_imessage: bool,
    ) -> ChannelHealth {
        let mut health = self.health(account_id).await;
        let account = crate::config::cached_config()
            .channels
            .find_account(account_id)
            .cloned();
        let Some(account) = account else {
            return health;
        };
        if !should_probe_account(
            &account.channel_id,
            health.is_running,
            include_stopped_imessage,
        ) {
            return health;
        }
        let Some(plugin) = self.get_plugin(&account.channel_id) else {
            return health;
        };
        let cached_imessage = health.is_running && account.channel_id == ChannelId::IMessage;
        let probe_result = tokio::time::timeout(std::time::Duration::from_secs(4), async {
            if cached_imessage {
                plugin.probe_running_cached(&account).await
            } else {
                plugin.probe(&account).await.map(Some)
            }
        })
        .await;
        if let Ok(Ok(Some(probe))) = probe_result {
            if cached_imessage {
                let current = self.health(account_id).await;
                if !current.is_running {
                    return current;
                }
            }
            health.probe_ok = probe.probe_ok;
            health.bot_name = probe.bot_name;
            health.error = probe.error;
            health.last_probe = probe.last_probe;
            health.capability_snapshot = probe.capability_snapshot;
        } else if cached_imessage {
            // Stop may have removed the cached client after our worker read.
            return self.health(account_id).await;
        }
        health
    }

    pub async fn list_health_with_probes(&self) -> Vec<(String, ChannelHealth)> {
        let accounts = crate::config::cached_config()
            .channels
            .accounts
            .iter()
            .enumerate()
            .map(|(index, account)| (index, account.id.clone(), account.channel_id.clone()))
            .collect::<Vec<_>>();
        // The Settings UI polls this aggregate endpoint every 10 seconds.
        // Sidecars need bounded discovery; iMessage is polled only while its
        // worker runs, when probe reads cached status without a new process.
        // Stopped iMessage accounts retain worker-only aggregate health.
        let mut health = stream::iter(accounts.into_iter().map(
            |(index, account_id, channel_id)| async move {
                let worker = self.health(&account_id).await;
                let snapshot = if should_probe_in_health_list(&channel_id, worker.is_running) {
                    self.health_with_probe_mode(&account_id, false).await
                } else {
                    worker
                };
                (index, account_id, snapshot)
            },
        ))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        health.sort_by_key(|(index, _, _)| *index);
        health
            .into_iter()
            .map(|(_, account_id, snapshot)| (account_id, snapshot))
            .collect()
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

#[cfg(test)]
mod health_list_tests {
    use super::*;

    #[test]
    fn running_imessage_uses_cached_probe_but_stopped_account_is_not_polled() {
        assert!(should_probe_in_health_list(&ChannelId::IMessage, true));
        assert!(!should_probe_in_health_list(&ChannelId::IMessage, false));
        assert!(should_probe_in_health_list(&ChannelId::Signal, false));
        assert!(!should_probe_in_health_list(&ChannelId::Telegram, true));
        assert!(!should_probe_account(&ChannelId::IMessage, false, false));
        assert!(should_probe_account(&ChannelId::IMessage, false, true));
    }
}
