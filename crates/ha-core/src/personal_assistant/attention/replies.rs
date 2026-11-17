//! Mobile replies carry a fresh platform event and an immutable sent-message
//! reference. Quoted platform text is never accepted as authority or context.
use super::*;
use crate::personal_assistant::research::queries::{self, SourceKey};
use crate::personal_assistant_im::{Connection as ImConnection, IngressOutcome};

#[derive(Debug)]
struct SentItem {
    candidate: Candidate,
    disclosures: Option<String>,
    grant_revision: Option<i64>,
}
fn sent_item(conn: &Connection, delivery: &str, ordinal: i64) -> Result<Option<SentItem>> {
    Ok(conn.query_row("SELECT logical_key,kind,subject_id,subject_revision,source_run_id,source_set_json,home_session_id,global_stop_epoch,home_stop_epoch,expires_at,disclosure_set_json,im_grant_revision
        FROM assistant_attention_delivery_items WHERE delivery_id=?1 AND ordinal=?2",params![delivery,ordinal],|row|Ok(SentItem{
            candidate:Candidate{key:row.get(0)?,kind:row.get(1)?,subject:row.get(2)?,revision:row.get(3)?,source_run:row.get(4)?,source_set:row.get(5)?,home:row.get(6)?,global_epoch:row.get(7)?,home_epoch:row.get(8)?,created:0,ready:0,expires:row.get(9)?,im_reply:None},disclosures:row.get(10)?,grant_revision:row.get(11)?})).optional()?)
}
fn sent_target(conn: &Connection, candidate: &Candidate) -> Result<Option<(String, String)>> {
    let row:Option<(String,String)>=conn.query_row("SELECT i.id,r.target_session_id FROM assistant_updates u JOIN assistant_items i ON i.id=u.item_id JOIN personal_assistant_requests r ON r.id=i.request_id WHERE u.id=?1 AND u.source_revision=?2 AND i.revision=?2 AND i.state<>'stopped'",params![candidate.subject,candidate.revision],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((item, _)) = row.as_ref() {
        if let Err(error) = super::super::items::validate_item(conn, item) {
            if error
                .downcast_ref::<super::super::AssistantError>()
                .is_some()
            {
                return Ok(None);
            }
            return Err(error);
        }
    }
    Ok(row)
}
fn sent_current(
    conn: &Connection,
    state: &ImConnection,
    item: &SentItem,
    content: bool,
    now: i64,
) -> Result<bool> {
    if !boundary_current(conn, &item.candidate, now)? || claims::settings(conn)?.is_none() {
        return Ok(false);
    }
    if !content {
        return Ok(true);
    }
    if matches!(item.candidate.kind.as_str(), "finding" | "im_query") {
        let Some(json) = item.candidate.source_set.as_deref() else {
            return Ok(false);
        };
        let Some(disclosures) = item.disclosures.as_deref() else {
            return Ok(false);
        };
        let keys: Vec<SourceKey> = serde_json::from_str(json)?;
        if !disclosure::captured_current(conn, state, &keys, disclosures, now)? {
            return Ok(false);
        }
        let Some(run) = item.candidate.source_run.as_deref() else {
            return Ok(false);
        };
        // The original run may be replaced by semantic dedupe, but its full
        // input identity must remain current and committed. Reuse only the
        // actual bounded text that was disclosed, never the latest finding.
        let committed:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_runs r JOIN chat_stream_runs stream ON stream.run_id=r.execution_id WHERE r.id=?1 AND r.state='finished' AND stream.status='committed')",[run],|row|row.get(0))?;
        return Ok(committed
            && crate::personal_assistant::research::input_set_current(conn, run, now)?
            && serde_json::to_string(&queries::keys_for_run(conn, run)?)? == json);
    }
    let Some((_, target)) = sent_target(conn, &item.candidate)? else {
        return Ok(false);
    };
    if item.candidate.kind=="question" && !conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_updates u JOIN ask_user_questions q ON u.event_key='question:' || q.request_id WHERE u.id=?1 AND q.status='pending' AND (q.timeout_at IS NULL OR q.timeout_at>?2))",params![item.candidate.subject,now],|row|row.get::<_,bool>(0))? {return Ok(false);}

    Ok(
        crate::personal_assistant_im::store::grant(conn, &state.view.connection_id, &target)?
            .is_some_and(|grant| {
                Some(grant.revision) == item.grant_revision
                    && grant.expires_at > now
                    && grant.query_status
                    && grant.delegate
            }),
    )
}

