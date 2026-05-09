//! Feishu docx (云文档 / Lark Docs) — 4 LLM tools.
//!
//! - [`feishu_docx_create`] — create an empty document
//! - [`feishu_docx_get_blocks`] — list blocks (paginated)
//! - [`feishu_docx_append_block`] — append a child block under a given parent
//! - [`feishu_docx_update_block_text`] — overwrite a text-bearing block's content
//!
//! All four go through [`super::resolve_feishu_api`] so they pick the
//! correct configured account (single-account convenience or explicit
//! `account` arg) and share the cached tenant access token. Tier 3
//! Configured — opt-in per agent; the system prompt's `# Unconfigured
//! Capabilities` section nudges the user when the agent enabled Feishu
//! tools without a configured account.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::tools::definitions::{ToolDefinition, ToolTier};

use super::resolve_feishu_api;

pub const TOOL_DOCX_CREATE: &str = "feishu_docx_create";
pub const TOOL_DOCX_GET_BLOCKS: &str = "feishu_docx_get_blocks";
pub const TOOL_DOCX_APPEND_BLOCK: &str = "feishu_docx_append_block";
pub const TOOL_DOCX_UPDATE_BLOCK_TEXT: &str = "feishu_docx_update_block_text";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable docx tools.";

fn account_param() -> Value {
    json!({
        "type": "string",
        "description": "Feishu channel account ID. Required only when more than one Feishu account is configured; otherwise the only configured account is used."
    })
}

fn configured_tier() -> ToolTier {
    ToolTier::Configured {
        // Off-by-default — Feishu tools are niche; enabling them on every
        // agent would bloat prompts. Users opt in via Agent → Capabilities.
        default_for_main: false,
        default_for_others: false,
        // Eligible for the deferred-loading pool (10+ feishu_* tools land
        // by v0.2.0; users with deferredTools.enabled can move them to
        // tool_search to keep the eager schema small).
        default_deferred: true,
        config_hint: CONFIG_HINT,
    }
}

// ── Tool definitions ────────────────────────────────────────────

pub fn create_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOCX_CREATE.into(),
        description:
            "Create a new Feishu (Lark) docx document. Returns the new `document_id` which can be \
             passed to other docx_* tools to read or modify content. Required Feishu app scope: \
             `docx:document`."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Optional initial title for the new document."
                },
                "folder_token": {
                    "type": "string",
                    "description": "Optional drive folder token to create the document in. Defaults to the user's drive root."
                },
                "account": account_param(),
            },
            "additionalProperties": false
        }),
    }
}

pub fn get_blocks_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOCX_GET_BLOCKS.into(),
        description:
            "List all blocks in a Feishu (Lark) docx document. Returns one page of blocks plus a \
             `page_token` if more pages exist. Pass that token back as `page_token` to fetch the \
             next page. Required Feishu app scope: `docx:document.readonly` or `docx:document`."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The docx document ID (e.g. `doxcnAbC123`)."
                },
                "page_token": {
                    "type": "string",
                    "description": "Pagination token from a previous call. Omit for the first page."
                },
                "page_size": {
                    "type": "integer",
                    "description": "Items per page, 1-500. Default 100."
                },
                "account": account_param(),
            },
            "required": ["document_id"],
            "additionalProperties": false
        }),
    }
}

pub fn append_block_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOCX_APPEND_BLOCK.into(),
        description:
            "Append a new child block under an existing block in a Feishu (Lark) docx. The \
             `block` argument must conform to Feishu's docx block schema. The most common \
             paragraph block is `{\"block_type\": 2, \"text\": {\"style\": {}, \"elements\": \
             [{\"text_run\": {\"content\": \"hello\"}}]}}`. To append at the document root, use \
             the document's root block ID (typically equal to `document_id`) as `parent_block_id`. \
             Required Feishu app scope: `docx:document`."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The docx document ID."
                },
                "parent_block_id": {
                    "type": "string",
                    "description": "Block ID under which to append. Use the document's root block ID for top-level append."
                },
                "block": {
                    "type": "object",
                    "description": "The new block in Feishu docx block schema. See Feishu docs for `block_type` values (2=paragraph, 3=heading1, 12=bulleted, 13=ordered, etc.)."
                },
                "index": {
                    "type": "integer",
                    "description": "Optional 0-based insert position among the parent's children. Default appends at the end."
                },
                "account": account_param(),
            },
            "required": ["document_id", "parent_block_id", "block"],
            "additionalProperties": false
        }),
    }
}

