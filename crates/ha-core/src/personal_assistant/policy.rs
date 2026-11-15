//! Shared requested/applied gate; cached config is never send/research authority.
use crate::session::SessionDB;
use anyhow::{anyhow, bail, Result};
pub use ha_config_schema::personal_assistant::{
    AssistantNoticeRoute, AssistantPolicyConfig, AssistantPolicySettings,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantPolicyStatus {
    pub requested_revision: i64,
    pub applied_revision: i64,
    pub settings: AssistantPolicySettings,
    pub pending: bool,
    pub failed: bool,
}

pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS assistant_policy_gate(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        requested_revision INTEGER NOT NULL,
        applied_revision INTEGER NOT NULL,
        requested_json TEXT NOT NULL,
        applied_json TEXT NOT NULL,
        failed INTEGER NOT NULL DEFAULT 0
    );",
    )?;
    Ok(())
}

fn validate(settings: &AssistantPolicySettings) -> Result<()> {
    if settings.quiet_start_hour > 23 || settings.quiet_end_hour > 23 {
        bail!("Invalid quiet hours");
    }
    if let Some(timezone) = &settings.timezone {
        if timezone.parse::<chrono_tz::Tz>().is_err() {
            bail!("An IANA owner timezone is required");
        }
    }
    Ok(())
}

pub(crate) fn current(conn: &Connection) -> Result<Option<AssistantPolicyConfig>> {
    let row: Option<(i64,i64,String,bool)> = conn.query_row(
        "SELECT requested_revision,applied_revision,applied_json,failed FROM assistant_policy_gate WHERE singleton=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    row.map(|(requested, applied, json, failed)| {
        if requested != applied || failed {
            return Ok(None);
        }
        let settings: AssistantPolicySettings = serde_json::from_str(&json)?;
        validate(&settings)?;
        Ok(Some(AssistantPolicyConfig {
            revision: applied,
            settings,
        }))
    })
    .transpose()
    .map(Option::flatten)
}

