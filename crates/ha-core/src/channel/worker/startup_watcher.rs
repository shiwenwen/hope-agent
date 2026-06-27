//! On every fresh process boot, post a short "back online" notice
//! into every IM chat that was active within `window_secs`. Gives the
//! user a visible signal that a service restart / crash recovery /
//! upgrade just landed so they can re-send anything that was in flight.
//!
//! Trigger path:
//! 1. `app_init.rs::spawn_channel_listeners` calls
//!    `spawn_startup_notifier(registry)` once after all channel plugin
//!    workers have been brought up. This runs in all three runtime
//!    modes (desktop / server / acp).
//! 2. The watcher sleeps 3s so `start_watchdog::spawn_loop` finishes
//!    its first `start_account` round, then queries
//!    [`ChannelDB::list_recent_active_conversations`] for chats with
//!    user activity inside `AppConfig.startup_notification.window_secs`.
//! 3. Each chat is rate-limited by a per-chat `last_notified_at` stored
//!    in [`startup_state::StartupState`] (sentinel JSON at
//!    `~/.hope-agent/startup_state.json`). Cooldown defaults to 30 min.
//! 4. `HOPE_AGENT_CRASH_COUNT >= crash_loop_threshold` suppresses the
//!    notice entirely so a crashing process does not spam the user.
//! 5. Only the Primary process sends — the runtime-lock guard prevents
//!    a desktop+server pair on the same machine from double-delivering.
//!
//! Per-account silencing via `ChannelAccountConfig.notify_startup`
//! (default `true`); global kill switch via
//! `AppConfig.startup_notification.enabled`.
//!
//! Notice text follows `AppConfig.language` through `ha_core::i18n`.
//! IM servers do not carry per-recipient locale today, so this is a global
//! process preference rather than per-chat language negotiation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::task::JoinSet;

use super::startup_state::{conv_key, StartupState};
use crate::channel::config::ChannelStoreConfig;
use crate::channel::db::ChannelConversation;
use crate::channel::registry::ChannelRegistry;
use crate::channel::types::{ChannelId, ParseMode, ReplyPayload};

/// How long to wait after the channel registry is up before fanning out.
/// Lets `start_watchdog::spawn_loop` enter its first start_account pass.
const STARTUP_GRACE: Duration = Duration::from_secs(3);

/// Per-account ceiling on how long a send task will wait for the
/// account worker to flip to `is_running` before bailing. Generous
/// enough to cover an OAuth-y first handshake (Lark / Slack) but
/// bounded so a permanently-broken account does not stall fan-out.
/// Skipping does not write a cooldown entry — the next boot retries.
const ACCOUNT_READY_WAIT_SECS: u64 = 30;
const ACCOUNT_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Defensive hard cap on the candidate pool pulled from
/// `channel_conversations` before per-account / cooldown / silence
/// filtering. Decoupled from `cfg.global_max` so a noisy 72h window
/// (lots of cooldown-bound entries) cannot starve fresh chats: the
/// real send cap is applied later, after filtering.
const CANDIDATE_HARD_CAP: usize = 500;

/// Sentinel state retention. Drop entries older than ~7 days so the
/// JSON file does not grow unbounded over years of operation. Cooldown
/// only needs minutes; anything older is dead weight.
const STATE_RETENTION_SECS: u64 = 7 * 24 * 3600;

/// Outcome of `pick_targets`: which conversations to fan out to plus
/// classifier counts for the diagnostic summary line at the end.
pub(crate) struct PickOutcome {
    pub targets: Vec<ChannelConversation>,
    pub skipped_cooldown: usize,
    pub skipped_silenced: usize,
}

