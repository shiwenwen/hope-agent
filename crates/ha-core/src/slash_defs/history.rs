//! Slash 命令转录落库 + 命令显示文本——写 `messages` 表的 event 行，
//! 只依赖 kernel 的 session / chat_engine。IM 渠道（未来 ha-channel）
//! 直接调 [`append_slash_history_events`]，不必经装配层。

/// Persist a control slash command to visible transcript without feeding it
/// to the LLM. The command row is styled like a user bubble by the frontend;
/// the result row remains a normal event chip/card.
pub fn append_slash_history_events(
    session_db: &crate::session::SessionDB,
    session_id: &str,
    command_text: &str,
    result_content: Option<&str>,
    source: crate::chat_engine::ChatSource,
) -> anyhow::Result<Vec<i64>> {
    let command_text = command_text.trim();
    if command_text.is_empty() {
        return Ok(Vec::new());
    }

    let mut ids = Vec::with_capacity(2);
    let command_display = slash_history_command_display(command_text);

    let mut command_msg =
        crate::session::NewMessage::event(&command_display.content).with_source(source);
    command_msg.attachments_meta = Some(
        serde_json::json!({
            "slash_command": {
                "kind": "command",
                "command": command_text,
                "displayAs": "user",
                "mode": command_display.mode,
            }
        })
        .to_string(),
    );
    ids.push(session_db.append_message(session_id, &command_msg)?);
    let _ = crate::session::ensure_first_message_title(
        session_db,
        session_id,
        &command_display.content,
        None,
    );

    if let Some(result_content) = result_content.filter(|s| !s.trim().is_empty()) {
        let mut result_msg = crate::session::NewMessage::event(result_content).with_source(source);
        result_msg.attachments_meta = Some(
            serde_json::json!({
                "slash_command": {
                    "kind": "result",
                    "command": command_text,
                }
            })
            .to_string(),
        );
        ids.push(session_db.append_message(session_id, &result_msg)?);
    }

    Ok(ids)
}

struct SlashHistoryCommandDisplay {
    content: String,
    mode: Option<&'static str>,
}

fn slash_history_command_display(command_text: &str) -> SlashHistoryCommandDisplay {
    let trimmed = command_text.trim();
    if let Some(args) = slash_command_args(trimmed, "goal") {
        let content = goal_command_display_text(args)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Goal".to_string());
        return SlashHistoryCommandDisplay {
            content,
            mode: Some("goal"),
        };
    }
    if let Some(args) = slash_command_args(trimmed, "loop") {
        let content = loop_command_display_text(args)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Loop".to_string());
        return SlashHistoryCommandDisplay {
            content,
            mode: Some("loop"),
        };
    }
    SlashHistoryCommandDisplay {
        content: trimmed.to_string(),
        mode: None,
    }
}

fn slash_command_args<'a>(trimmed: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("/{name}");
    let raw_rest = trimmed.strip_prefix(&prefix)?;
    if !raw_rest.is_empty() && !raw_rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(raw_rest.trim())
}

fn goal_command_display_text(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Some("Show active goal".to_string());
    }
    match trimmed {
        "status" | "show" => Some("Show active goal".to_string()),
        "pause" => Some("Pause active goal".to_string()),
        "resume" => Some("Resume active goal".to_string()),
        "clear" | "cancel" => Some("Clear active goal".to_string()),
        "evaluate" | "audit" => Some("Evaluate active goal".to_string()),
        "help" => Some("Goal help".to_string()),
        _ => Some(trimmed.to_string()),
    }
}

fn loop_command_display_text(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Some("Start self-paced loop".to_string());
    }
    let first = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match first.as_str() {
        "status" | "list" | "show" => Some("Show loops".to_string()),
        "pause" => Some("Pause loop".to_string()),
        "resume" => Some("Resume loop".to_string()),
        "stop" | "cancel" => Some("Stop loop".to_string()),
        "help" => Some("Loop help".to_string()),
        _ => Some(trimmed.to_string()),
    }
}

pub(crate) fn is_loop_create_slash_command(command_text: &str) -> bool {
    let Some(args) = slash_command_args(command_text.trim(), "loop") else {
        return false;
    };
    if args.trim().is_empty() {
        return false;
    }
    let first = args
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(
        first.as_str(),
        "status" | "list" | "show" | "help" | "pause" | "resume" | "stop" | "cancel"
    )
}

#[cfg(test)]
mod tests {
    use super::goal_command_display_text;

    #[test]
    fn goal_slash_history_does_not_strip_objective_prefix_words() {
        assert_eq!(
            goal_command_display_text("pause react upgrade"),
            Some("pause react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("update react upgrade"),
            Some("update react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("set react upgrade"),
            Some("set react upgrade".to_string())
        );
        assert_eq!(
            goal_command_display_text("pause"),
            Some("Pause active goal".to_string())
        );
    }
}
