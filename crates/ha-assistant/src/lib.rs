//! Assistant coordination. Kernel services own all durable state and authority.
use anyhow::Result;

pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            start,
        )
        .expect("ha-assistant startup registers once");
    });
}

fn start() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = pump().await {
                ha_core::app_warn!(
                    "chat",
                    "personal_assistant",
                    "Assistant reconciliation deferred: {}",
                    ha_core::logging::redact_sensitive(&error.to_string())
                );
            }
        }
    });
}

async fn pump() -> Result<()> {
    let Some(db) = ha_core::get_session_db().cloned() else {
        return Ok(());
    };
    let mut failure = None;
    for id in db.run(|db| db.assistant_stopped_work()).await? {
        if let Err(error) =
            ha_core::personal_assistant::items::cancel_item_work(db.clone(), id).await
        {
            failure = Some(error);
        }
    }
    let targets = db.run(|db| db.assistant_settlement_targets()).await?;
    for (execution, _) in targets {
        if let Err(error) = db
            .run(move |db| db.settle_assistant_execution(&execution))
            .await
        {
            failure = Some(error);
        }
    }
    db.run(|db| db.queue_assistant_followups()).await?;
    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(())
    }
}
