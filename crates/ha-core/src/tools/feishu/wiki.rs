//! Feishu wiki (知识库 / Lark Wiki) — 1 LLM tool.
//!
//! - [`feishu_wiki_get_node`] — resolve a wiki token to its node metadata,
//!   including the underlying `obj_token` + `obj_type`. Typical agent
//!   flow: user pastes a wiki link → agent calls this tool to get the
//!   `obj_token` → agent then calls `feishu_docx_get_blocks` (or
//!   `feishu_bitable_*`) on the resolved obj to actually read content.
//!
//! Wiki node creation / child listing are deferred to v0.3+
//! (`feishu-business-tools.md` §9 P3).

use anyhow::Result;
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::{account_param, arg_required_str, arg_str, configured_tier, resolve_feishu_api};

pub const TOOL_WIKI_GET_NODE: &str = "feishu_wiki_get_node";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable wiki tools.";

fn cfg() -> ToolTier {
    configured_tier(CONFIG_HINT)
}

pub fn get_node_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_WIKI_GET_NODE.into(),
        description:
            "Resolve a Feishu (Lark) wiki token to its node metadata: the wiki space, parent \
             node, underlying `obj_token`, and `obj_type`. Use this first when the user gives \
             you a wiki link — then pass the returned `obj_token` to `feishu_docx_get_blocks` \
             (for `obj_type=docx`) or `feishu_bitable_*` (for `obj_type=bitable`) to read or \
             modify the actual content. Required Feishu app scope: `wiki:wiki.readonly` or \
             `wiki:wiki`."
                .into(),
        tier: cfg(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "The wiki token (e.g. `wikcnXxx`) extracted from a wiki URL, or the underlying obj_token (e.g. `doxcnYyy`) when paired with `obj_type`."
                },
                "obj_type": {
                    "type": "string",
                    "description": "Optional object type hint when `token` is the underlying obj_token instead of a wiki token. One of `docx` / `doc` / `sheet` / `mindnote` / `bitable` / `file` / `slides` / `wiki`."
                },
                "account": account_param(),
            },
            "required": ["token"],
            "additionalProperties": false
        }),
    }
}

// ── Execute fn ──────────────────────────────────────────────────

pub(crate) async fn execute_get_node(args: &Value) -> Result<String> {
    let token = arg_required_str(args, "token")?;
    let obj_type = arg_str(args, "obj_type");
    let account = arg_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let node = api.wiki_get_node(token, obj_type).await?;
    Ok(serde_json::to_string(&node)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_has_expected_name_and_tier() {
        let def = get_node_tool();
        assert_eq!(def.name, TOOL_WIKI_GET_NODE);
        match def.tier {
            ToolTier::Configured {
                default_for_main,
                default_for_others,
                config_hint,
                ..
            } => {
                assert!(!default_for_main);
                assert!(!default_for_others);
                assert!(config_hint.contains("Feishu"));
            }
            _ => panic!("must be Tier 3 Configured"),
        }
    }

    #[tokio::test]
    async fn execute_requires_token() {
        let err = execute_get_node(&json!({})).await.unwrap_err();
        assert!(err.to_string().contains("token"), "{}", err);
    }
}