/// Decide which of `candidates` should actually receive a startup
/// notice. Filters (silenced / missing account / cooldown) **do not**
/// consume `global_max` budget — a chat that fails a filter just gets
/// skipped, leaving the budget for the next eligible conversation.
/// That's the entire point of the helper: with a flat `LIMIT global_max`
/// at the SQL layer the first 30 cooldown-bound rows would starve
/// every fresh chat behind them.
pub(crate) fn pick_targets(
    candidates: Vec<ChannelConversation>,
    accounts: &ChannelStoreConfig,
    state: &StartupState,
    cooldown_secs: u64,
    global_max: usize,
    now: DateTime<Utc>,
) -> PickOutcome {
    let mut out = PickOutcome {
        targets: Vec::new(),
        skipped_cooldown: 0,
        skipped_silenced: 0,
    };
    for conv in candidates {
        if out.targets.len() >= global_max {
            break;
        }
        let Some(account) = accounts.find_account(&conv.account_id) else {
            continue;
        };
        // Disabled accounts aren't auto-started, so readiness will never
        // flip to running — sending against them just burns the 30s
        // ACCOUNT_READY_WAIT_SECS and (worse) consumes a global_max slot
        // that an enabled account behind us in the candidate list would
        // have used. Treat them like notify_startup=false.
        if !account.enabled || !account.notify_startup {
            out.skipped_silenced += 1;
            continue;
        }
        let key = conv_key(
            &conv.channel_id,
            &conv.account_id,
            &conv.chat_id,
            conv.thread_id.as_deref(),
        );
        if state.cooldown_active(&key, now, cooldown_secs) {
            out.skipped_cooldown += 1;
            continue;
        }
        out.targets.push(conv);
    }
    out
}

/// Spawn the startup-notifier task. Idempotent at the contract level —
/// the inner task only runs once per process and is a no-op for any
/// caller that finds itself Secondary or with the global toggle off.
///
/// `registry` is cloned per `tokio::spawn` so the watcher does not
/// outlive the channel layer's normal Arc lifetime expectations.
pub fn spawn_startup_notifier(registry: Arc<ChannelRegistry>) {
    if !crate::runtime_lock::is_primary() {
        return;
    }
    let cfg = crate::config::cached_config().startup_notification.clone();
    if !cfg.enabled {
        app_info!(
            "channel",
            "startup_notifier",
            "Skipped: startup_notification.enabled=false"
        );
        return;
    }

    tokio::spawn(async move {
        tokio::time::sleep(STARTUP_GRACE).await;

        let crash_count: u32 = std::env::var("HOPE_AGENT_CRASH_COUNT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if crash_count >= cfg.crash_loop_threshold {
            app_warn!(
                "channel",
                "startup_notifier",
                "Suppressed: HOPE_AGENT_CRASH_COUNT={} >= threshold={}",
                crash_count,
                cfg.crash_loop_threshold
            );
            return;
        }

        let Some(channel_db) = crate::get_channel_db() else {
            app_warn!(
                "channel",
                "startup_notifier",
                "ChannelDB not initialised; skipping"
            );
            return;
        };

        let convs = match channel_db
            .list_recent_active_conversations(cfg.window_secs, CANDIDATE_HARD_CAP)
        {
            Ok(v) => v,
            Err(e) => {
                app_warn!(
                    "channel",
                    "startup_notifier",
                    "list_recent_active_conversations failed: {}",
                    e
                );
                return;
            }
        };

        if convs.is_empty() {
            app_info!(
                "channel",
                "startup_notifier",
                "No active conversations within {}s window — nothing to notify",
                cfg.window_secs
            );
            return;
        }

        let mut state = StartupState::load();
        let now = Utc::now();
        let candidate_count = convs.len();

        // Snapshot the config once — `cached_config()` is a lock-free
        // ArcSwap load, but hoisting it makes the loop intent obvious.
        let store = crate::config::cached_config();
        let startup_text = crate::i18n::localized_backend_message(
            crate::i18n::BackendMessage::StartupBackOnline,
            crate::i18n::effective_ui_locale(&store),
        );
        let PickOutcome {
            targets,
            skipped_cooldown,
            skipped_silenced,
        } = pick_targets(
            convs,
            &store.channels,
            &state,
            cfg.cooldown_secs,
            cfg.global_max,
            now,
        );

        let mut tasks: JoinSet<(
            String,
            String,
            String,
            String,
            anyhow::Result<crate::channel::types::DeliveryResult>,
        )> = JoinSet::new();
        for conv in targets {
            let channel_id = match ChannelId::from_storage_str(&conv.channel_id) {
                Ok(c) => c,
                Err(e) => {
                    app_warn!(
                        "channel",
                        "startup_notifier",
                        "Unknown channel_id {} on startup notice: {}",
                        conv.channel_id,
                        e
                    );
                    continue;
                }
            };
            let Some(plugin) = registry.get_plugin(&channel_id).cloned() else {
                continue;
            };

            let key = conv_key(
                &conv.channel_id,
                &conv.account_id,
                &conv.chat_id,
                conv.thread_id.as_deref(),
            );
            let reply = ReplyPayload {
                text: Some(plugin.markdown_to_native(startup_text)),
                thread_id: conv.thread_id.clone(),
                parse_mode: Some(ParseMode::Html),
                ..ReplyPayload::text("")
            };

            // Fan out the network I/O — each send is an HTTP round-trip
            // to a different vendor (Telegram / Lark / Slack / …), so
            // serializing them would mean global_max × per-call latency
            // (10s+ for a 30-chat fan-out). JoinSet lets us await all
            // and still record `mark_notified` per success.
            let channel_id_str = conv.channel_id.clone();
            let account_id = conv.account_id.clone();
            let chat_id = conv.chat_id.clone();
            let registry_for_task = registry.clone();
            tasks.spawn(async move {
                // Per-account readiness gate. The first `start_account`
                // pass runs in another tokio task spawned by
                // `start_background_tasks`, and OAuth-y handshakes can
                // take well beyond `STARTUP_GRACE`. Poll until the
                // account flips to running or the deadline elapses;
                // skip on timeout without writing a cooldown entry so
                // the next boot retries (subject to watchdog recovery).
                let deadline = Instant::now() + Duration::from_secs(ACCOUNT_READY_WAIT_SECS);
                while !registry_for_task.health(&account_id).await.is_running {
                    if Instant::now() >= deadline {
                        return (
                            key,
                            channel_id_str,
                            account_id,
                            chat_id,
                            Err(anyhow::anyhow!(
                                "account not ready within {}s",
                                ACCOUNT_READY_WAIT_SECS
                            )),
                        );
                    }
                    tokio::time::sleep(ACCOUNT_READY_POLL_INTERVAL).await;
                }
                let result = plugin.send_message(&account_id, &chat_id, &reply).await;
                (key, channel_id_str, account_id, chat_id, result)
            });
        }

        let mut sent = 0usize;
        while let Some(joined) = tasks.join_next().await {
            let Ok((key, channel_id_str, account_id, chat_id, result)) = joined else {
                continue;
            };
            match result {
                Ok(delivery) if delivery.success => {
                    state.mark_notified(key, now);
                    sent += 1;
                }
                Ok(delivery) => app_warn!(
                    "channel",
                    "startup_notifier",
                    "send_message returned failure for {}/{}/{}: {}",
                    channel_id_str,
                    account_id,
                    chat_id,
                    delivery.error.as_deref().unwrap_or("(no error message)")
                ),
                Err(e) => app_warn!(
                    "channel",
                    "startup_notifier",
                    "send_message failed for {}/{}/{}: {}",
                    channel_id_str,
                    account_id,
                    chat_id,
                    e
                ),
            }
        }

        state.prune(now, STATE_RETENTION_SECS);
        if let Err(e) = state.save() {
            app_warn!(
                "channel",
                "startup_notifier",
                "startup_state.json save failed: {}",
                e
            );
        }

        app_info!(
            "channel",
            "startup_notifier",
            "completed: sent={} cooldown_skipped={} silenced={} candidates={} crash_count={}",
            sent,
            skipped_cooldown,
            skipped_silenced,
            candidate_count,
            crash_count
        );
    });
}

