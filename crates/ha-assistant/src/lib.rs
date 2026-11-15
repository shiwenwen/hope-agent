//! Assistant coordination. Kernel services own all durable state and authority.
use anyhow::Result;

pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::personal_assistant::research::register_coordinator();
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            start,
        )
        .expect("ha-assistant startup registers once");
    });
}

fn start() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Some(db) = ha_core::get_session_db().cloned() {
                if ha_core::personal_assistant::research::scan_sources(db)
                    .await
                    .is_err()
                {
                    ha_core::app_warn!(
                        "chat",
                        "personal_assistant",
                        "Source observation could not be verified"
                    );
                }
            }
        }
    });
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
    let mut failure = ha_core::personal_assistant::policy::apply_policy(db.clone())
        .await
        .err();
    static LAST_REVISION: std::sync::Mutex<Option<(i64, i64)>> = std::sync::Mutex::new(None);
    match db.run(|db| db.assistant_scope_change_revision()).await {
        Ok(revision) => {
            let mut previous = LAST_REVISION
                .lock()
                .map_err(|_| anyhow::anyhow!("Assistant revision lock unavailable"))?;
            if *previous != Some(revision) {
                ha_core::personal_assistant::research::emit_research_hint(
                    previous.is_none_or(|value| value.1 != revision.1),
                );
                *previous = Some(revision);
            }
        }
        Err(error) => failure = Some(error),
    }
    if let Err(error) =
        ha_core::personal_assistant::research::process_pending_research(db.clone()).await
    {
        failure = Some(error);
    }
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
    db.run(|db| {
        db.queue_assistant_followups()?;
        db.reconcile_assistant_attention(ha_core::personal_assistant::attention::now())
    })
    .await?;
    if let Some(error) = failure {
        Err(error)
    } else {
        Ok(())
    }
}
