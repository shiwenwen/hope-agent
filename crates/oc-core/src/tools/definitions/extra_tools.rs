use serde_json::json;

use super::super::{TOOL_CANVAS, TOOL_SEND_NOTIFICATION, TOOL_WEB_SEARCH};
use super::types::ToolDefinition;

/// Returns the web_search tool definition (conditionally injected when enabled).
pub fn get_web_search_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_WEB_SEARCH.into(),
        description: "Search the web for information. Returns relevant results with titles, URLs, and snippets. Use this when the user asks about current events, recent information, or anything that requires up-to-date knowledge. Pass `run_in_background: true` for slow providers or large result sets so the conversation can continue while the search runs.".into(),
        internal: false,
        deferred: false,
        always_load: false,
        async_capable: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (1-10, default from settings)"
                },
                "country": {
                    "type": "string",
                    "description": "ISO 3166-1 alpha-2 country code (e.g. 'US', 'CN'). Limits results to this country. Supported by: Brave, Google, Tavily."
                },
                "language": {
                    "type": "string",
                    "description": "ISO 639-1 language code (e.g. 'en', 'zh'). Prefer results in this language. Supported by: Brave, SearXNG, Google."
                },
                "freshness": {
                    "type": "string",
                    "enum": ["day", "week", "month", "year"],
                    "description": "Time filter: only return results from the specified period. Supported by: Brave, SearXNG, Perplexity, Google, Tavily."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

/// Returns the notification tool definition (conditionally injected).
pub fn get_notification_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_SEND_NOTIFICATION.into(),
        description: "Send a native desktop notification to the user. Use this to proactively alert the user about important events, task completions, or findings that need their attention.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Notification title (short, descriptive)"
                },
                "body": {
                    "type": "string",
                    "description": "Notification body text with details"
                }
            },
            "required": ["body"],
            "additionalProperties": false
        }),
    }
}

/// Returns the canvas tool definition (conditionally injected when enabled).
pub fn get_canvas_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_CANVAS.into(),
        description: "Create and manage interactive canvas projects — HTML/CSS/JS live preview, documents (Markdown/code), data visualizations (Chart.js), diagrams (Mermaid), presentations (slides), and SVG graphics. Canvas content is rendered in a sandboxed preview panel visible to the user. Use snapshot to capture the current visual state for analysis.".into(),
        internal: true,
        deferred: false,
        always_load: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update", "show", "hide", "snapshot", "eval_js", "list", "delete", "versions", "restore", "export"],
                    "description": "Canvas operation to perform"
                },
                "project_id": {
                    "type": "string",
                    "description": "Canvas project ID (returned by create, required for most actions)"
                },
                "title": {
                    "type": "string",
                    "description": "Project title (for create/update)"
                },
                "content_type": {
                    "type": "string",
                    "enum": ["html", "markdown", "code", "svg", "mermaid", "chart", "slides"],
                    "description": "Content type (default: html). Determines rendering mode."
                },
                "html": {
                    "type": "string",
                    "description": "HTML content (for html/slides content_type)"
                },
                "css": {
                    "type": "string",
                    "description": "CSS styles"
                },
                "js": {
                    "type": "string",
                    "description": "JavaScript code (for html content_type or eval_js action)"
                },
                "content": {
                    "type": "string",
                    "description": "Text content (for markdown/code/svg/mermaid/chart content_type)"
                },
                "language": {
                    "type": "string",
                    "description": "Programming language (for code content_type, e.g. 'python', 'rust')"
                },
                "version_id": {
                    "type": "integer",
                    "description": "Version number (for restore action)"
                },
                "version_message": {
                    "type": "string",
                    "description": "Optional commit message for this version (for update)"
                },
                "format": {
                    "type": "string",
                    "enum": ["html", "markdown", "png"],
                    "description": "Export format (for export action)"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}
