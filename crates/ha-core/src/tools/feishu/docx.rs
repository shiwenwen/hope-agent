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

use super::{
    account_param, arg_required_str, arg_str, arg_u32, configured_tier, resolve_feishu_api,
};

pub const TOOL_DOCX_CREATE: &str = "feishu_docx_create";
pub const TOOL_DOCX_GET_BLOCKS: &str = "feishu_docx_get_blocks";
pub const TOOL_DOCX_APPEND_BLOCK: &str = "feishu_docx_append_block";
pub const TOOL_DOCX_UPDATE_BLOCK_TEXT: &str = "feishu_docx_update_block_text";

const CONFIG_HINT: &str =
    "Configure a Feishu IM channel account in Settings → Channels to enable docx tools.";

fn cfg() -> ToolTier {
    configured_tier(CONFIG_HINT)
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
        tier: cfg(),
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
        tier: cfg(),
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
        tier: cfg(),
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
        tier: cfg(),
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

// ── Execute fns ─────────────────────────────────────────────────

pub(crate) async fn execute_create(args: &Value) -> Result<String> {
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let doc = api
        .docx_create(arg_str(args, "title"), arg_str(args, "folder_token"))
        .await?;
    Ok(serde_json::to_string(&doc)?)
}

pub(crate) async fn execute_get_blocks(args: &Value) -> Result<String> {
    let document_id = arg_required_str(args, "document_id")?;
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let page = api
        .docx_get_blocks(
            document_id,
            arg_str(args, "page_token"),
            arg_u32(args, "page_size")?,
        )
        .await?;
    Ok(serde_json::to_string(&page)?)
}

pub(crate) async fn execute_append_block(args: &Value) -> Result<String> {
    let block = args
        .get("block")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| anyhow!("`block` is required and must be an object"))?;
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
    let result = api
        .docx_append_block(
            arg_required_str(args, "document_id")?,
            arg_required_str(args, "parent_block_id")?,
            block,
            arg_u32(args, "index")?,
        )
        .await?;
    Ok(serde_json::to_string(&result)?)
}

pub(crate) async fn execute_update_block_text(args: &Value) -> Result<String> {
    let document_id = arg_required_str(args, "document_id")?;
    let block_id = arg_required_str(args, "block_id")?;
    let text = arg_required_str(args, "text")?;
    let api = resolve_feishu_api(arg_str(args, "account")).await?;
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
        assert!(err.to_string().contains("document_id"), "{}", err);
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

    // arg_u32 / arg_str etc. are exercised by `super::tests` in mod.rs.
}
