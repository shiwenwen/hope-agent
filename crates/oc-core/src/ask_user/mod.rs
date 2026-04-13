//! Independent ask-user question module.
//!
//! Provides a general-purpose structured Q&A tool that allows the LLM to send
//! interactive questions to the user in any conversation (not only Plan Mode).

mod questions;
mod types;

// ── Re-exports ──────────────────────────────────────────────────

pub use types::{
    AskUserQuestion, AskUserQuestionAnswer, AskUserQuestionGroup, AskUserQuestionOption,
};

pub use questions::{
    cancel_pending_ask_user_question, find_live_pending_group_for_session,
    is_ask_user_question_live, mark_group_answered, persist_pending_group,
    register_ask_user_question, submit_ask_user_question_response, EVENT_ASK_USER_REQUEST,
};
