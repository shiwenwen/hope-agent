use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{emit_inbox_changed, execution_item, validate_item, AssistantError};
use crate::ask_user::{AskUserQuestionAnswer, AskUserQuestionGroup};
use crate::session::SessionDB;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAnswerTarget {
    pub item_id: String,
    pub item_revision: i64,
    pub execution_id: String,
}

impl SessionDB {
    pub fn create_assistant_question(
        &self,
        execution: &str,
        text: &str,
        options: Vec<crate::ask_user::AskUserQuestionOption>,
    ) -> Result<AskUserQuestionGroup> {
        if text.trim().is_empty()
            || text.len() > 8_192
            || options.len() > 6
            || options.iter().any(|o| {
                o.value.trim().is_empty()
                    || o.value.len() > 128
                    || o.label.fallback_text().trim().is_empty()
                    || o.label.fallback_text().len() > 512
            })
            || options
                .iter()
                .enumerate()
                .any(|(n, o)| options[..n].iter().any(|p| p.value == o.value))
        {
            bail!(AssistantError::InvalidInput)
        }
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (item, _, revision) =
            execution_item(&tx, execution)?.ok_or(AssistantError::Unavailable)?;
        validate_item(&tx, &item)?;
        let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM chat_stream_runs s
            JOIN assistant_execution_contexts c ON c.execution_id=s.run_id
            WHERE s.run_id=?1 AND s.status='running' AND c.is_subtask=0 AND c.closed_for_question=0)",
            [execution],|r|r.get(0))?;
        if !live {
            bail!(AssistantError::Unavailable)
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_item_questions
            WHERE execution_id=?1)",
            [execution],
            |r| r.get(0),
        )?;
        if exists {
            bail!(AssistantError::Conflict)
        }
        let home: String = tx.query_row(
            "SELECT r.home_session_id FROM assistant_items i
            JOIN personal_assistant_requests r ON r.id=i.request_id WHERE i.id=?1",
            [&item],
            |r| r.get(0),
        )?;
        let request_id = format!("auq_{}", uuid::Uuid::new_v4().simple());
        let target = AssistantAnswerTarget {
            item_id: item.clone(),
            item_revision: revision,
            execution_id: execution.to_owned(),
        };
        let group: AskUserQuestionGroup = serde_json::from_value(serde_json::json!({
            "requestId":request_id,"sessionId":home,
            "questions":[{"questionId":"decision","text":text,"options":options,
                "allowCustom":true,"multiSelect":false}],
            "source":"personal_assistant","serverNow":chrono::Utc::now().timestamp(),
            "ownerResponse":{"action":"assistant_item_answer","assistantItem":target}
        }))?;
        tx.execute(
            "INSERT INTO ask_user_questions(request_id,session_id,payload,status,created_at)
            VALUES(?1,?2,?3,'pending',?4)",
            params![
                request_id,
                home,
                serde_json::to_string(&group)?,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        tx.execute(
            "INSERT INTO assistant_item_questions(question_id,item_id,item_revision,execution_id)
            VALUES(?1,?2,?3,?4)",
            params![request_id, item, revision, execution],
        )?;
        tx.execute(
            "UPDATE assistant_execution_contexts SET closed_for_question=1 WHERE execution_id=?1",
            [execution],
        )?;
        tx.execute(
            "UPDATE assistant_items SET state='waiting',reason='needs_input',updated_at=?2
            WHERE id=?1 AND revision=?3",
            params![item, chrono::Utc::now().to_rfc3339(), revision],
        )?;
        tx.execute("INSERT INTO assistant_updates(id,item_id,event_key,kind,summary,source_revision,created_at)
            VALUES(?1,?2,?3,'question',?4,?5,?6)",params![uuid::Uuid::new_v4().to_string(),item,
                format!("question:{request_id}"),text,revision,chrono::Utc::now().to_rfc3339()])?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(group)
    }

    pub fn answer_assistant_question(
        &self,
        request_id: &str,
        target: &AssistantAnswerTarget,
        answers: &[AskUserQuestionAnswer],
    ) -> Result<()> {
        let mut conn = self.conn.lock().map_err(|e| anyhow!("Lock error: {e}"))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let payload: Option<String> = tx
            .query_row(
                "SELECT payload FROM ask_user_questions
            WHERE request_id=?1 AND status='pending' AND (timeout_at IS NULL OR timeout_at>?2)",
                params![request_id, chrono::Utc::now().timestamp()],
                |r| r.get(0),
            )
            .optional()?;
        let group: AskUserQuestionGroup =
            serde_json::from_str(&payload.ok_or(AssistantError::Unavailable)?)?;
        let stored = group
            .owner_response
            .as_ref()
            .and_then(|r| r.assistant_item.as_ref())
            .ok_or(AssistantError::Unavailable)?;
        if stored != target {
            bail!(AssistantError::Conflict)
        }
        let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM assistant_item_questions
            WHERE question_id=?1 AND item_id=?2 AND item_revision=?3 AND execution_id=?4 AND answer_json IS NULL)",
            params![request_id,target.item_id,target.item_revision,target.execution_id],|r|r.get(0))?;
        if !bound {
            bail!(AssistantError::Conflict)
        }
        validate_item(&tx, &target.item_id)?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM assistant_items WHERE id=?1",
            [&target.item_id],
            |r| r.get(0),
        )?;
        if revision != target.item_revision {
            bail!(AssistantError::Stopped)
        }
        if answers.len() != 1
            || answers[0].question_id != "decision"
            || !answers[0].files.is_empty()
        {
            bail!(AssistantError::InvalidInput)
        }
        let answer = &answers[0];
        let options = &group.questions[0].options;
        if answer.selected.len() > 1
            || answer
                .selected
                .iter()
                .any(|v| !options.iter().any(|o| o.value == *v))
            || (answer.selected.is_empty()
                && answer
                    .custom_input
                    .as_deref()
                    .is_none_or(|s| s.trim().is_empty()))
            || answer
                .custom_input
                .as_ref()
                .is_some_and(|s| s.len() > 16_384)
        {
            bail!(AssistantError::InvalidInput)
        }
        let answer_json = serde_json::to_string(answers)?;
        tx.execute("UPDATE assistant_item_questions SET answer_json=?2 WHERE question_id=?1 AND answer_json IS NULL",
            params![request_id,answer_json])?;
        let won = tx.execute(
            "UPDATE ask_user_questions SET status='answered',answered_at=?2
            WHERE request_id=?1 AND status='pending'",
            params![request_id, chrono::Utc::now().to_rfc3339()],
        )?;
        if won != 1 {
            bail!(AssistantError::Conflict)
        }
        let instruction=format!("The owner answered the pending question. Continue only within the existing request. Answer data: {answer_json}");
        tx.execute("INSERT INTO assistant_followups(id,item_id,item_revision,source_turn_id,instruction,due_at)
            VALUES(?1,?2,?3,?4,?5,?6)",params![format!("answer:{request_id}"),target.item_id,
                revision,target.execution_id,instruction,chrono::Utc::now().timestamp()])?;
        tx.execute(
            "UPDATE assistant_items SET state='open',reason=NULL,updated_at=?2 WHERE id=?1",
            params![target.item_id, chrono::Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        drop(conn);
        emit_inbox_changed();
        Ok(())
    }
}
