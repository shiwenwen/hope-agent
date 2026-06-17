//! Sprite instruction builder — fuses the persona with the enabled senses
//! (current document, recent edit, conversation, recalled memories, cross-session
//! awareness) into a single bounded `side_query` instruction. Pure + testable:
//! callers gather the raw pieces, this only formats + budgets them.

use super::config::SpriteConfig;
use super::types::SpriteObserveParams;
use crate::truncate_utf8;

// Per-segment character budgets (defensive — each input is also pre-trimmed).
const DOC_BUDGET: usize = 3000;
const EDIT_BUDGET: usize = 600;
const CONV_MSG_BUDGET: usize = 360;
const CONV_TOTAL: usize = 6;
const MEMORY_BUDGET: usize = 1200;
const AWARENESS_BUDGET: usize = 800;

/// Built-in personas (English, not free-text configurable; the user picks the
/// proactive vs. restrained variant via `SpriteConfig.proactive`).
const PERSONA_RESTRAINED: &str = "You are a thoughtful writing sprite — warm, perceptive, and restrained. \
You quietly accompany the user as they write in their knowledge space, and only when the moment \
is genuinely right do you offer a single line worth saying: a suggestion for what to write next, \
honest feedback on what was just written, a connection to an old note or memory, a timely reminder, \
or simply a word of encouragement. Never verbose. Stay silent unless you truly have something \
worth saying.";

/// More forthcoming voice (default). Still a single line, still never verbose,
/// but biased toward offering something helpful rather than staying silent.
const PERSONA_PROACTIVE: &str = "You are a lively, warm writing sprite who actively accompanies the \
user as they write in their knowledge space. Lean toward offering one genuinely helpful line whenever \
you reasonably can: a suggestion for what to write next, honest feedback on what was just written, a \
connection to an old note or memory, a timely reminder, or a word of encouragement. Always exactly one \
line, never verbose. Only stay silent when there's truly nothing useful to add.";

/// Build the full sprite instruction. `conversation` is (role, text) newest-last;
/// `memory_lines` / `awareness_lines` are already-rendered single lines.
pub fn build_instruction(
    cfg: &SpriteConfig,
    params: &SpriteObserveParams,
    conversation: &[(String, String)],
    memory_lines: &[String],
    awareness_lines: &[String],
) -> String {
    let mut s = String::new();
    s.push_str(if cfg.proactive {
        PERSONA_PROACTIVE
    } else {
        PERSONA_RESTRAINED
    });
    s.push_str("\n\n");
    s.push_str(if cfg.proactive {
        "Below is what the user is currently editing in their knowledge space. Offer one line worth \
         saying right now in the JSON format below whenever you reasonably can; only return \
         {\"category\":\"none\"} if there's genuinely nothing useful to add.\n"
    } else {
        "Below is what the user is currently editing in their knowledge space. Decide: is there one \
         line genuinely worth saying right now? If so, return exactly one suggestion in the JSON \
         format below. If not (which should be most of the time), return {\"category\":\"none\"}.\n"
    });

    if cfg.senses.doc && !params.doc_content.trim().is_empty() {
        s.push_str("\n## Current document\n");
        s.push_str(truncate_utf8(params.doc_content.trim(), DOC_BUDGET));
        s.push('\n');
    }

    if cfg.senses.edit {
        if let Some(edit) = params.recent_edit.as_deref() {
            let edit = edit.trim();
            if !edit.is_empty() {
                s.push_str("\n## What just changed (recent edit)\n");
                s.push_str(truncate_utf8(edit, EDIT_BUDGET));
                s.push('\n');
            }
        }
    }

    if cfg.senses.conversation && !conversation.is_empty() {
        s.push_str("\n## Recent conversation\n");
        for (role, text) in conversation.iter().rev().take(CONV_TOTAL).rev() {
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            s.push_str(&format!(
                "- {}: {}\n",
                role,
                truncate_utf8(text, CONV_MSG_BUDGET)
            ));
        }
    }

    if cfg.senses.memory && !memory_lines.is_empty() {
        s.push_str("\n## What you remember about the user (memory recall)\n");
        let joined = memory_lines.join("\n");
        s.push_str(truncate_utf8(&joined, MEMORY_BUDGET));
        s.push('\n');
    }

    if cfg.senses.awareness && !awareness_lines.is_empty() {
        s.push_str("\n## What the user has been doing elsewhere (cross-session awareness)\n");
        let joined = awareness_lines.join("\n");
        s.push_str(truncate_utf8(&joined, AWARENESS_BUDGET));
        s.push('\n');
    }

    s.push_str(
        "\n---\nOutput exactly one JSON object, no extra text and no code fences:\n\
         {\"category\": \"writing|review|encourage|remind|connect\", \"text\": \"≤2 sentences, in the SAME language as the document, natural and friendly\"}\n\
         If you have nothing worth saying, output {\"category\":\"none\"}.\n\
         category meanings: writing = a suggestion for what to write next; review = honest feedback on what was just written; \
         encourage = encouragement / emotional support; remind = a timely reminder; connect = a link to an old note or memory.",
    );

    s
}
