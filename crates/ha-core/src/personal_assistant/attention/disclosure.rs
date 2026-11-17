use super::*;
use crate::personal_assistant::research::queries::{keys_current, SourceKey};
use crate::personal_assistant_im::{Connection as ImConnection, PairingState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DisclosureKey {
    pub source_id: String,
    pub source_revision: i64,
    pub disclosure_revision: i64,
}

pub(super) fn pairing(conn: &Connection) -> Result<Option<ImConnection>> {
    Ok(
        crate::personal_assistant_im::store::load(conn)?.filter(|state| {
            state.view.state == PairingState::Active
                && state.view.user_id.is_some()
                && state.view.user_id == state.view.chat_id
        }),
    )
}
pub(super) fn capture(
    conn: &Connection,
    state: &ImConnection,
    keys: &[SourceKey],
    now: i64,
) -> Result<Option<Vec<DisclosureKey>>> {
    if !keys_current(conn, keys, now)? {
        return Ok(None);
    }
    let mut result = Vec::new();
    for key in keys {
        let revision:Option<i64>=conn.query_row("SELECT revision FROM assistant_source_im_disclosures WHERE source_id=?1 AND source_revision=?2 AND connection_id=?3 AND generation=?4 AND enabled=1 AND expires_at>?5",
            params![key.source_id,key.revision,state.view.connection_id,state.view.generation,now],|row|row.get(0)).optional()?;
        let Some(revision) = revision else {
            return Ok(None);
        };
        result.push(DisclosureKey {
            source_id: key.source_id.clone(),
            source_revision: key.revision,
            disclosure_revision: revision,
        });
    }
    Ok(Some(result))
}
pub(super) fn all_disclosed(
    conn: &Connection,
    state: &ImConnection,
    keys: &[SourceKey],
    now: i64,
) -> Result<bool> {
    Ok(capture(conn, state, keys, now)?.is_some())
}
pub(super) fn captured_current(
    conn: &Connection,
    state: &ImConnection,
    keys: &[SourceKey],
    snapshot: &str,
    now: i64,
) -> Result<bool> {
    let expected: Vec<DisclosureKey> = serde_json::from_str(snapshot)?;
    let Some(actual) = capture(conn, state, keys, now)? else {
        return Ok(false);
    };
    Ok(serde_json::to_string(&actual)? == serde_json::to_string(&expected)?)
}
pub(crate) fn source_disclosure_status(
    conn: &Connection,
    source: &str,
    revision: i64,
    now: i64,
) -> Result<(bool, i64)> {
    let Some(state) = pairing(conn)? else {
        return Ok((false, 0));
    };
    let row:Option<(bool,i64)>=conn.query_row("SELECT d.enabled=1 AND d.expires_at>?5 AND s.state='active' AND s.expires_at>?5,d.revision FROM assistant_source_im_disclosures d JOIN assistant_sources s ON s.id=d.source_id AND s.revision=d.source_revision
        WHERE d.source_id=?1 AND d.source_revision=?2 AND d.connection_id=?3 AND d.generation=?4",
        params![source,revision,state.view.connection_id,state.view.generation,now],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    Ok(row.unwrap_or((false, 0)))
}
impl SessionDB {
    pub fn set_assistant_source_im_disclosure(
        &self,
        source: &str,
        revision: i64,
        enabled: bool,
        expected_disclosure_revision: i64,
        expected_connection: &str,
        expected_generation: i64,
        now: i64,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Source disclosure unavailable"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expiry:Option<i64>=tx.query_row("SELECT expires_at FROM assistant_sources WHERE id=?1 AND revision=?2 AND state='active' AND expires_at>?3",params![source,revision,now],|row|row.get(0)).optional()?;
        let expiry =
            expiry.ok_or_else(|| anyhow!("The shared source changed; reload before saving"))?;
        let state = pairing(&tx)?
            .ok_or_else(|| anyhow!("Pair your dedicated assistant IM before sharing summaries"))?;
        if state.view.connection_id != expected_connection
            || state.view.generation != expected_generation
        {
            bail!("The assistant IM pairing changed; review it before sharing summaries");
        }
        let previous:Option<(i64,bool)>=tx.query_row("SELECT revision,enabled FROM assistant_source_im_disclosures WHERE source_id=?1 AND source_revision=?2 AND connection_id=?3 AND generation=?4",params![source,revision,expected_connection,expected_generation],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (old, was_enabled) = previous.unwrap_or((0, false));
        if old != expected_disclosure_revision {
            bail!("Source disclosure changed; reload before saving");
        }
        if was_enabled != enabled {
            let next = old
                .checked_add(1)
                .ok_or_else(|| anyhow!("Disclosure revision exhausted"))?;
            tx.execute("INSERT INTO assistant_source_im_disclosures(source_id,source_revision,connection_id,generation,revision,enabled,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)
                ON CONFLICT(source_id,source_revision,connection_id,generation) DO UPDATE SET revision=excluded.revision,enabled=excluded.enabled,expires_at=excluded.expires_at",params![source,revision,expected_connection,expected_generation,next,enabled,expiry])?;
            if !enabled {
                replies::invalidate_im_queries(&tx, Some(source), now)?;
                tx.execute("DELETE FROM assistant_attention_payloads WHERE delivery_id IN(SELECT item.delivery_id FROM assistant_attention_delivery_items item,json_each(item.source_set_json) ref WHERE json_extract(ref.value,'$.sourceId')=?1)",[source])?;
            }
            super::super::research::changed(&tx)?;
        }
        tx.commit()?;
        drop(conn);
        super::super::research::emit_research_hint(false);
        changed();
        Ok(())
    }
}
