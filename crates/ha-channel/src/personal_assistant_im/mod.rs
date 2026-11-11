//! Dedicated Telegram entry for the private assistant. This module never
//! creates a normal channel account or accesses an IM conversation attach.

mod credentials;
mod runtime;
mod telegram;

pub use runtime::spawn;

use std::sync::Arc;

use anyhow::Result;
use ha_core::personal_assistant_im::{
    AssistantImError, BotPurpose, ConnectionView, DeliveryReceipt, NotificationPolicy, PairingCode,
    SessionGrant,
};
use ha_core::session::SessionDB;
use serde::Serialize;

use telegram::TelegramClient;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantImStatus {
    pub connection: Option<ConnectionView>,
    pub grants: Vec<SessionGrant>,
    pub deliveries: Vec<DeliveryReceipt>,
    pub notification_policy: NotificationPolicy,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Serializes configuration publication and rollback across desktop/server
/// processes. Platform identity itself is still enforced by the SQLite claim.
pub(crate) async fn configuration_guard() -> Result<std::fs::File> {
    let guard = ha_core::blocking::run_blocking(|| -> Result<std::fs::File> {
        let directory = ha_core::paths::credentials_dir()?;
        std::fs::create_dir_all(&directory)?;
        ha_core::platform::try_acquire_exclusive_lock(&directory.join("telegram-purpose.lock"))?
            .ok_or_else(|| AssistantImError::Unavailable.into())
    })
    .await?;
    // Lock order is purpose -> config. A cross-process cached snapshot can be
    // older than a successfully published account, so it must never authorize
    // deletion of an apparent orphan reservation.
    let config =
        ha_core::blocking::run_blocking(ha_core::config::reload_config_snapshot_from_disk).await?;
    let accounts: Vec<_> = config
        .channels
        .accounts
        .iter()
        .filter(|account| account.channel_id == ha_core::channel::types::ChannelId::Telegram)
        .cloned()
        .collect();
    let ids = accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    let db = ha_core::get_session_db().ok_or(AssistantImError::Unavailable)?;
    let pending = db
        .run(move |db| db.reconcile_telegram_bot_reservations(&ids))
        .await?;
    let pending_ids = pending
        .into_iter()
        .map(|(_, id)| id)
        .collect::<std::collections::HashSet<_>>();
    for account in accounts
        .into_iter()
        .filter(|account| pending_ids.contains(&account.id))
    {
        // An existing account may still name the previous token if a process
        // died before publication. Resolve today's credentials, not the staged
        // bot id, before committing or discarding any remaining reservation.
        let Ok(bot_id) = ordinary_bot_identity(&account.credentials, &account.settings).await
        else {
            // Keep unverified claims closed, but allow the owner to replace a
            // revoked token or remove the account. Assistant configuration does
            // its own mandatory verification of every remaining ordinary bot.
            continue;
        };
        db.run(move |db| db.commit_ordinary_telegram_bot(bot_id, &account.id))
            .await?;
    }
    Ok(guard)
}

async fn ordinary_bot_identity(
    credentials: &serde_json::Value,
    settings: &serde_json::Value,
) -> Result<i64> {
    let token = credentials
        .get("token")
        .and_then(|token| token.as_str())
        .ok_or(AssistantImError::InvalidInput)?;
    let proxy = settings
        .get("proxy")
        .and_then(|proxy| proxy.as_str())
        .map(str::trim)
        .filter(|proxy| !proxy.is_empty())
        .map(str::to_owned)
        .or_else(ha_core::provider::active_custom_proxy_url);
    let client = TelegramClient::connect_with_proxy(token, proxy.as_deref()).await?;
    Ok(client.bot_id)
}

pub(crate) async fn reserve_ordinary_bot(
    account_id: &str,
    credentials: &serde_json::Value,
    settings: &serde_json::Value,
) -> Result<i64> {
    let bot_id = ordinary_bot_identity(credentials, settings).await?;
    let id = account_id.to_owned();
    let db = ha_core::get_session_db().ok_or(AssistantImError::Unavailable)?;
    db.run(move |db| db.reserve_telegram_bot(bot_id, &id))
        .await?;
    Ok(bot_id)
}

pub(crate) async fn finish_ordinary_bot_reservation(
    account_id: &str,
    bot_id: i64,
    published: bool,
) -> Result<()> {
    let db = ha_core::get_session_db().ok_or(AssistantImError::Unavailable)?;
    let account_id = account_id.to_owned();
    db.run(move |db| {
        if published {
            db.commit_ordinary_telegram_bot(bot_id, &account_id)
        } else {
            db.discard_telegram_bot_reservation(bot_id, &account_id)
        }
    })
    .await
}

pub(crate) async fn claim_ordinary_bot_start(
    account: &ha_core::channel::types::ChannelAccountConfig,
    observed_bot_id: i64,
) -> Result<()> {
    let canonical = if account
        .settings
        .get("apiRoot")
        .and_then(|value| value.as_str())
        .is_some_and(|root| !root.trim().is_empty())
    {
        reserve_ordinary_bot(&account.id, &account.credentials, &account.settings).await?
    } else {
        observed_bot_id
    };
    if canonical != observed_bot_id {
        return Err(AssistantImError::IdentityInUse.into());
    }
    // Start may use a worker snapshot; it may verify its own claim but cannot
    // release another identity's binding as a config-publication operation can.
    let db = ha_core::get_session_db().ok_or(AssistantImError::Unavailable)?;
    let id = account.id.clone();
    db.run(move |db| db.claim_telegram_bot(canonical, BotPurpose::Ordinary, &id))
        .await
}

pub async fn status(db: Arc<SessionDB>) -> Result<AssistantImStatus> {
    db.run(|db| {
        let (notification_policy, deliveries) = db.assistant_im_notification_status()?;
        Ok(AssistantImStatus {
            connection: db.assistant_im_connection()?,
            grants: db.assistant_im_grants(now())?,
            deliveries,
            notification_policy,
        })
    })
    .await
}

/// All previously configured ordinary bots must have verified identity claims
/// before the assistant may occupy one. An unavailable old identity keeps the
/// new connection closed rather than assuming its credentials name another bot.
async fn register_ordinary_identities(db: &Arc<SessionDB>) -> Result<()> {
    let accounts = ha_core::config::cached_config().channels.accounts.clone();
    for account in accounts {
        if account.channel_id != ha_core::channel::types::ChannelId::Telegram {
            continue;
        }
        let token = account
            .credentials
            .get("token")
            .and_then(|token| token.as_str())
            .ok_or(AssistantImError::Unavailable)?;
        let proxy = account
            .settings
            .get("proxy")
            .and_then(|proxy| proxy.as_str())
            .map(str::trim)
            .filter(|proxy| !proxy.is_empty())
            .map(str::to_owned)
            .or_else(ha_core::provider::active_custom_proxy_url);
        let client = TelegramClient::connect_with_proxy(token, proxy.as_deref()).await?;
        db.run(move |db| db.claim_telegram_bot(client.bot_id, BotPurpose::Ordinary, &account.id))
            .await?;
    }
    Ok(())
}

/// Owner-only. The token is deliberately absent from status/config exports.
pub async fn configure(db: Arc<SessionDB>, token: String) -> Result<ConnectionView> {
    let _guard = configuration_guard().await?;
    register_ordinary_identities(&db).await?;
    let client = TelegramClient::connect(token.trim()).await?;
    let previous = db.run(|db| db.assistant_im_runtime_snapshot()).await?;
    let credential_ref =
        ha_core::blocking::run_blocking(move || credentials::publish(token.trim())).await?;
    let save_ref = credential_ref.clone();
    let result = db
        .run(move |db| {
            db.save_assistant_im_connection(client.bot_id, &client.username, &save_ref, now())
        })
        .await;
    match result {
        Ok(view) => {
            if let Some(previous) = previous {
                if ha_core::blocking::run_blocking(move || {
                    credentials::remove(&previous.credential_ref)
                })
                .await
                .is_err()
                {
                    app_warn!(
                        "channel",
                        "personal_assistant_im",
                        "Retired assistant IM credential cleanup remains pending"
                    );
                }
            }
            Ok(view)
        }
        Err(error) => {
            let _ =
                ha_core::blocking::run_blocking(move || credentials::remove(&credential_ref)).await;
            Err(error)
        }
    }
}

pub async fn begin_pairing(db: Arc<SessionDB>) -> Result<PairingCode> {
    db.run(|db| db.begin_assistant_im_pairing(now())).await
}

pub async fn confirm_pairing(
    db: Arc<SessionDB>,
    generation: i64,
    user_id: i64,
    chat_id: i64,
) -> Result<()> {
    // Both Primary and Secondary owner surfaces persist the same intent. Only
    // the Primary transport may claim its one-shot activation delivery.
    db.run(move |db| {
        db.confirm_assistant_im_pairing(generation, user_id, chat_id, now())
            .map(|_| ())
    })
    .await
}

pub async fn set_grant(db: Arc<SessionDB>, input: SessionGrant) -> Result<SessionGrant> {
    db.run(move |db| db.set_assistant_im_grant(input, now()))
        .await
}

pub async fn revoke_grant(db: Arc<SessionDB>, session_id: String) -> Result<()> {
    db.run(move |db| db.revoke_assistant_im_grant(&session_id))
        .await
}

pub async fn revoke_pairing(db: Arc<SessionDB>) -> Result<()> {
    db.run(|db| db.revoke_assistant_im_pairing()).await
}

pub async fn remove_connection(db: Arc<SessionDB>) -> Result<()> {
    let _guard = configuration_guard().await?;
    let reference = db.run(|db| db.remove_assistant_im_connection()).await?;
    if let Some(reference) = reference {
        if ha_core::blocking::run_blocking(move || credentials::remove(&reference))
            .await
            .is_err()
        {
            app_warn!(
                "channel",
                "personal_assistant_im",
                "Retired assistant IM credential cleanup remains pending"
            );
        }
    }
    Ok(())
}

pub async fn set_notifications_paused(db: Arc<SessionDB>, paused: bool) -> Result<()> {
    db.run(move |db| db.set_assistant_im_notifications_paused(paused))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::personal_assistant_im::{
        IngressOutcome, PairingState, TelegramInput, TelegramInputKind,
    };

    #[tokio::test]
    async fn confirmation_needs_no_local_delivery_owner_or_platform_network() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.db");
        let secondary = Arc::new(SessionDB::open(&path).unwrap());
        let timestamp = now();
        secondary
            .save_assistant_im_connection(
                100,
                "synthetic_bot",
                &uuid::Uuid::new_v4().to_string(),
                timestamp,
            )
            .unwrap();
        let code = secondary.begin_assistant_im_pairing(timestamp).unwrap();
        let lease = secondary
            .assistant_im_poll_lease(timestamp)
            .unwrap()
            .unwrap();
        assert_eq!(
            secondary
                .receive_assistant_im_update(
                    &lease,
                    TelegramInput {
                        update_id: 1,
                        sent_at: Some(timestamp),
                        user_id: 42,
                        chat_id: 42,
                        kind: TelegramInputKind::PairCode(code.code),
                    },
                    timestamp
                )
                .unwrap(),
            IngressOutcome::PairingCandidate
        );
        // No runtime initialization, secret file, or Telegram service exists.
        // The GUI only commits the owner's intent to the shared database.
        confirm_pairing(secondary.clone(), lease.generation, 42, 42)
            .await
            .unwrap();
        assert_eq!(
            secondary.assistant_im_connection().unwrap().unwrap().state,
            PairingState::AwaitingActivation
        );
        let primary = SessionDB::open(&path).unwrap();
        let activation = primary
            .prepare_assistant_im_activation(timestamp)
            .unwrap()
            .unwrap();
        assert_eq!(activation.prompt.chat_id, 42);
        assert_eq!(activation.prompt.lease.generation, lease.generation);
    }
}
