//! LLM-based memory selection for relevance filtering.
//!
//! When the number of candidate memories exceeds a threshold, uses
//! side_query() to select only the most relevant ones for the current
//! user message, reducing system prompt noise from irrelevant entries.
//!
//! Reference: claude-code `findRelevantMemories.ts` + `SELECT_MEMORIES_SYSTEM_PROMPT`.

const SELECTION_PROMPT: &str = r#"You are a memory relevance filter.
Given the user's current message and a list of stored memories, select
the most relevant memories that would help the AI assistant respond effectively.

Be selective — only include memories that are clearly relevant to the current context.
Include gotchas, warnings, and user preferences that apply to the task at hand.
Discard memories about topics unrelated to the user's current message.

User's current message:
{MESSAGE}

Candidate memories (id: content preview):
{CANDIDATES}

Return ONLY a JSON array of selected memory IDs (integers), most relevant first.
Select at most {MAX} memories. If none are relevant, return [].
Example: [3, 7, 1]"#;

/// Build the LLM selection instruction from user message and candidate memories.
pub(crate) fn build_selection_instruction(
    user_message: &str,
    candidates: &[(i64, String)],
    max_selected: usize,
) -> String {
    let candidate_lines: String = candidates
        .iter()
        .map(|(id, preview)| format!("{}: {}", id, preview))
        .collect::<Vec<_>>()
        .join("\n");

    SELECTION_PROMPT
        .replace("{MESSAGE}", user_message)
        .replace("{CANDIDATES}", &candidate_lines)
        .replace("{MAX}", &max_selected.to_string())
}

/// Parse the LLM response into a list of selected memory IDs.
/// Handles JSON arrays, markdown fences, and extra text around the array.
pub(crate) fn parse_selection_response(response: &str) -> Vec<i64> {
    let text = response.trim();

    // Try to extract JSON array from the response
    let json_str = if let Some(start) = text.find('[') {
        if let Some(end) = text[start..].find(']') {
            &text[start..start + end + 1]
        } else {
            return Vec::new();
        }
    } else {
        return Vec::new();
    };

    // Parse as JSON array of numbers
    match serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
        Ok(arr) => arr
            .iter()
            .filter_map(|v| v.as_i64())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Replace the `# Memory` section in a system prompt with new content.
/// If no memory section exists, does nothing.
pub(crate) fn replace_memory_section(system_prompt: &mut String, new_memory_content: &str) {
    const MEMORY_HEADER: &str = "\n# Memory\n";

    let start = match system_prompt.find(MEMORY_HEADER) {
        Some(pos) => pos,
        None => return,
    };

    // Find the end of the memory section: next top-level heading or end of string.
    // Skip the header itself, then look for the next "\n# " or "\n## " that starts
    // a new top-level section (not a sub-heading within memory).
    let content_start = start + MEMORY_HEADER.len();
    let end = system_prompt[content_start..]
        .find("\n# ")
        .map(|pos| content_start + pos)
        .unwrap_or(system_prompt.len());

    // Build replacement
    let replacement = if new_memory_content.is_empty() {
        String::new()
    } else {
        format!("{}{}", MEMORY_HEADER, new_memory_content)
    };

    system_prompt.replace_range(start..end, &replacement);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_selection_response_clean() {
        assert_eq!(parse_selection_response("[3, 7, 1]"), vec![3, 7, 1]);
    }

    #[test]
    fn test_parse_selection_response_with_markdown() {
        assert_eq!(
            parse_selection_response("```json\n[5, 2]\n```"),
            vec![5, 2]
        );
    }

    #[test]
    fn test_parse_selection_response_with_text() {
        assert_eq!(
            parse_selection_response("Here are the selected memories: [1, 4, 9]"),
            vec![1, 4, 9]
        );
    }

    #[test]
    fn test_parse_selection_response_empty() {
        assert_eq!(parse_selection_response("[]"), Vec::<i64>::new());
        assert_eq!(parse_selection_response("no array here"), Vec::<i64>::new());
    }

    #[test]
    fn test_replace_memory_section() {
        let mut prompt = "# Identity\nI am an AI.\n\n# Memory\n\n## About the User\n- likes Rust\n\n## Preferences\n- concise\n\n# Runtime\ndate: 2024".to_string();
        replace_memory_section(&mut prompt, "\n## Selected\n- relevant memory\n");
        assert!(prompt.contains("# Memory\n\n## Selected\n- relevant memory\n"));
        assert!(prompt.contains("# Runtime\ndate: 2024"));
        assert!(!prompt.contains("likes Rust"));
    }

    #[test]
    fn test_replace_memory_section_no_header() {
        let mut prompt = "# Identity\nI am an AI.".to_string();
        let original = prompt.clone();
        replace_memory_section(&mut prompt, "new content");
        assert_eq!(prompt, original); // unchanged
    }

    #[test]
    fn test_build_selection_instruction() {
        let candidates = vec![
            (1, "User prefers Rust".to_string()),
            (2, "Project deadline is Friday".to_string()),
        ];
        let result = build_selection_instruction("Help me refactor", &candidates, 5);
        assert!(result.contains("Help me refactor"));
        assert!(result.contains("1: User prefers Rust"));
        assert!(result.contains("2: Project deadline is Friday"));
        assert!(result.contains("at most 5"));
    }
}
