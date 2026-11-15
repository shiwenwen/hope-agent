use super::*;
use crate::personal_assistant::research::queries::{keys_for_run, SourceKey};
use std::sync::OnceLock;

pub(super) fn instance_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}
fn unix(value: &str) -> Result<i64> {
    Ok(chrono::DateTime::parse_from_rfc3339(value)?.timestamp())
}
fn save_candidate(conn: &Connection, candidate: &Candidate) -> Result<()> {
    conn.execute("INSERT INTO assistant_attention_candidates(logical_key,kind,subject_id,subject_revision,source_run_id,source_set_json,home_session_id,global_stop_epoch,home_stop_epoch,created_at,ready_at,expires_at,im_reply_id)
        SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13 WHERE NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items WHERE logical_key=?1)
        ON CONFLICT(logical_key) DO UPDATE SET source_run_id=excluded.source_run_id,source_set_json=excluded.source_set_json,expires_at=MIN(assistant_attention_candidates.expires_at,excluded.expires_at)",
        params![candidate.key,candidate.kind,candidate.subject,candidate.revision,candidate.source_run,candidate.source_set,candidate.home,candidate.global_epoch,candidate.home_epoch,candidate.created,candidate.ready,candidate.expires,candidate.im_reply])?;
    Ok(())
}
pub(super) fn collect_candidates(conn: &Connection, now: i64) -> Result<()> {
    // Only the latest meaningful update for each item may escalate. Routine
    // progress or external polling stays local even when many tools completed.
    let updates = {
        let mut statement=conn.prepare("SELECT u.id,u.kind,u.source_revision,u.created_at,r.home_session_id,r.global_stop_epoch,r.source_stop_epoch,i.expires_at
            FROM assistant_updates u JOIN assistant_items i ON i.id=u.item_id JOIN personal_assistant_requests r ON r.id=i.request_id
            WHERE u.viewed_at IS NULL AND u.source_revision=i.revision AND i.state IN('open','waiting','done') AND i.expires_at>?1
              AND u.kind IN('result','question','blocked','needs_review')
              AND CAST(strftime('%s',u.created_at) AS INTEGER)>?2
              AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key='update:' || u.id || ':' || u.source_revision)
              AND NOT EXISTS(SELECT 1 FROM assistant_updates newer WHERE newer.item_id=u.item_id AND newer.rowid>u.rowid)
            ORDER BY u.created_at,u.id LIMIT 200")?;
        let rows = statement
            .query_map(params![now, now - 86_400], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, kind, revision, created, home, global, epoch, expires) in updates {
        let created = unix(&created)?;
        let question = kind == "question";
        save_candidate(
            conn,
            &Candidate {
                key: format!("update:{id}:{revision}"),
                kind: if question { "question" } else { "item" }.into(),
                subject: id,
                revision,
                source_run: None,
                source_set: None,
                home,
                global_epoch: global,
                home_epoch: epoch,
                created,
                ready: created + if question { 0 } else { 60 },
                expires: expires.min(created + 86_400),
                im_reply: None,
            },
        )?;
    }
    let approvals = {
        let mut statement=conn.prepare("SELECT h.id,h.item_revision,h.created_at,r.home_session_id,r.global_stop_epoch,r.source_stop_epoch,MIN(h.expires_at,i.expires_at)
            FROM assistant_attention_handoffs h JOIN assistant_items i ON i.id=h.item_id JOIN personal_assistant_requests r ON r.id=i.request_id
            WHERE h.active=1 AND h.expires_at>?1 AND h.item_revision=i.revision AND i.state IN('open','waiting') AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key='approval:' || h.id) ORDER BY h.created_at,h.id LIMIT 64")?;
        let rows = statement
            .query_map([now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, revision, created, home, global, epoch, expires) in approvals {
        save_candidate(
            conn,
            &Candidate {
                key: format!("approval:{id}"),
                kind: "approval".into(),
                subject: id,
                revision,
                source_run: None,
                source_set: None,
                home,
                global_epoch: global,
                home_epoch: epoch,
                created,
                ready: created,
                expires,
                im_reply: None,
            },
        )?;
    }
    let home_handoffs = {
        let mut statement=conn.prepare("SELECT id,kind,home_session_id,global_stop_epoch,home_stop_epoch,created_at,expires_at FROM assistant_attention_home_handoffs WHERE active=1 AND instance_id=?1 AND expires_at>?2 AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key='home-handoff:' || assistant_attention_home_handoffs.id) ORDER BY created_at,id LIMIT 64")?;
        let rows = statement
            .query_map(params![instance_id(), now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, kind, home, global, epoch, created, expires) in home_handoffs {
        save_candidate(
            conn,
            &Candidate {
                key: format!("home-handoff:{id}"),
                kind: format!("home_{kind}"),
                subject: id,
                revision: 0,
                source_run: None,
                source_set: None,
                home,
                global_epoch: global,
                home_epoch: epoch,
                created,
                ready: created,
                expires,
                im_reply: None,
            },
        )?;
    }
    let findings = {
        let mut statement=conn.prepare("SELECT f.id,f.revision,f.updated_at,f.run_id,r.home_session_id,r.global_stop_epoch,r.home_stop_epoch
            FROM assistant_findings f JOIN assistant_research_runs r ON r.id=f.run_id LEFT JOIN assistant_research_queries q ON q.id=r.query_id
            WHERE f.state='active' AND f.viewed_at IS NULL AND r.state='finished' AND q.im_reply_id IS NULL AND (json_extract(f.body_json,'$.attentionUntil') IS NULL OR CAST(strftime('%s',json_extract(f.body_json,'$.attentionUntil')) AS INTEGER)>?1+86400)
              AND CAST(strftime('%s',f.updated_at) AS INTEGER)>?1
              AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key='finding:' || f.id || ':' || f.revision)
            ORDER BY f.updated_at,f.id LIMIT 200")?;
        let rows = statement
            .query_map([now - 86_400], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, revision, created, run, home, global, epoch) in findings {
        let created = unix(&created)?;
        let keys = keys_for_run(conn, &run)?;
        let body: String = conn.query_row(
            "SELECT body_json FROM assistant_findings WHERE id=?1",
            [&id],
            |row| row.get(0),
        )?;
        let finding: crate::personal_assistant::research::ResearchFindingInput =
            serde_json::from_str(&body)?;
        let expires = finding
            .attention_until
            .as_deref()
            .map(unix)
            .transpose()?
            .unwrap_or(created + 86_400)
            .min(created + 86_400);
        save_candidate(
            conn,
            &Candidate {
                key: format!("finding:{id}:{revision}"),
                kind: "finding".into(),
                subject: id,
                revision,
                source_run: Some(run),
                source_set: Some(serde_json::to_string(&keys)?),
                home,
                global_epoch: global,
                home_epoch: epoch,
                created,
                ready: created + 60,
                expires,
                im_reply: None,
            },
        )?;
    }
    // An explicitly requested mobile answer has one immutable disposition. It
    // is excluded from the generic finding path above, so it cannot be sent a
    // second time through the selected proactive channel.
    let answers = {
        let mut statement=conn.prepare("SELECT q.id,q.im_reply_id,q.state,q.run_id,q.home_session_id,q.global_stop_epoch,q.home_stop_epoch,q.created_at
            FROM assistant_research_queries q WHERE q.im_reply_id IS NOT NULL AND q.state IN('finished','failed','cancelled') AND q.created_at>?1 AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key='im-query:' || q.id || ':result') ORDER BY q.created_at,q.id LIMIT 64")?;
        let rows = statement
            .query_map([now - 86_400], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, reply, state, run, home, global, epoch, created) in answers {
        let source_set = run
            .as_deref()
            .map(|run| keys_for_run(conn, run).and_then(|keys| Ok(serde_json::to_string(&keys)?)))
            .transpose()?;
        save_candidate(
            conn,
            &Candidate {
                key: format!("im-query:{id}:result"),
                kind: if state == "finished" {
                    "im_query"
                } else {
                    "query_status"
                }
                .into(),
                subject: id,
                revision: 1,
                source_run: run,
                source_set,
                home,
                global_epoch: global,
                home_epoch: epoch,
                created,
                ready: created,
                expires: created + 86_400,
                im_reply: Some(reply),
            },
        )?;
    }
    conn.execute("DELETE FROM assistant_attention_candidates WHERE expires_at<=?1 OR EXISTS(SELECT 1 FROM assistant_attention_delivery_items item WHERE item.logical_key=assistant_attention_candidates.logical_key)",[now])?;
    // Prune before the bounded selection. Viewed, answered, replaced or
    // revoked candidates must never occupy the entire first scan window.
    let old = {
        let mut statement = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM assistant_attention_candidates"
        ))?;
        let rows = statement
            .query_map([], row_candidate)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    // Pending publication is temporary. In particular a body-free mobile
    // selection prompt must survive until the requested policy is applied.
    if settings(conn)?.is_some() {
        for candidate in old {
            if !current(conn, &candidate, now)? {
                conn.execute(
                    "DELETE FROM assistant_attention_candidates WHERE logical_key=?1",
                    [candidate.key],
                )?;
            }
        }
    }
    conn.execute(
        "DELETE FROM assistant_attention_payloads WHERE expires_at<=?1",
        [now],
    )?;
    Ok(())
}

pub(super) fn settings(
    conn: &Connection,
) -> Result<Option<super::super::policy::AssistantPolicyConfig>> {
    Ok(super::super::policy::current(conn)?.filter(|policy| !policy.settings.paused))
}
fn candidate(conn: &Connection, key: &str) -> Result<Option<Candidate>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM assistant_attention_candidates WHERE logical_key=?1"),
            [key],
            row_candidate,
        )
        .optional()?)
}
fn source_keys(value: Option<&str>) -> Result<Vec<SourceKey>> {
    Ok(serde_json::from_str(value.ok_or_else(|| {
        anyhow!("A current full source snapshot is required")
    })?)?)
}

pub(super) fn item_for_candidate(
    conn: &Connection,
    candidate: &Candidate,
) -> Result<Option<(String, String, String)>> {
    let row = if candidate.kind == "approval" {
        if !signals::handoff_pending(&candidate.subject) {
            return Ok(None);
        }
        conn.query_row("SELECT i.id,r.target_session_id,r.source_turn_id FROM assistant_attention_handoffs h
            JOIN assistant_items i ON i.id=h.item_id JOIN personal_assistant_requests r ON r.id=i.request_id
            JOIN chat_stream_runs stream ON stream.run_id=h.execution_id
            WHERE h.id=?1 AND h.active=1 AND h.item_revision=?2 AND i.revision=?2 AND stream.status='running' AND i.state IN('open','waiting')",
            params![candidate.subject,candidate.revision],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?
    } else {
        conn.query_row("SELECT i.id,r.target_session_id,r.source_turn_id FROM assistant_updates u JOIN assistant_items i ON i.id=u.item_id JOIN personal_assistant_requests r ON r.id=i.request_id
            WHERE u.id=?1 AND u.source_revision=?2 AND i.revision=?2 AND u.viewed_at IS NULL
              AND ((u.kind='result' AND i.state='done') OR (u.kind IN('blocked','needs_review') AND i.state='waiting')
                OR (u.kind='question' AND i.state='waiting' AND EXISTS(SELECT 1 FROM assistant_item_questions binding JOIN ask_user_questions q ON q.request_id=binding.question_id
                    WHERE binding.item_id=i.id AND binding.item_revision=i.revision AND q.status='pending' AND u.event_key='question:' || q.request_id)))
              AND NOT EXISTS(SELECT 1 FROM assistant_updates newer WHERE newer.item_id=u.item_id AND newer.rowid>u.rowid)",params![candidate.subject,candidate.revision],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?
    };
    Ok(row)
}
pub(super) fn current(conn: &Connection, candidate: &Candidate, now: i64) -> Result<bool> {
    if !boundary_current(conn, candidate, now)? {
        return Ok(false);
    }
    let Some(policy) = settings(conn)? else {
        return Ok(false);
    };
    match candidate.kind.as_str() {
        "home_approval" | "home_question" => signals::home_handoff_current(conn, candidate, now),
        "item" | "question" | "approval" => {
            let Some((item, _, _)) = item_for_candidate(conn, candidate)? else {
                return Ok(false);
            };
            Ok(super::super::items::validate_item(conn, &item).is_ok())
        }
        "finding" | "im_query" => {
            let Some(run) = candidate.source_run.as_deref() else {
                return Ok(false);
            };
            let finished:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_runs r JOIN chat_stream_runs stream ON stream.run_id=r.execution_id
                WHERE r.id=?1 AND r.state='finished' AND stream.status='committed')",[run],|row|row.get(0))?;
            if !finished || !super::super::research::input_set_current(conn, run, now)? {
                return Ok(false);
            }
            let keys = source_keys(candidate.source_set.as_deref())?;
            let actual = keys_for_run(conn, run)?;
            if serde_json::to_string(&keys)? != serde_json::to_string(&actual)? {
                return Ok(false);
            }
            if candidate.kind == "finding" {
                let active:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_findings WHERE id=?1 AND revision=?2 AND run_id=?3 AND state='active' AND viewed_at IS NULL)",params![candidate.subject,candidate.revision,run],|row|row.get(0))?;
                if !active {
                    return Ok(false);
                }
                let kind: String = conn.query_row(
                    "SELECT kind FROM assistant_research_runs WHERE id=?1",
                    [run],
                    |row| row.get(0),
                )?;
                if kind == "discovery" {
                    if !policy.settings.research_enabled {
                        return Ok(false);
                    }
                    for key in &keys {
                        if !super::super::research::source_autonomy_current(
                            conn,
                            &key.source_id,
                            now,
                        )? {
                            return Ok(false);
                        }
                    }
                }
            } else {
                let Some(reply) = candidate.im_reply.as_deref() else {
                    return Ok(false);
                };
                if !replies::reply_scope_current(conn, reply, true, now)? {
                    return Ok(false);
                }
                let active:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_queries q WHERE q.id=?1 AND q.im_reply_id=?2 AND q.state='finished' AND q.run_id=?3)",params![candidate.subject,reply,run],|row|row.get(0))?;
                if !active {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        "reply_status" => Ok(replies::reply_scope_current(
            conn,
            &candidate.subject,
            false,
            now,
        )?),
        "query_status" => {
            let Some(reply) = candidate.im_reply.as_deref() else {
                return Ok(false);
            };
            Ok(replies::reply_scope_current(conn,reply,false,now)? && conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_research_queries WHERE id=?1 AND im_reply_id=?2 AND state IN('failed','cancelled'))",params![candidate.subject,reply],|row|row.get::<_,bool>(0))?)
        }
        _ => Ok(false),
    }
}
pub(super) fn route_allowed(
    conn: &Connection,
    candidate: &Candidate,
    route: &str,
    now: i64,
) -> Result<bool> {
    if !current(conn, candidate, now)? {
        return Ok(false);
    }
    let Some(policy) = settings(conn)? else {
        return Ok(false);
    };
    if candidate.im_reply.is_some() {
        return Ok(route == "im");
    }
    // Home has no target-session IM grant. A live UI interaction may use
    // only the owner's selected Desktop route, with no content disclosed.
    if matches!(candidate.kind.as_str(), "home_approval" | "home_question") && route != "desktop" {
        return Ok(false);
    }
    let selected = match policy.settings.notice_route {
        super::super::policy::AssistantNoticeRoute::None => "none",
        super::super::policy::AssistantNoticeRoute::Desktop => "desktop",
        super::super::policy::AssistantNoticeRoute::Im => "im",
    };
    if selected != route {
        return Ok(false);
    }
    let boundary:i64=conn.query_row("SELECT COALESCE((SELECT eligible_after FROM assistant_attention_boundary WHERE singleton=1),?1)",[now],|row|row.get(0))?;
    // Boundaries seal their entire UTC second. This conservatively keeps
    // same-second updates in Hope instead of replaying a pre-enable result.
    if candidate.created <= boundary {
        return Ok(false);
    }
    if route == "desktop" {
        return Ok(true);
    }
    let im_boundary: i64 = conn.query_row(
        "SELECT im_eligible_after FROM assistant_attention_boundary WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if candidate.created <= im_boundary {
        return Ok(false);
    }
    let Some(state) = disclosure::pairing(conn)? else {
        return Ok(false);
    };
    if state.view.notifications_paused {
        return Ok(false);
    }
    if candidate.kind == "finding" {
        return disclosure::all_disclosed(
            conn,
            &state,
            &source_keys(candidate.source_set.as_deref())?,
            now,
        );
    }
    let Some((_, target, source_turn)) = item_for_candidate(conn, candidate)? else {
        return Ok(false);
    };
    let from_im: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_im_commands WHERE request_id=?1)",
        [source_turn],
        |row| row.get(0),
    )?;
    if !from_im {
        if !policy.settings.desktop_to_im_notices {
            return Ok(false);
        }
        let threshold: i64 = conn.query_row(
            "SELECT desktop_eligible_after FROM assistant_attention_boundary WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if candidate.created <= threshold {
            return Ok(false);
        }
    }
    Ok(
        crate::personal_assistant_im::store::grant(conn, &state.view.connection_id, &target)?
            .is_some_and(|grant| {
                grant.expires_at > now && grant.query_status && grant.completion_notice
            }),
    )
}

pub(crate) fn read_scope_current(
    conn: &Connection,
    key: &str,
    source: &str,
    revision: i64,
    now: i64,
) -> Result<bool> {
    let Some(candidate) = candidate(conn, key)? else {
        return Ok(false);
    };
    if !matches!(candidate.kind.as_str(), "finding" | "im_query")
        || !current(conn, &candidate, now)?
    {
        return Ok(false);
    }
    let Some(policy) = settings(conn)? else {
        return Ok(false);
    };
    let route = if candidate.im_reply.is_some() {
        "im"
    } else {
        match policy.settings.notice_route {
            super::super::policy::AssistantNoticeRoute::Desktop => "desktop",
            super::super::policy::AssistantNoticeRoute::Im => "im",
            _ => return Ok(false),
        }
    };
    if !route_allowed(conn, &candidate, route, now)? {
        return Ok(false);
    }
    Ok(source_keys(candidate.source_set.as_deref())?
        .iter()
        .any(|key| key.source_id == source && key.revision == revision))
}

fn proactive_ready(conn: &Connection, candidate: &Candidate, now: i64) -> Result<bool> {
    if candidate.ready > now {
        return Ok(false);
    }
    if candidate.im_reply.is_some() {
        return Ok(true);
    }
    Ok(!conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_attention_presence WHERE expires_at>?1)",
        [now],
        |row| row.get::<_, bool>(0),
    )?)
}
fn notification_policy(
    conn: &Connection,
) -> Result<Option<crate::personal_assistant_im::NotificationPolicy>> {
    Ok(
        settings(conn)?.map(|config| crate::personal_assistant_im::NotificationPolicy {
            timezone: config.settings.timezone,
            quiet_start_hour: config.settings.quiet_start_hour,
            quiet_end_hour: config.settings.quiet_end_hour,
            daily_limit: 2,
        }),
    )
}
pub(super) fn preflight_keys(conn: &Connection, route: &str, now: i64) -> Result<Vec<String>> {
    let Some(policy) = notification_policy(conn)? else {
        return Ok(Vec::new());
    };
    let allow_proactive = quota::deferred_reason(conn, &policy, now)?.is_none();
    if route == "desktop" && !allow_proactive {
        return Ok(Vec::new());
    }
    let mut statement=conn.prepare(&format!("SELECT {COLUMNS} FROM assistant_attention_candidates WHERE ready_at<=?1 AND expires_at>?1 AND (?3 OR im_reply_id IS NOT NULL)
        AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items sent WHERE sent.logical_key=assistant_attention_candidates.logical_key)
        ORDER BY CASE WHEN im_reply_id IS NOT NULL THEN 0 ELSE 1 END,ready_at,logical_key LIMIT 200 OFFSET ?2"))?;
    let mut selected = Vec::new();
    let mut offset = 0i64;
    loop {
        let candidates = statement
            .query_map(params![now, offset, allow_proactive], row_candidate)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if candidates.is_empty() {
            break;
        }
        offset += i64::try_from(candidates.len())?;
        for candidate in candidates {
            if !route_allowed(conn, &candidate, route, now)?
                || !proactive_ready(conn, &candidate, now)?
            {
                continue;
            }
            let interactive = candidate.im_reply.is_some();
            if !selected.is_empty() && interactive {
                continue;
            }
            selected.push(candidate.key);
            if interactive || selected.len() >= 4 {
                return Ok(selected);
            }
        }
    }
    Ok(selected)
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .filter(|value| !matches!(*value,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn source_summary(finding: &crate::personal_assistant::research::ResearchFindingInput) -> String {
    let mut text = one_line(&finding.fact);
    if !finding.prepared.trim().is_empty() {
        text.push_str("；已准备：");
        text.push_str(&one_line(&finding.prepared));
    }
    if !finding.uncertainty.trim().is_empty() {
        text.push_str("；待确认：");
        text.push_str(&one_line(&finding.uncertainty));
    }
    text
}
fn render(conn: &Connection, candidate: &Candidate) -> Result<String> {
    match candidate.kind.as_str() {
        "finding" => {
            let body:String=conn.query_row("SELECT body_json FROM assistant_findings WHERE id=?1 AND revision=?2 AND run_id=?3 AND state='active'",params![candidate.subject,candidate.revision,candidate.source_run],|row|row.get(0))?;
            Ok(source_summary(&serde_json::from_str(&body)?))
        }
        "im_query" => {
            let mut statement=conn.prepare("SELECT body_json FROM assistant_findings WHERE run_id=?1 AND state='active' ORDER BY id LIMIT 8")?;
            let rows = statement
                .query_map([candidate.source_run.as_deref()], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let summaries = rows
                .into_iter()
                .map(|body| Ok(source_summary(&serde_json::from_str(&body)?)))
                .collect::<Result<Vec<_>>>()?;
            if summaries.is_empty() {
                return Ok("这次检查没有新的可行动发现；你可以继续回复追问。".into());
            }
            Ok(summaries.join("\n"))
        }
        "reply_status" => Ok(
            "请用“编号: 你的问题”选择原消息中的一项，例如“2: 请继续检查”。审批请回到 Hope 处理。"
                .into(),
        ),
        "query_status" => {
            Ok("这次资料检查未能完成，资料或授权可能已变化。请在 Hope 核对后重新发起。".into())
        }
        "home_approval" => Ok("有一项操作需要你批准或拒绝。请回到 Hope 的审批界面处理。".into()),
        "home_question" => Ok("Hope 有一个问题等你回答。请回到助手对话查看和回答。".into()),
        "approval" => Ok(
            "有一项工作需要你批准或拒绝。请在 Hope 的审批界面处理；回复此消息不会批准工具。".into(),
        ),
        "question" => Ok(
            "有一项工作等你回答。请在 Hope 查看问题；也可在明确知道所问内容时回复此条消息。".into(),
        ),
        "item" => {
            let kind: String = conn.query_row(
                "SELECT kind FROM assistant_updates WHERE id=?1",
                [&candidate.subject],
                |row| row.get(0),
            )?;
            Ok(if kind == "result" {
                "一项交办已核实交付，请在 Hope 查看结果。"
            } else {
                "一项交办需要你查看或决定是否继续，请在 Hope 处理。"
            }
            .into())
        }
        _ => bail!("Unknown attention update"),
    }
}
pub(super) struct Claimed {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) body: String,
}
pub(super) fn claim(
    conn: &Connection,
    keys: &[String],
    route: &str,
    client: Option<&str>,
    claim_key: &str,
    now: i64,
) -> Result<Option<Claimed>> {
    if keys.is_empty() {
        return Ok(None);
    }
    let duplicate: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_attention_deliveries WHERE claim_key=?1)",
        [claim_key],
        |row| row.get(0),
    )?;
    if duplicate {
        return Ok(None);
    }
    let ready = preflight_keys(conn, route, now)?;
    let mut candidates = Vec::new();
    for key in keys.iter().take(4) {
        if !ready.contains(key) {
            continue;
        }
        if let Some(candidate) = candidate(conn, key)? {
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    let interactive = candidates
        .iter()
        .any(|candidate| candidate.im_reply.is_some());
    if interactive && candidates.len() != 1 {
        bail!("A requested IM answer has one receipt");
    }
    let paired = if route == "im" {
        Some(disclosure::pairing(conn)?.ok_or_else(|| anyhow!("Assistant IM pairing changed"))?)
    } else {
        None
    };
    let id = uuid::Uuid::new_v4().to_string();
    let title: String = conn.query_row(
        "SELECT COALESCE((SELECT name FROM personal_assistant_profile WHERE singleton=1),'Hope')",
        [],
        |row| row.get(0),
    )?;
    let mut parts = Vec::new();
    let item_bytes = 3000 / candidates.len();
    for (index, candidate) in candidates.iter().enumerate() {
        let part = format!(
            "{}. {}",
            index + 1,
            crate::truncate_utf8(&render(conn, candidate)?, item_bytes.saturating_sub(8))
        );
        parts.push(part);
    }
    let mut body = parts.join("\n\n");
    if route == "im"
        && candidates
            .iter()
            .any(|candidate| matches!(candidate.kind.as_str(), "finding" | "im_query" | "question"))
    {
        body.push_str(if candidates.len() == 1 {
            "\n\n可回复本条消息继续；完整依据和方案留在 Hope。"
        } else {
            "\n\n回复“编号: 你的问题”继续对应事项，例如“2: 再准备一个方案”。完整内容留在 Hope。"
        });
    }
    let expires = candidates
        .iter()
        .map(|candidate| candidate.expires)
        .min()
        .unwrap_or(now);
    conn.execute("INSERT INTO assistant_attention_deliveries(id,route,claim_key,client_id,state,created_at,submitted_at,connection_id,generation,chat_id,expires_at,instance_id,policy_revision)
        VALUES(?1,?2,?3,?4,'submitting',?5,?5,?6,?7,?8,?9,?10,?11)",params![id,route,claim_key,client,now,paired.as_ref().map(|state|&state.view.connection_id),paired.as_ref().map(|state|state.view.generation),paired.as_ref().and_then(|state|state.view.chat_id),expires,instance_id(),settings(conn)?.ok_or_else(||anyhow!("Assistant policy changed"))?.revision])?;
    for (index, candidate) in candidates.iter().enumerate() {
        let (disclosures, grant_revision) = if let Some(paired) = paired.as_ref() {
            if matches!(candidate.kind.as_str(), "finding" | "im_query") {
                let captured = disclosure::capture(
                    conn,
                    paired,
                    &source_keys(candidate.source_set.as_deref())?,
                    now,
                )?
                .ok_or_else(|| anyhow!("Source disclosure changed"))?;
                (Some(serde_json::to_string(&captured)?), None)
            } else if let Some((_, target, _)) = item_for_candidate(conn, candidate)? {
                (
                    None,
                    crate::personal_assistant_im::store::grant(
                        conn,
                        &paired.view.connection_id,
                        &target,
                    )?
                    .map(|grant| grant.revision),
                )
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };
        conn.execute("INSERT INTO assistant_attention_delivery_items(delivery_id,ordinal,logical_key,kind,subject_id,subject_revision,source_run_id,source_set_json,disclosure_set_json,im_grant_revision,home_session_id,global_stop_epoch,home_stop_epoch,expires_at,candidate_created_at,im_reply_id)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![id,i64::try_from(index+1)?,candidate.key,candidate.kind,candidate.subject,candidate.revision,candidate.source_run,candidate.source_set,disclosures,grant_revision,candidate.home,candidate.global_epoch,candidate.home_epoch,candidate.expires,candidate.created,candidate.im_reply])?;
        if route == "im" && matches!(candidate.kind.as_str(), "finding" | "im_query") {
            conn.execute("INSERT INTO assistant_attention_payloads(delivery_id,ordinal,body,expires_at) VALUES(?1,?2,?3,?4)",params![id,i64::try_from(index+1)?,parts[index],expires])?;
        }
    }
    if !interactive {
        // Same ledger used by legacy completion notices. Every route shares
        // these immutable UTC attempts, including unknown/proven-unsent cases.
        conn.execute("INSERT INTO assistant_im_notification_attempts(delivery_id,attempted_at) VALUES(?1,?2)",params![id,now])?;
    }
    Ok(Some(Claimed { id, title, body }))
}

impl SessionDB {
    pub async fn claim_assistant_desktop_notice(
        self: std::sync::Arc<Self>,
        client: String,
        claim_key: String,
    ) -> Result<Option<AssistantDesktopNotice>> {
        validate_client(&client)?;
        validate_client(&claim_key)?;
        let keys = self
            .run(|db| {
                let conn = db.read_conn()?;
                preflight_keys(&conn, "desktop", chrono::Utc::now().timestamp())
            })
            .await?;
        for key in &keys {
            super::super::research::refresh_attention_sources(self.clone(), key.clone()).await?;
        }
        self.run(move |db| {
            let mut conn = db
                .conn
                .lock()
                .map_err(|_| anyhow!("Assistant attention unavailable"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let claimed = claim(
                &tx,
                &keys,
                "desktop",
                Some(&client),
                &claim_key,
                chrono::Utc::now().timestamp(),
            )?;
            tx.commit()?;
            drop(conn);
            if claimed.is_some() {
                changed();
            }
            Ok(claimed.map(|notice| AssistantDesktopNotice {
                id: notice.id,
                title: notice.title,
                body: notice.body,
            }))
        })
        .await
    }
}

pub struct AssistantAttentionImSend {
    pub id: String,
    pub lease: crate::personal_assistant_im::PollLease,
    pub chat_id: i64,
    pub text: String,
}
impl SessionDB {
    pub async fn prepare_assistant_attention_im(
        self: std::sync::Arc<Self>,
    ) -> Result<Option<AssistantAttentionImSend>> {
        if !crate::runtime_lock::is_primary() {
            return Ok(None);
        }
        let keys = self
            .run(|db| {
                let conn = db.read_conn()?;
                preflight_keys(&conn, "im", chrono::Utc::now().timestamp())
            })
            .await?;
        for key in &keys {
            super::super::research::refresh_attention_sources(self.clone(), key.clone()).await?;
        }
        self.run(move |db| {
            let mut conn = db
                .conn
                .lock()
                .map_err(|_| anyhow!("Assistant attention unavailable"))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let claim_key = uuid::Uuid::new_v4().to_string();
            let claimed = claim(
                &tx,
                &keys,
                "im",
                None,
                &claim_key,
                chrono::Utc::now().timestamp(),
            )?;
            let paired = disclosure::pairing(&tx)?;
            let result = if let (Some(claimed), Some(paired)) = (claimed, paired) {
                Some(AssistantAttentionImSend {
                    id: claimed.id,
                    lease: crate::personal_assistant_im::store::lease(&paired),
                    chat_id: paired
                        .view
                        .chat_id
                        .ok_or_else(|| anyhow!("Assistant IM destination changed"))?,
                    text: claimed.body,
                })
            } else {
                None
            };
            tx.commit()?;
            drop(conn);
            if result.is_some() {
                changed();
            }
            Ok(result)
        })
        .await
    }
    pub fn finish_assistant_attention_im(
        &self,
        id: &str,
        message_id: Option<i64>,
        now: i64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant receipt unavailable"))?;
        let accepted = message_id.filter(|id| *id > 0);
        conn.execute("UPDATE assistant_attention_deliveries SET state=?2,message_id=?3,settled_at=?4 WHERE id=?1 AND route='im' AND state IN('submitting','unknown')",
            params![id,if accepted.is_some(){"accepted"}else{"unknown"},accepted,now])?;
        drop(conn);
        changed();
        Ok(())
    }
    pub fn cancel_unsubmitted_assistant_attention_im(&self, id: &str, now: i64) -> Result<()> {
        self.conn.lock().map_err(|_|anyhow!("Assistant receipt unavailable"))?.execute("UPDATE assistant_attention_deliveries SET state='cancelled',settled_at=?2 WHERE id=?1 AND route='im' AND state='submitting'",params![id,now])?;
        changed();
        Ok(())
    }
    pub fn assistant_attention_status(&self, now: i64) -> Result<AssistantAttentionStatus> {
        let conn = self.read_conn()?;
        let applied = super::super::policy::current(&conn)?;
        let paired = disclosure::pairing(&conn)?;
        let mut next = None;
        let reason = if let Some(config) = applied {
            if config.settings.paused {
                Some("paused".into())
            } else if config.settings.notice_route
                == super::super::policy::AssistantNoticeRoute::None
            {
                Some("disabled".into())
            } else if config.settings.notice_route == super::super::policy::AssistantNoticeRoute::Im
                && paired
                    .as_ref()
                    .is_none_or(|state| state.view.notifications_paused)
            {
                Some("pairing_required".into())
            } else if conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_attention_presence WHERE expires_at>?1)",
                [now],
                |row| row.get::<_, bool>(0),
            )? {
                Some("using_hope".into())
            } else {
                let policy = notification_policy(&conn)?
                    .ok_or_else(|| anyhow!("Assistant policy changed"))?;
                let reason = quota::deferred_reason(&conn, &policy, now)?.map(|value| {
                    match value {
                        crate::personal_assistant_im::DeliveryDeferredReason::TimezoneRequired => {
                            "timezone_required"
                        }
                        crate::personal_assistant_im::DeliveryDeferredReason::QuietHours => {
                            "quiet_hours"
                        }
                        crate::personal_assistant_im::DeliveryDeferredReason::DailyLimit => {
                            "daily_limit"
                        }
                    }
                    .to_owned()
                });
                if reason.is_some() {
                    next = quota::next_eligible_at(&conn, &policy, now)?;
                }
                reason
            }
        } else {
            Some("policy_pending".into())
        };
        let mut statement=conn.prepare("SELECT d.id,d.route,d.state,d.submitted_at,(SELECT COUNT(*) FROM assistant_attention_delivery_items item WHERE item.delivery_id=d.id)
            FROM assistant_attention_deliveries d ORDER BY d.submitted_at DESC,d.id LIMIT 20")?;
        let receipts = statement
            .query_map([], |row| {
                Ok(AttentionReceipt {
                    id: row.get(0)?,
                    route: row.get(1)?,
                    state: row.get(2)?,
                    submitted_at: row.get(3)?,
                    item_count: row.get::<_, u32>(4)? as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let pending:i64=conn.query_row("SELECT COUNT(*) FROM assistant_attention_candidates WHERE expires_at>?1 AND NOT EXISTS(SELECT 1 FROM assistant_attention_delivery_items item WHERE item.logical_key=assistant_attention_candidates.logical_key)",[now],|row|row.get(0))?;
        Ok(AssistantAttentionStatus {
            paired: paired.is_some(),
            pairing: paired.as_ref().map(|state| AssistantAttentionPairing {
                connection_id: state.view.connection_id.clone(),
                generation: state.view.generation,
            }),
            deferred_reason: reason,
            next_eligible_at: next,
            pending_count: usize::try_from(pending)?,
            receipts,
        })
    }
}

impl SessionDB {
    pub fn assistant_attention_im_send_current(&self, id: &str, now: i64) -> Result<bool> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Assistant send authority unavailable"))?;
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let conn = &tx;
        let Some(paired) = disclosure::pairing(&conn)? else {
            return Ok(false);
        };
        let Some(policy) = settings(&conn)? else {
            return Ok(false);
        };
        let live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM assistant_attention_deliveries WHERE id=?1 AND route='im' AND state='submitting' AND connection_id=?2 AND generation=?3 AND expires_at>?4 AND policy_revision=?5)",params![id,paired.view.connection_id,paired.view.generation,now,policy.revision],|row|row.get(0))?;
        if !live {
            return Ok(false);
        }
        let mut statement=conn.prepare("SELECT logical_key,kind,subject_id,subject_revision,source_run_id,source_set_json,home_session_id,global_stop_epoch,home_stop_epoch,candidate_created_at,candidate_created_at,expires_at,im_reply_id,disclosure_set_json,im_grant_revision FROM assistant_attention_delivery_items WHERE delivery_id=?1 ORDER BY ordinal")?;
        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row_candidate(row)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            return Ok(false);
        }
        for (candidate, captured, grant_revision) in rows {
            if !route_allowed(&conn, &candidate, "im", now)?
                || !proactive_ready(&conn, &candidate, now)?
            {
                return Ok(false);
            }
            if matches!(candidate.kind.as_str(), "finding" | "im_query") {
                let Some(captured) = captured else {
                    return Ok(false);
                };
                if !disclosure::captured_current(
                    &conn,
                    &paired,
                    &source_keys(candidate.source_set.as_deref())?,
                    &captured,
                    now,
                )? {
                    return Ok(false);
                }
            } else if let Some((_, target, _)) = item_for_candidate(&conn, &candidate)? {
                if crate::personal_assistant_im::store::grant(
                    &conn,
                    &paired.view.connection_id,
                    &target,
                )?
                .map(|grant| grant.revision)
                    != grant_revision
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