pub(crate) fn reply_scope_current(
    conn: &Connection,
    id: &str,
    content: bool,
    now: i64,
) -> Result<bool> {
    let Some(state) = disclosure::pairing(conn)? else {
        return Ok(false);
    };
    type Receipt = (
        String,
        i64,
        i64,
        i64,
        String,
        i64,
        String,
        i64,
        i64,
        i64,
        String,
    );
    let row:Option<Receipt>=conn.query_row("SELECT connection_id,generation,user_id,chat_id,delivery_id,ordinal,home_session_id,global_stop_epoch,home_stop_epoch,expires_at,state FROM assistant_attention_inbounds WHERE id=?1",[id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?))).optional()?;
    let Some((
        connection,
        generation,
        user,
        chat,
        delivery,
        ordinal,
        home,
        global,
        epoch,
        expires,
        status,
    )) = row
    else {
        return Ok(false);
    };
    if connection != state.view.connection_id
        || generation != state.view.generation
        || Some(user) != state.view.user_id
        || Some(chat) != state.view.chat_id
        || expires <= now
        || status == "invalidated"
    {
        return Ok(false);
    }
    let candidate = Candidate {
        key: String::new(),
        kind: String::new(),
        subject: String::new(),
        revision: 0,
        source_run: None,
        source_set: None,
        home,
        global_epoch: global,
        home_epoch: epoch,
        created: 0,
        ready: 0,
        expires,
        im_reply: None,
    };
    if !boundary_current(conn, &candidate, now)? || claims::settings(conn)?.is_none() {
        return Ok(false);
    }
    if ordinal == 0 {
        return Ok(!content && status == "selection_required");
    }
    let Some(item) = sent_item(conn, &delivery, ordinal)? else {
        return Ok(false);
    };
    sent_current(conn, &state, &item, content, now)
}

pub(crate) fn query_scope_current(conn: &Connection, query: &str, now: i64) -> Result<bool> {
    let reply: Option<Option<String>> = conn
        .query_row(
            "SELECT im_reply_id FROM assistant_research_queries WHERE id=?1",
            [query],
            |row| row.get(0),
        )
        .optional()?;
    match reply {
        Some(Some(reply)) => reply_scope_current(conn, &reply, true, now),
        Some(None) => Ok(true),
        None => Ok(false),
    }
}

