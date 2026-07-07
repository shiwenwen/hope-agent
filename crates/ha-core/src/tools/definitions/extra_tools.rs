use serde_json::json;

use super::super::{TOOL_CANVAS, TOOL_DESIGN, TOOL_SEND_NOTIFICATION, TOOL_WEB_SEARCH};
use super::types::{ToolDefinition, ToolTier};

/// Returns the web_search tool definition (conditionally injected when enabled).
pub fn get_web_search_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_WEB_SEARCH.into(),
        description: "Search the web for information. Returns relevant results with titles, URLs, and snippets. Use this when the user asks about current events, recent information, or anything that requires up-to-date knowledge. Pass `run_in_background: true` for slow providers or large result sets so the conversation can continue while the search runs.".into(),
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Tools → Web Search",
        },
        internal: false,
        concurrent_safe: true,
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
                    "description": "Time filter: only return results from the specified period. Supported by: Bocha, Brave, SearXNG, Perplexity, Google, Tavily."
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
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Tools → Notifications",
        },
        internal: true,
        concurrent_safe: false,
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
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Tools → Canvas",
        },
        internal: true,
        concurrent_safe: false,
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

/// The `design` tool — Design Space: generate deliverable, self-contained design
/// artifacts (web/mobile pages, decks, dashboards, posters, documents, emails)
/// grounded in reusable brand design systems, previewed live in a stable panel.
pub fn get_design_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DESIGN.into(),
        description: "Create and iterate deliverable design artifacts in the Design Space. Produces self-contained HTML (web/mobile/deck/dashboard/poster/document/email) rendered live in a stable preview panel the user sees. Workflow: call action=list_recipes (optionally filter by kind) to see structure guidance, then action=create_artifact with kind + body_html/css/js. Reference design-system CSS variables (var(--ds-color-primary), var(--ds-space-4), ...) so the artifact stays on-brand. NEVER use external CDNs/network resources (sandboxed). Iterate with action=update_artifact. This is for polished, managed, exportable designs — not throwaway chat visualizations.".into(),
        tier: ToolTier::Configured {
            default_for_main: true,
            default_for_others: true,
            default_deferred: false,
            config_hint: "Settings → Tools → Design Space",
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list_recipes", "get_recipe", "list_systems", "get_system", "extract_system",
                        "import_design_md", "export_system", "export_tokens",
                        "propose_directions", "list_projects", "list_artifacts", "get_artifact",
                        "create_artifact", "update_artifact", "delete_artifact",
                        "versions", "restore", "critique", "save_to_knowledge", "show"
                    ],
                    "description": "Design operation to perform. import_design_md = import a DESIGN.md-spec design system from 'content' (interop format); export_system = export a design system as a portable DESIGN.md; export_tokens = export a design system's tokens as developer code (CSS/SCSS/TS/Swift/Android XML/DTCG JSON) — optionally pass 'format' for a single target."
                },
                "kind": {
                    "type": "string",
                    "enum": ["web", "mobile", "deck", "dashboard", "poster", "document", "email", "image", "motion"],
                    "description": "Artifact form (for create_artifact / filtering list_recipes). web=landing/desktop page, mobile=390x844 framed, deck=16:9 slides (each <section class=\"ds-slide\">), dashboard=data panels, poster=1080x1080, document=long-form, email=table-based, image=generated raster (needs 'prompt'), motion=1280x720 self-contained CSS/JS animation."
                },
                "recipe_id": { "type": "string", "description": "Recipe id (for get_recipe)" },
                "project_id": { "type": "string", "description": "Design project id (optional; defaults to the session's draft project)" },
                "artifact_id": { "type": "string", "description": "Artifact id (for get/update/delete/versions/restore/show)" },
                "system_id": { "type": "string", "description": "Design system id to apply (injects brand tokens)" },
                "kb_id": { "type": "string", "description": "Knowledge base id for save_to_knowledge (optional; defaults to the primary KB)" },
                "from": { "type": "string", "enum": ["brief", "codebase", "url", "image"], "description": "Source for extract_system: 'brief' (text description), 'codebase' (read a local project's CSS/tailwind/theme files at 'path'), 'url' (fetch a live page's HTML at 'url'), or 'image' (analyze a local screenshot/design image at 'path' via a vision model)." },
                "brief": { "type": "string", "description": "Brand description text (for extract_system from=brief)." },
                "content": { "type": "string", "description": "DESIGN.md text to import (for import_design_md): a 9-section design-system doc (Brand/Palette/Typography/Spacing/Layout/Components/Motion/Voice/Anti-patterns) with an optional --ds-* Token table." },
                "path": { "type": "string", "description": "Local path — a codebase directory (from=codebase) or a screenshot/image file (from=image)." },
                "url": { "type": "string", "description": "Web page URL to extract a design system from (for extract_system from=url)." },
                "count": { "type": "integer", "description": "Number of options for propose_directions (2–6, default 4)." },
                "title": { "type": "string", "description": "Artifact title" },
                "prompt": { "type": "string", "description": "Image description (for create_artifact kind=image — generates the image via the configured image provider)." },
                "body_html": { "type": "string", "description": "Artifact body HTML (structure). For deck, use multiple <section class=\"ds-slide\">…</section>." },
                "css": { "type": "string", "description": "Artifact CSS (inline). Reference var(--ds-*) design tokens." },
                "js": { "type": "string", "description": "Optional artifact JavaScript (inline)." },
                "version_id": { "type": "integer", "description": "Version number (for restore)" },
                "version_message": { "type": "string", "description": "Optional message for update" },
                "format": { "type": "string", "enum": ["css", "scss", "ts", "swift", "android", "dtcg"], "description": "Single export target for export_tokens (omit to return all six)." }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}