impl SessionDB {
    pub fn assistant_policy_status(&self) -> Result<AssistantPolicyStatus> {
        let conn = self.read_conn()?;
        let row:Option<(i64,i64,String,bool)> = conn.query_row(
            "SELECT requested_revision,applied_revision,requested_json,failed FROM assistant_policy_gate WHERE singleton=1",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((requested, applied, json, failed)) = row else {
            return Ok(AssistantPolicyStatus {
                requested_revision: 0,
                applied_revision: 0,
                settings: AssistantPolicySettings::default(),
                pending: true,
                failed: false,
            });
        };
        Ok(AssistantPolicyStatus {
            requested_revision: requested,
            applied_revision: applied,
            settings: serde_json::from_str(&json)?,
            pending: requested != applied,
            failed,
        })
    }

    pub fn request_assistant_policy(
        &self,
        expected: i64,
        settings: AssistantPolicySettings,
    ) -> Result<AssistantPolicyStatus> {
        validate(&settings)?;
        if expected < 0 {
            bail!("Invalid policy revision")
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<i64> = tx
            .query_row(
                "SELECT requested_revision FROM assistant_policy_gate WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if old.unwrap_or(0) != expected {
            bail!("Assistant policy changed; reload before saving");
        }
        let revision = expected
            .checked_add(1)
            .ok_or_else(|| anyhow!("Policy revision exhausted"))?;
        if settings.paused {
            tx.execute("UPDATE assistant_items SET state='stopped',reason='assistant_paused',revision=revision+1,stop_epoch=stop_epoch+1 WHERE state IN('open','waiting')",[])?;
            tx.execute(
                "UPDATE assistant_followups SET state='cancelled' WHERE state='pending'",
                [],
            )?;
            tx.execute("UPDATE ask_user_questions SET status='cancelled',answered_at=?1 WHERE status='pending' AND request_id IN(SELECT question_id FROM assistant_item_questions)",[chrono::Utc::now().to_rfc3339()])?;
            let sources = {
                let mut statement =
                    tx.prepare("SELECT id FROM assistant_sources WHERE state='active'")?;
                let rows = statement
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let now = chrono::Utc::now().timestamp();
            for source in sources {
                super::research::quarantine_source(&tx, &source, now)?;
            }
            tx.execute("UPDATE assistant_sources SET body=NULL,version=NULL,previous_body=NULL,previous_version=NULL,next_scan_at=0,next_review_at=?1 WHERE state='active'",[now+super::research::REVIEW_SECONDS])?;
            tx.execute(
                "DELETE FROM assistant_source_events WHERE run_id IS NULL",
                [],
            )?;
        }
        super::research::changed(&tx)?;
        let json = serde_json::to_string(&settings)?;
        let defaults = serde_json::to_string(&AssistantPolicySettings::default())?;
        tx.execute("INSERT INTO assistant_policy_gate(singleton,requested_revision,applied_revision,requested_json,applied_json)
            VALUES(1,?1,0,?2,?3) ON CONFLICT(singleton) DO UPDATE SET requested_revision=excluded.requested_revision,
            requested_json=excluded.requested_json,failed=0",params![revision,json,defaults])?;
        tx.commit()?;
        drop(conn);
        super::research::emit_research_hint(settings.paused);
        self.assistant_policy_status()
    }
}

/// Only Primary writes the standard config then publishes the derived gate.
/// A newer request racing config IO leaves the gate closed until its own ack.
pub async fn apply_policy(db: Arc<SessionDB>) -> Result<()> {
    if !crate::runtime_lock::is_primary() {
        return Ok(());
    }
    let mut initial = crate::config::cached_config().assistant_policy.clone();
    let initialized = db
        .run(|db| -> Result<bool> {
            let conn = db.read_conn()?;
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_policy_gate WHERE singleton=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if !initialized
        && initial.revision == 0
        && initial.settings == AssistantPolicySettings::default()
    {
        let legacy = db
            .run(|db| -> Result<bool> {
                let active = db.assistant_im_connection()?.is_some_and(|connection| {
                    connection.state == crate::personal_assistant_im::PairingState::Active
                });
                Ok(active
                    && db
                        .assistant_im_grants(chrono::Utc::now().timestamp())?
                        .iter()
                        .any(|grant| {
                            grant.completion_notice
                                && grant.expires_at > chrono::Utc::now().timestamp()
                        }))
            })
            .await?;
        if legacy {
            initial.revision = 1;
            initial.settings.notice_route = AssistantNoticeRoute::Im;
            initial.settings.timezone =
                crate::blocking::run_blocking(|| -> Result<Option<String>> {
                    Ok(crate::user_config::load_user_config()?.timezone)
                })
                .await
                .unwrap_or(None)
                .filter(|name| name.parse::<chrono_tz::Tz>().is_ok());
        }
    }
    validate(&initial.settings)?;
    let intent=db.run(move |db| -> Result<AssistantPolicyConfig> {
        let mut conn=db.conn.lock().map_err(|e|anyhow!("Lock error: {e}"))?;
        let tx=conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let json=serde_json::to_string(&initial.settings)?;
        tx.execute("INSERT OR IGNORE INTO assistant_policy_gate(singleton,requested_revision,applied_revision,requested_json,applied_json)
            VALUES(1,?1,?1,?2,?2)",params![initial.revision,json])?;
        let (revision,json):(i64,String)=tx.query_row("SELECT requested_revision,requested_json FROM assistant_policy_gate WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
        tx.commit()?;
        Ok(AssistantPolicyConfig{revision,settings:serde_json::from_str(&json)?})
    }).await?;
    let id = intent.revision;
    let published = intent.clone();
    let saved = if crate::config::cached_config().assistant_policy == intent {
        Ok(())
    } else {
        crate::config::mutate_config_async(("personal_assistant", "owner_policy"), move |config| {
            config.assistant_policy = published;
            Ok(())
        })
        .await
    };
    let ok = saved.is_ok();
    let acknowledged=db.run(move |db| -> Result<bool> {
        let conn=db.conn.lock().map_err(|e|anyhow!("Lock error: {e}"))?;
        let changed=if ok {
            conn.execute("UPDATE assistant_policy_gate SET applied_revision=requested_revision,applied_json=requested_json,failed=0
                WHERE singleton=1 AND requested_revision=?1 AND (applied_revision<>requested_revision OR failed<>0 OR applied_json<>requested_json)",[id])?
        }else{
            conn.execute("UPDATE assistant_policy_gate SET failed=1 WHERE singleton=1 AND requested_revision=?1 AND failed=0",[id])?
        };
        if changed>0 {super::research::changed(&conn)?;}
        Ok(changed>0)
    }).await?;
    if acknowledged {
        super::research::emit_research_hint(false);
    }
    saved
}

pub(crate) fn requested_paused(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_policy_gate WHERE singleton=1 AND json_extract(requested_json,'$.paused')=1)",[],|r|r.get(0))?)
}