#[cfg(test)]
mod pick_targets_tests {
    use super::*;
    use crate::channel::config::ChannelStoreConfig;
    use crate::channel::types::{ChannelAccountConfig, ChannelId, SecurityConfig};
    use chrono::Duration as ChronoDuration;

    fn account(id: &str, notify_startup: bool) -> ChannelAccountConfig {
        account_with_enabled(id, notify_startup, true)
    }

    fn account_with_enabled(id: &str, notify_startup: bool, enabled: bool) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: id.to_string(),
            channel_id: ChannelId::Telegram,
            label: id.to_string(),
            enabled,
            agent_id: None,
            credentials: serde_json::Value::Null,
            settings: serde_json::Value::Null,
            security: SecurityConfig::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
            notify_startup,
        }
    }

    fn conv(account_id: &str, chat_id: &str) -> ChannelConversation {
        ChannelConversation {
            id: 0,
            channel_id: "telegram".to_string(),
            account_id: account_id.to_string(),
            chat_id: chat_id.to_string(),
            thread_id: None,
            session_id: format!("sess-{chat_id}"),
            sender_id: None,
            sender_name: None,
            chat_type: "dm".to_string(),
            source: "inbound".to_string(),
            attached_at: None,
            created_at: "2026-05-10T00:00:00Z".to_string(),
            updated_at: "2026-05-10T00:00:00Z".to_string(),
        }
    }

    fn store(accounts: Vec<ChannelAccountConfig>) -> ChannelStoreConfig {
        ChannelStoreConfig {
            accounts,
            ..ChannelStoreConfig::default()
        }
    }

    #[test]
    fn cooldown_skips_do_not_consume_global_max_budget() {
        // 5 candidates, first 3 are cooldown-bound, global_max=2.
        // Expectation: the two fresh chats both get picked — cooldown
        // skips must not eat the budget.
        let accounts = store(vec![account("acc", true)]);
        let mut state = StartupState::default();
        let now = Utc::now();
        for chat in ["cold-1", "cold-2", "cold-3"] {
            state.mark_notified(
                conv_key("telegram", "acc", chat, None),
                now - ChronoDuration::seconds(60),
            );
        }
        let candidates = vec![
            conv("acc", "cold-1"),
            conv("acc", "cold-2"),
            conv("acc", "cold-3"),
            conv("acc", "fresh-1"),
            conv("acc", "fresh-2"),
        ];

        let out = pick_targets(candidates, &accounts, &state, 1800, 2, now);

        let picked: Vec<_> = out.targets.iter().map(|c| c.chat_id.as_str()).collect();
        assert_eq!(picked, vec!["fresh-1", "fresh-2"]);
        assert_eq!(out.skipped_cooldown, 3);
        assert_eq!(out.skipped_silenced, 0);
    }

    #[test]
    fn silenced_account_does_not_consume_global_max_budget() {
        // Account "muted" silences its 3 chats; account "live" has 2;
        // global_max=2 → both live chats should fit.
        let accounts = store(vec![account("muted", false), account("live", true)]);
        let candidates = vec![
            conv("muted", "m-1"),
            conv("muted", "m-2"),
            conv("muted", "m-3"),
            conv("live", "l-1"),
            conv("live", "l-2"),
        ];
        let out = pick_targets(
            candidates,
            &accounts,
            &StartupState::default(),
            1800,
            2,
            Utc::now(),
        );
        let picked: Vec<_> = out.targets.iter().map(|c| c.chat_id.as_str()).collect();
        assert_eq!(picked, vec!["l-1", "l-2"]);
        assert_eq!(out.skipped_silenced, 3);
    }

    #[test]
    fn global_max_caps_real_targets_only() {
        // 4 fresh chats, global_max=2 → first 2 picked, last 2 dropped.
        let accounts = store(vec![account("acc", true)]);
        let candidates = (0..4).map(|i| conv("acc", &format!("c-{i}"))).collect();
        let out = pick_targets(
            candidates,
            &accounts,
            &StartupState::default(),
            1800,
            2,
            Utc::now(),
        );
        assert_eq!(out.targets.len(), 2);
        assert_eq!(out.skipped_cooldown, 0);
        assert_eq!(out.skipped_silenced, 0);
    }

    #[test]
    fn disabled_account_does_not_consume_global_max_budget() {
        // Disabled account won't be auto-started — its chats would burn
        // the 30s ACCOUNT_READY_WAIT_SECS and then fail, but worse: they
        // would consume budget that enabled chats behind them need.
        let accounts = store(vec![
            account_with_enabled("off", true, false),
            account("on", true),
        ]);
        let candidates = vec![
            conv("off", "o-1"),
            conv("off", "o-2"),
            conv("on", "n-1"),
            conv("on", "n-2"),
        ];
        let out = pick_targets(
            candidates,
            &accounts,
            &StartupState::default(),
            1800,
            2,
            Utc::now(),
        );
        let picked: Vec<_> = out.targets.iter().map(|c| c.chat_id.as_str()).collect();
        assert_eq!(picked, vec!["n-1", "n-2"]);
        assert_eq!(out.skipped_silenced, 2);
    }

    #[test]
    fn missing_account_is_silently_dropped() {
        // Conversation references account that no longer exists — just
        // skip, do not crash, do not count against budget.
        let accounts = store(vec![account("acc", true)]);
        let candidates = vec![conv("ghost", "g-1"), conv("acc", "a-1")];
        let out = pick_targets(
            candidates,
            &accounts,
            &StartupState::default(),
            1800,
            5,
            Utc::now(),
        );
        let picked: Vec<_> = out.targets.iter().map(|c| c.chat_id.as_str()).collect();
        assert_eq!(picked, vec!["a-1"]);
    }
}