pub fn update_block_text_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_DOCX_UPDATE_BLOCK_TEXT.into(),
        description:
            "Overwrite the text content of an existing text-bearing block (paragraph / heading / \
             list item) in a Feishu (Lark) docx. Destructive — replaces the entire `elements` \
             array of the block. To preserve inline styling, prefer creating a fresh block with \
             `feishu_docx_append_block`. Required Feishu app scope: `docx:document`."
                .into(),
        tier: configured_tier(),
        internal: false,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The docx document ID."
                },
                "block_id": {
                    "type": "string",
                    "description": "Target block ID (must already exist and carry text)."
                },
                "text": {
                    "type": "string",
                    "description": "New plain text content. Replaces the block's existing text entirely."
                },
                "account": account_param(),
            },
            "required": ["document_id", "block_id", "text"],
            "additionalProperties": false
        }),
    }
}

// ── Argument helpers ────────────────────────────────────────────

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn get_required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    get_str(args, key).ok_or_else(|| anyhow!("`{}` is required and must be a string", key))
}

fn get_u32(args: &Value, key: &str) -> Result<Option<u32>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().and_then(|x| u32::try_from(x).ok()).map(Some).ok_or_else(
            || anyhow!("`{}` must be a non-negative integer fitting in u32", key),
        ),
        _ => Err(anyhow!("`{}` must be an integer", key)),
    }
}

// ── Execute fns ─────────────────────────────────────────────────

pub(crate) async fn execute_create(args: &Value) -> Result<String> {
    let title = get_str(args, "title");
    let folder_token = get_str(args, "folder_token");
    let account = get_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let doc = api.docx_create(title, folder_token).await?;
    Ok(serde_json::to_string(&doc)?)
}

pub(crate) async fn execute_get_blocks(args: &Value) -> Result<String> {
    let document_id = get_required_str(args, "document_id")?;
    let page_token = get_str(args, "page_token");
    let page_size = get_u32(args, "page_size")?;
    let account = get_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let page = api
        .docx_get_blocks(document_id, page_token, page_size)
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_append_block(args: &Value) -> Result<String> {
    let document_id = get_required_str(args, "document_id")?;
    let parent_block_id = get_required_str(args, "parent_block_id")?;
    let block = args
        .get("block")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| anyhow!("`block` is required and must be an object"))?;
    let index = get_u32(args, "index")?;
    let account = get_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let result = api
        .docx_append_block(document_id, parent_block_id, block, index)
        .await?;
    Ok(serde_json::to_string(&result)?)
}

pub(crate) async fn execute_update_block_text(args: &Value) -> Result<String> {
    let document_id = get_required_str(args, "document_id")?;
    let block_id = get_required_str(args, "block_id")?;
    let text = get_required_str(args, "text")?;
    let account = get_str(args, "account");
    let api = resolve_feishu_api(account).await?;
    let result = api
        .docx_update_block_text(document_id, block_id, text)
        .await?;
    Ok(serde_json::to_string(&result)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_have_expected_names() {
        assert_eq!(create_tool().name, TOOL_DOCX_CREATE);
        assert_eq!(get_blocks_tool().name, TOOL_DOCX_GET_BLOCKS);
        assert_eq!(append_block_tool().name, TOOL_DOCX_APPEND_BLOCK);
        assert_eq!(update_block_text_tool().name, TOOL_DOCX_UPDATE_BLOCK_TEXT);
    }

    #[test]
    fn definitions_are_tier_configured_off_by_default() {
        for def in [
            create_tool(),
            get_blocks_tool(),
            append_block_tool(),
            update_block_text_tool(),
        ] {
            match def.tier {
                ToolTier::Configured {
                    default_for_main,
                    default_for_others,
                    config_hint,
                    ..
                } => {
                    assert!(!default_for_main, "{} should be off-by-default", def.name);
                    assert!(!default_for_others, "{} should be off-by-default", def.name);
                    assert!(config_hint.contains("Feishu"), "{}", def.name);
                }
                _ => panic!("{} must be Tier 3 Configured", def.name),
            }
        }
    }

    #[tokio::test]
    async fn execute_get_blocks_requires_document_id() {
        let err = execute_get_blocks(&json!({})).await.unwrap_err();
        assert!(
            err.to_string().contains("document_id"),
            "{}",
            err
        );
    }

    #[tokio::test]
    async fn execute_append_block_requires_block_object() {
        let err = execute_append_block(&json!({
            "document_id": "doxcnX",
            "parent_block_id": "p1",
            "block": "not-an-object"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("block"), "{}", err);
    }

    #[tokio::test]
    async fn execute_update_block_text_requires_text() {
        let err = execute_update_block_text(&json!({
            "document_id": "doxcnX",
            "block_id": "b1"
        }))
        .await
        .unwrap_err();
        assert!(err.to_string().contains("text"), "{}", err);
    }

    #[test]
    fn get_u32_rejects_negative() {
        let v = json!({"page_size": -5});
        let err = get_u32(&v, "page_size").unwrap_err();
        assert!(err.to_string().contains("u32"), "{}", err);
    }

    #[test]
    fn get_u32_returns_none_for_missing() {
        let v = json!({});
        assert_eq!(get_u32(&v, "page_size").unwrap(), None);
    }
}
