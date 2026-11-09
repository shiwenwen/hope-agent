use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ha_core::personal_assistant_im::{AssistantImError, BotPurpose, IngressOutcome, PollLease};
use ha_core::session::SessionDB;

use super::{credentials, now, telegram, TelegramClient};

pub fn spawn() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if ha_core::channel_hooks::im_delivery_ownership()
        != ha_core::channel_hooks::ImDeliveryOwnership::LocalOwner
    {
        return;
    }
    let Some(db) = ha_core::get_session_db().cloned() else {
        return;
    };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        if db
            .run(|db| {
                db.recover_assistant_im_deliveries()?;
                db.recover_assistant_im_activations()?;
                db.recover_assistant_im_commands()?;
                anyhow::Ok(())
            })
            .await
            .is_err()
        {
            app_warn!(
                "channel",
                "personal_assistant_im",
                "Assistant IM recovery failed"
            );
            return;
        }
        tokio::join!(
            poll_loop(db.clone()),
            command_loop(db.clone()),
            delivery_loop(db)
        );
    });
}

struct ClientCache {
    credential_ref: String,
    client: TelegramClient,
}

async fn client_for<'a>(
    cache: &'a mut Option<ClientCache>,
    lease: &PollLease,
) -> Result<&'a TelegramClient> {
    let matches = cache.as_ref().is_some_and(|cached| {
        cached.credential_ref == lease.credential_ref && cached.client.bot_id == lease.bot_id
    });
    if !matches {
        *cache = None;
        let reference = lease.credential_ref.clone();
        let token = ha_core::blocking::run_blocking(move || credentials::read(&reference)).await?;
        let client = TelegramClient::connect(&token).await?;
        if client.bot_id != lease.bot_id {
            return Err(AssistantImError::StaleGeneration.into());
        }
        *cache = Some(ClientCache {
            credential_ref: lease.credential_ref.clone(),
            client,
        });
    }
    Ok(&cache.as_ref().ok_or(AssistantImError::Unavailable)?.client)
}

async fn poll_loop(db: Arc<SessionDB>) {
    let mut cache = None;
    let mut failed = false;
    loop {
        if poll_once(&db, &mut cache).await.is_err() {
            if !failed {
                app_warn!(
                    "channel",
                    "personal_assistant_im",
                    "Assistant IM polling is unavailable; retrying"
                );
                failed = true;
            }
            cache = None;
            tokio::time::sleep(Duration::from_secs(10)).await;
        } else {
            if failed {
                app_info!(
                    "channel",
                    "personal_assistant_im",
                    "Assistant IM polling failure cleared"
                );
                failed = false;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

async fn poll_once(db: &Arc<SessionDB>, cache: &mut Option<ClientCache>) -> Result<()> {
    db.run(|db| db.expire_assistant_im_state(now())).await?;
    let Some(lease) = db.run(|db| db.assistant_im_poll_lease(now())).await? else {
        *cache = None;
        return Ok(());
    };
    let client = client_for(cache, &lease).await?;
    let bot_id = lease.bot_id;
    let account_id = lease.connection_id.clone();
    db.run(move |db| db.claim_telegram_bot(bot_id, BotPurpose::PersonalAssistant, &account_id))
        .await?;
    let updates = client.updates(lease.cursor).await?;
    for update in updates {
        let input = telegram::decode(&update, lease.bot_id);
        let expected = lease.clone();
        let outcome = db
            .run(move |db| db.receive_assistant_im_update(&expected, input, now()))
            .await?;
        if outcome == IngressOutcome::Activated {
            client.acknowledge_activation(&update).await;
        }
    }
    db.run(move |db| db.assistant_im_poll_completed(&lease, now()))
        .await
}

async fn command_loop(db: Arc<SessionDB>) {
    loop {
        if ha_core::personal_assistant_im::process_pending_commands(db.clone())
            .await
            .is_err()
        {
            // Only a stable reason is logged. Model/provider failures can
            // contain the private prompt or a credential-bearing endpoint.
            app_warn!(
                "channel",
                "personal_assistant_im",
                "Assistant IM command requires attention"
            );
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn delivery_loop(db: Arc<SessionDB>) {
    let mut cache = None;
    let mut failed = false;
    loop {
        if deliver_once(&db, &mut cache).await.is_err() {
            if !failed {
                app_warn!(
                    "channel",
                    "personal_assistant_im",
                    "Assistant IM delivery worker is unavailable; retrying unclaimed work"
                );
                failed = true;
            }
            cache = None;
            tokio::time::sleep(Duration::from_secs(10)).await;
        } else if failed {
            app_info!(
                "channel",
                "personal_assistant_im",
                "Assistant IM delivery worker resumed"
            );
            failed = false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn deliver_once(db: &Arc<SessionDB>, cache: &mut Option<ClientCache>) -> Result<()> {
    let Some(lease) = db.run(|db| db.assistant_im_runtime_snapshot()).await? else {
        *cache = None;
        return Ok(());
    };
    let client = client_for(cache, &lease).await?;
    if let Some(activation) = db
        .run(|db| db.prepare_assistant_im_activation(now()))
        .await?
    {
        if !same_route(&activation.prompt.lease, &lease) {
            return db
                .run(move |db| db.cancel_unsubmitted_assistant_im_activation(&activation.id))
                .await;
        }
        let message_id = client
            .send(
                activation.prompt.chat_id,
                "Hope 已确认此私聊。点击启用后开始接收新指令。",
                Some(&activation.prompt.callback_data),
            )
            .await
            .ok();
        if message_id.is_none() {
            // No raw error is logged: provider URLs can contain the bot token.
            // The submitting marker prohibits another automatic attempt.
            app_warn!(
                "channel",
                "personal_assistant_im",
                "Assistant IM activation delivery is unknown; start a new pairing if needed"
            );
        }
        return db
            .run(move |db| db.finish_assistant_im_activation(&activation.id, message_id))
            .await;
    }
    let Some(delivery) = db.run(|db| db.prepare_assistant_im_delivery(now())).await? else {
        return Ok(());
    };
    if !same_route(&delivery.lease, &lease) {
        // The marker has been crossed, but this caller can prove no HTTP
        // mutation began. A changed route is cancelled, never redirected.
        return db
            .run(move |db| db.cancel_unsubmitted_assistant_im_delivery(&delivery.id))
            .await;
    }
    let message_id = client
        .send(delivery.chat_id, &delivery.text, None)
        .await
        .ok();
    db.run(move |db| db.finish_assistant_im_delivery(&delivery.id, message_id))
        .await
}

fn same_route(left: &PollLease, right: &PollLease) -> bool {
    left.credential_ref == right.credential_ref
        && left.bot_id == right.bot_id
        && left.generation == right.generation
        && left.connection_id == right.connection_id
}