pub(crate) fn invalidate_im_queries(
    conn: &Connection,
    source: Option<&str>,
    now: i64,
) -> Result<()> {
    let ids = {
        let mut statement=conn.prepare("SELECT inbound.id FROM assistant_attention_inbounds inbound WHERE inbound.state<>'invalidated' AND (?1 IS NULL OR EXISTS(SELECT 1 FROM assistant_attention_delivery_items item,json_each(item.source_set_json) refs WHERE item.delivery_id=inbound.delivery_id AND (item.ordinal=inbound.ordinal OR inbound.ordinal=0) AND json_extract(refs.value,'$.sourceId')=?1))")?;
        let rows = statement
            .query_map([source], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for id in ids {
        conn.execute(
            "UPDATE assistant_attention_inbounds SET state='invalidated' WHERE id=?1",
            [&id],
        )?;
        conn.execute("UPDATE assistant_research_queries SET state='cancelled',reason='scope_changed',question='',prior_finding=NULL WHERE im_reply_id=?1",[&id])?;
        conn.execute("UPDATE assistant_research_runs SET state='invalidated',outcome_reason='scope_changed',ended_at=?2,question=NULL,prior_finding=NULL,reported_json=NULL WHERE query_id IN(SELECT id FROM assistant_research_queries WHERE im_reply_id=?1)",params![id,now])?;
        conn.execute("UPDATE assistant_research_inputs SET body=NULL,previous_body=NULL WHERE run_id IN(SELECT run_id FROM assistant_research_queries WHERE im_reply_id=?1)",[&id])?;
        conn.execute("UPDATE assistant_findings SET state='invalidated',body_json='{}' WHERE run_id IN(SELECT run_id FROM assistant_research_queries WHERE im_reply_id=?1)",[&id])?;
        conn.execute("UPDATE sessions SET context_json=NULL,context_revision=context_revision+1,context_run_id=NULL WHERE id IN(SELECT session_id FROM assistant_research_runs WHERE query_id IN(SELECT id FROM assistant_research_queries WHERE im_reply_id=?1))",[&id])?;
    }
    Ok(())
}
fn selection(text: &str, count: i64) -> Option<(i64, &str)> {
    if count == 1 {
        return Some((1, text));
    }
    let (index, question) = text.split_once(':').or_else(|| text.split_once('：'))?;
    let ordinal = index.trim().parse::<i64>().ok()?;
    (ordinal >= 1 && ordinal <= count && !question.trim().is_empty())
        .then_some((ordinal, question.trim()))
}

/// None means an exactly proven ordinary dialog reply; a known attention reference,
/// including an expired one, can never fall through to ordinary conversation.
pub(crate) fn consume_reply(
    conn: &Connection,
    state: &ImConnection,
    update: i64,
    user: i64,
    chat: i64,
    parent: i64,
    text: &str,
    now: i64,
) -> Result<Option<IngressOutcome>> {
    let known:Option<(String,i64,String,i64,i64,String)>=conn.query_row("SELECT id,generation,state,expires_at,(SELECT COUNT(*) FROM assistant_attention_delivery_items item WHERE item.delivery_id=d.id),connection_id FROM assistant_attention_deliveries d WHERE route='im' AND chat_id=?1 AND message_id=?2 ORDER BY submitted_at DESC LIMIT 1",params![chat,parent],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
    let Some((delivery, generation, status, expires, count, connection)) = known else {
        return Ok(
            if crate::personal_assistant_im::delivery::ordinary_reply_current(
                conn, state, parent, now,
            )? {
                None
            } else {
                Some(IngressOutcome::Ignored)
            },
        );
    };
    if connection != state.view.connection_id
        || generation != state.view.generation
        || status != "accepted"
        || expires <= now
        || count == 0
        || claims::settings(conn)?.is_none()
    {
        return Ok(Some(IngressOutcome::Ignored));
    }
    if text.trim().is_empty() || text.len() > 8_000 {
        return Ok(Some(IngressOutcome::Ignored));
    }
    if count == 1 {
        if let Some(item) = sent_item(conn, &delivery, 1)? {
            if item.candidate.kind == "reply_status" {
                // A reply to the host's numbering clarification still refers
                // to the original batch. Never reinterpret it as a new chat.
                let original:Option<i64>=conn.query_row("SELECT d.message_id FROM assistant_attention_inbounds inbound JOIN assistant_attention_deliveries d ON d.id=inbound.delivery_id WHERE inbound.id=?1 AND inbound.state='selection_required' AND inbound.expires_at>?2 AND d.state='accepted'",params![item.candidate.subject,now],|row|row.get(0)).optional()?.flatten();
                return match original {
                    Some(parent) => {
                        consume_reply(conn, state, update, user, chat, parent, text, now)
                    }
                    None => Ok(Some(IngressOutcome::Ignored)),
                };
            }
        }
    }
    let selected = selection(text, count);
    let ordinal = selected.map_or(0, |(ordinal, _)| ordinal);
    let Some(item) = sent_item(conn, &delivery, ordinal.max(1))? else {
        return Ok(Some(IngressOutcome::Ignored));
    };
    if !sent_current(conn, state, &item, selected.is_some(), now)? {
        return Ok(Some(IngressOutcome::Ignored));
    }
    if matches!(item.candidate.kind.as_str(), "finding" | "im_query") {
        let full: i64 = conn.query_row(
            "SELECT COUNT(*) FROM assistant_research_queries WHERE state IN('pending','running')",
            [],
            |row| row.get(0),
        )?;
        if full >= 8 {
            return Ok(Some(IngressOutcome::Ignored));
        }
    }
    if item.candidate.kind == "item" {
        let full: i64 = conn.query_row(
            "SELECT COUNT(*) FROM assistant_im_commands WHERE state IN('pending','running')",
            [],
            |row| row.get(0),
        )?;
        if full >= 64 {
            return Ok(Some(IngressOutcome::Ignored));
        }
    }
    if selected.is_some() {
        conn.execute("UPDATE assistant_attention_inbounds SET state='selection_resolved' WHERE connection_id=?1 AND generation=?2 AND delivery_id=?3 AND state='selection_required'",params![state.view.connection_id,state.view.generation,delivery])?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute("INSERT INTO assistant_attention_inbounds(id,connection_id,generation,cursor_epoch,update_id,user_id,chat_id,delivery_id,ordinal,created_at,expires_at,home_session_id,global_stop_epoch,home_stop_epoch,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![id,state.view.connection_id,state.view.generation,state.cursor_epoch,update,user,chat,delivery,ordinal,now,expires.min(now+86_400),item.candidate.home,item.candidate.global_epoch,item.candidate.home_epoch,if selected.is_some(){"active"}else{"selection_required"}])?;
    let Some((_, question)) = selected else {
        conn.execute("INSERT INTO assistant_attention_candidates(logical_key,kind,subject_id,subject_revision,home_session_id,global_stop_epoch,home_stop_epoch,created_at,ready_at,expires_at,im_reply_id) VALUES(?1,'reply_status',?2,1,?3,?4,?5,?6,?6,?7,?2)",params![format!("reply:{id}:selection"),id,item.candidate.home,item.candidate.global_epoch,item.candidate.home_epoch,now,expires])?;
        return Ok(Some(IngressOutcome::Queued));
    };
    match item.candidate.kind.as_str() {
        "finding" | "im_query" => {
            let keys: Vec<SourceKey> = serde_json::from_str(
                item.candidate
                    .source_set
                    .as_deref()
                    .ok_or_else(|| anyhow!("Missing original source scope"))?,
            )?;
            let prior:String=conn.query_row("SELECT body FROM assistant_attention_payloads WHERE delivery_id=?1 AND ordinal=?2 AND expires_at>?3",params![delivery,ordinal,now],|row|row.get(0))?;
            let query = queries::insert_question(
                conn,
                &format!("im:{id}"),
                &item.candidate.home,
                question,
                None,
                item.candidate.source_run.as_deref(),
                &keys,
                Some(&prior),
                now,
            )?;
            conn.execute(
                "UPDATE assistant_research_queries SET im_reply_id=?2 WHERE id=?1",
                params![query.id, id],
            )?;
            conn.execute(
                "UPDATE assistant_attention_inbounds SET query_id=?2 WHERE id=?1",
                params![id, query.id],
            )?;
        }
        "question" => {
            let (request,payload):(String,String)=conn.query_row("SELECT q.request_id,q.payload FROM assistant_updates u JOIN ask_user_questions q ON u.event_key='question:' || q.request_id WHERE u.id=?1 AND q.status='pending'",[&item.candidate.subject],|row|Ok((row.get(0)?,row.get(1)?)))?;
            let group: crate::ask_user::AskUserQuestionGroup = serde_json::from_str(&payload)?;
            let target = group
                .owner_response
                .as_ref()
                .and_then(|owner| owner.assistant_item.as_ref())
                .ok_or_else(|| anyhow!("Question no longer available"))?;
            let answers: Vec<crate::ask_user::AskUserQuestionAnswer> = serde_json::from_value(
                serde_json::json!([{"questionId":"decision","selected":[],"customInput":question,"files":[]}]),
            )?;
            super::super::items::answer_with_conn(conn, &request, target, &answers)?;
            conn.execute(
                "UPDATE assistant_attention_inbounds SET state='answered',question_id=?2 WHERE id=?1",
                params![id,request],
            )?;
        }
        "item" => {
            // A continuation contains only the new owner's text and one host
            // selected target grant. It never inserts private source text or
            // the broad assistant history into an ordinary IM model view.
            let (_, target) =
                sent_target(conn, &item.candidate)?.ok_or_else(|| anyhow!("The work changed"))?;
            let grant = crate::personal_assistant_im::store::grant(
                conn,
                &state.view.connection_id,
                &target,
            )?
            .ok_or_else(|| anyhow!("The work grant changed"))?;
            let mut revisions = std::collections::BTreeMap::new();
            revisions.insert(target.clone(), grant.revision);
            let epoch = super::super::stop_epoch(conn, &target)?;
            let mut epochs = std::collections::BTreeMap::new();
            epochs.insert(target, epoch);
            let pending: i64 = conn.query_row(
                "SELECT COUNT(*) FROM assistant_im_commands WHERE state IN('pending','running')",
                [],
                |row| row.get(0),
            )?;
            if pending >= 64 {
                return Ok(Some(IngressOutcome::Ignored));
            }
            conn.execute("INSERT INTO assistant_im_commands(request_id,connection_id,generation,text,grant_revisions,global_stop_epoch,home_stop_epoch,target_stop_epochs,source_view_id,view_stop_epoch,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,NULL,0,'pending',?9)",params![id,state.view.connection_id,state.view.generation,question,serde_json::to_string(&revisions)?,item.candidate.global_epoch,item.candidate.home_epoch,serde_json::to_string(&epochs)?,now])?;
        }
        _ => {
            conn.execute(
                "UPDATE assistant_attention_inbounds SET state='invalidated' WHERE id=?1",
                [id],
            )?;
            return Ok(Some(IngressOutcome::Ignored));
        }
    }
    Ok(Some(IngressOutcome::Queued))
}
