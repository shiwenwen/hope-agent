//! Feishu business tools — shared resolver + tools-vec scaffold.
//!
//! All `feishu_*` tools (docx / bitable / drive / wiki / approval / calendar
//! / contact / hire) share one entry point:
//! [`resolve_feishu_api`]. It locates the right Feishu account from the
//! user's configured channels and returns an [`Arc<FeishuApi>`] whose
//! tenant access token is shared, auto-refreshing, and cached across
//! concurrent tool calls.
//!
//! ## Why decouple from running channel state
//!
//! Per [`docs/plans/feishu-business-tools.md`] §6.5, business tools must
//! work even when the IM channel's WebSocket gateway is **not** running —
//! a user may configure a Feishu app for docx / bitable access without
//! subscribing to inbound chat messages. Plugin-internal `accounts` HashMap
//! is only populated when `start_account` runs the gateway, so we can't
//! reuse it; instead we read accounts from `cached_config()` and build
//! [`FeishuApi`] on demand. The token mutex inside [`FeishuAuth`] is cached
//! per `account_id` so process-wide concurrent tool calls share the 7200s
//! token (no double-login).
//!
//! `get_feishu_tools()` is the single entry point used by
//! [`tools::dispatch::ALL_DISPATCHABLE_TOOLS`].

pub mod approval;
pub mod bitable;
pub mod calendar;
pub mod contact;
pub mod docx;
pub mod drive;
pub mod hire;
pub mod wiki;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use crate::channel::feishu::api::FeishuApi;
use crate::channel::feishu::auth::FeishuAuth;
use crate::channel::types::{ChannelAccountConfig, ChannelId};
use crate::config::cached_config;
use crate::tools::definitions::{ToolDefinition, ToolTier};

// ── Auth cache ──────────────────────────────────────────────────

/// Snapshot of the tenant credentials. Cache invalidates whenever any field
/// changes so a credential rotation in Settings → Channels takes effect on
/// the next tool call without an app restart.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CredsSnapshot {
    app_id: String,
    app_secret: String,
    domain: String,
}

type AuthCache = RwLock<HashMap<String, (CredsSnapshot, Arc<FeishuAuth>)>>;

fn auth_cache() -> &'static AuthCache {
    static CELL: OnceLock<AuthCache> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(HashMap::new()))
}

// ── Account enumeration & extraction ────────────────────────────

/// Pull all configured Feishu accounts from the cached AppConfig. Honors
/// the live config — picks up account adds/removes/edits without restart.
pub fn enumerate_feishu_accounts() -> Vec<ChannelAccountConfig> {
    cached_config()
        .channels
        .accounts
        .iter()
        .filter(|a| a.channel_id == ChannelId::Feishu)
        .cloned()
        .collect()
}

/// Allocation-free presence probe used by the dispatcher's hot path
/// (`is_globally_configured` runs once per Feishu tool per LLM round); a
/// full `enumerate_feishu_accounts` would deep-clone every account just to
/// check `is_empty()`.
pub fn has_any_account_configured() -> bool {
    cached_config()
        .channels
        .accounts
        .iter()
        .any(|a| a.channel_id == ChannelId::Feishu)
}

fn extract_creds(account: &ChannelAccountConfig) -> Result<CredsSnapshot> {
    let creds = &account.credentials;
    let app_id = creds
        .get("appId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "Feishu account '{}' is missing 'appId' in credentials",
                account.id
            )
        })?
        .to_string();
    let app_secret = creds
        .get("appSecret")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "Feishu account '{}' is missing 'appSecret' in credentials",
                account.id
            )
        })?
        .to_string();
    let domain = creds
        .get("domain")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("feishu")
        .to_string();
    Ok(CredsSnapshot {
        app_id,
        app_secret,
        domain,
    })
}

// ── Account selection (testable, no global config dep) ──────────

/// Pick the right [`ChannelAccountConfig`] for a tool call given the candidate
/// list and (optional) caller-provided account ID. Pure function — used as
/// the testable core of [`resolve_feishu_api`].
fn select_account(
    accounts: Vec<ChannelAccountConfig>,
    account: Option<&str>,
) -> Result<ChannelAccountConfig> {
    match (account, accounts.len()) {
        (Some(id), _) => accounts
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow!("Feishu account '{}' is not configured", id)),
        (None, 0) => Err(anyhow!(
            "No Feishu channel account configured. Add one in Settings → Channels."
        )),
        (None, 1) => Ok(accounts.into_iter().next().expect("len==1")),
        (None, _) => {
            let ids: Vec<String> = accounts.into_iter().map(|a| a.id).collect();
            Err(anyhow!(
                "{} Feishu accounts configured ({}); pass `account` to disambiguate.",
                ids.len(),
                ids.join(", ")
            ))
        }
    }
}

// ── Public resolver ─────────────────────────────────────────────

/// Resolve a Feishu API client for the (optional) account ID.
///
/// Account selection (see [`select_account`]):
/// - `Some(id)` — look up that specific account; error if not configured.
/// - `None` + 1 configured account — use that account.
/// - `None` + 0 configured accounts — error.
/// - `None` + ≥2 configured accounts — error: must specify.
///
/// Auth state is cached per-account: the token mutex inside [`FeishuAuth`]
/// is shared across all tool calls for a given account, so concurrent tools
/// share the 7200s tenant access token (no double-login). Cache invalidates
/// on credential change.
pub async fn resolve_feishu_api(account: Option<&str>) -> Result<Arc<FeishuApi>> {
    let target = select_account(enumerate_feishu_accounts(), account)?;
    let creds = extract_creds(&target)?;

    // Hot path: read lock, allocation-free hit when creds unchanged.
    if let Some((cached_creds, cached_auth)) = auth_cache().read().await.get(&target.id) {
        if *cached_creds == creds {
            return Ok(Arc::new(FeishuApi::new(cached_auth.clone())));
        }
    }

    // Cold path: take write lock, double-check (another writer may have inserted
    // while we were waiting), insert / replace.
    let mut cache = auth_cache().write().await;
    if let Some((cached_creds, cached_auth)) = cache.get(&target.id) {
        if *cached_creds == creds {
            return Ok(Arc::new(FeishuApi::new(cached_auth.clone())));
        }
    }
    let fresh = Arc::new(FeishuAuth::new(
        &creds.app_id,
        &creds.app_secret,
        &creds.domain,
    ));
    cache.insert(target.id.clone(), (creds, fresh.clone()));
    Ok(Arc::new(FeishuApi::new(fresh)))
}

// ── Tools list ──────────────────────────────────────────────────

/// Returns the full set of Feishu business tool definitions. The dispatch
/// layer extends [`tools::dispatch::ALL_DISPATCHABLE_TOOLS`] from here.
pub fn get_feishu_tools() -> Vec<ToolDefinition> {
    vec![
        docx::create_tool(),
        docx::get_blocks_tool(),
        docx::append_block_tool(),
        docx::update_block_text_tool(),
        bitable::list_records_tool(),
        bitable::search_records_tool(),
        bitable::create_record_tool(),
        bitable::batch_update_records_tool(),
        bitable::list_views_tool(),
        bitable::get_view_tool(),
        bitable::list_dashboards_tool(),
        drive::list_files_tool(),
        drive::upload_media_tool(),
        drive::download_media_tool(),
        wiki::get_node_tool(),
        approval::create_instance_tool(),
        approval::get_instance_tool(),
        approval::cancel_instance_tool(),
        approval::list_instances_tool(),
        approval::subscribe_tool(),
        calendar::list_tool(),
        calendar::create_event_tool(),
        calendar::list_events_tool(),
        calendar::update_event_tool(),
        calendar::delete_event_tool(),
        calendar::attendees_create_tool(),
        contact::get_user_tool(),
        contact::batch_get_users_tool(),
        contact::get_department_tool(),
        contact::search_users_by_department_tool(),
        hire::list_jobs_tool(),
        hire::get_job_tool(),
        hire::list_talents_tool(),
        hire::get_talent_tool(),
        hire::list_applications_tool(),
    ]
}

// ── Shared tool-definition helpers ──────────────────────────────
//
// All `tools/feishu/<module>.rs` files reuse these so the per-module
// boilerplate is just the schema + execute fn.

/// JSON schema fragment for the optional `account` argument every Feishu
/// tool accepts (multi-account routing — single-account users can omit it).
pub(super) fn account_param() -> Value {
    json!({
        "type": "string",
        "description": "Feishu channel account ID. Required only when more than one Feishu account is configured; otherwise the only configured account is used."
    })
}

/// Standard Tier 3 Configured tier for `feishu_*` tools — off-by-default,
/// supports deferred-loading, with a module-specific config_hint surfaced
/// in the `# Unconfigured Capabilities` section of system prompts.
pub(super) fn configured_tier(config_hint: &'static str) -> ToolTier {
    ToolTier::Configured {
        default_for_main: false,
        default_for_others: false,
        default_deferred: true,
        config_hint,
    }
}

// ── Shared argument extraction helpers ──────────────────────────
//
// Used by every `execute_*` function to parse `serde_json::Value` into typed
// Rust args. Centralized so the early "missing/wrong-type" `anyhow::Error`
// messages stay consistent across all 35 feishu_* tools.

pub(super) fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

pub(super) fn arg_required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    arg_str(args, key).ok_or_else(|| anyhow!("`{}` is required and must be a string", key))
}

pub(super) fn arg_u32(args: &Value, key: &str) -> Result<Option<u32>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|x| u32::try_from(x).ok())
            .map(Some)
            .ok_or_else(|| anyhow!("`{}` must be a non-negative integer fitting in u32", key)),
        _ => Err(anyhow!("`{}` must be an integer", key)),
    }
}

pub(super) fn arg_required_object(args: &Value, key: &str) -> Result<Value> {
    args.get(key)
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| anyhow!("`{}` is required and must be an object", key))
}

pub(super) fn arg_required_array(args: &Value, key: &str) -> Result<Value> {
    args.get(key)
        .filter(|v| v.is_array())
        .cloned()
        .ok_or_else(|| anyhow!("`{}` is required and must be an array", key))
}

pub(super) fn arg_required_string_array(args: &Value, key: &str) -> Result<Vec<String>> {
    let arr = args
        .get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("`{}` is required and must be an array of strings", key))?;
    let mut out = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let s = v
            .as_str()
            .ok_or_else(|| anyhow!("`{}[{}]` must be a string", key, i))?;
        out.push(s.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_account(id: &str, channel: ChannelId, app_id: &str) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: id.to_string(),
            channel_id: channel,
            label: format!("test-{}", id),
            enabled: true,
            agent_id: None,
            credentials: json!({
                "appId": app_id,
                "appSecret": "secret",
                "domain": "feishu",
            }),
            settings: serde_json::Value::Null,
            security: Default::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
        }
    }

    #[test]
    fn select_account_no_accounts_no_id_errors() {
        let err = select_account(Vec::new(), None).unwrap_err();
        assert!(
            err.to_string().contains("No Feishu channel account"),
            "{}",
            err
        );
    }

    #[test]
    fn select_account_no_accounts_with_id_errors() {
        let err = select_account(Vec::new(), Some("missing")).unwrap_err();
        assert!(
            err.to_string().contains("'missing' is not configured"),
            "{}",
            err
        );
    }

    #[test]
    fn select_account_single_no_id_picks_only_one() {
        let accounts = vec![make_account("acc1", ChannelId::Feishu, "cli_a")];
        let picked = select_account(accounts, None).unwrap();
        assert_eq!(picked.id, "acc1");
    }

    #[test]
    fn select_account_single_with_matching_id_returns_it() {
        let accounts = vec![make_account("acc1", ChannelId::Feishu, "cli_a")];
        let picked = select_account(accounts, Some("acc1")).unwrap();
        assert_eq!(picked.id, "acc1");
    }

    #[test]
    fn select_account_single_with_wrong_id_errors() {
        let accounts = vec![make_account("acc1", ChannelId::Feishu, "cli_a")];
        let err = select_account(accounts, Some("acc2")).unwrap_err();
        assert!(
            err.to_string().contains("'acc2' is not configured"),
            "{}",
            err
        );
    }

    #[test]
    fn select_account_multi_no_id_errors_with_list() {
        let accounts = vec![
            make_account("acc1", ChannelId::Feishu, "cli_a"),
            make_account("acc2", ChannelId::Feishu, "cli_b"),
        ];
        let err = select_account(accounts, None).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("2 Feishu accounts configured"), "{}", s);
        assert!(s.contains("acc1"), "{}", s);
        assert!(s.contains("acc2"), "{}", s);
        assert!(s.contains("disambiguate"), "{}", s);
    }

    #[test]
    fn select_account_multi_with_id_picks_match() {
        let accounts = vec![
            make_account("acc1", ChannelId::Feishu, "cli_a"),
            make_account("acc2", ChannelId::Feishu, "cli_b"),
        ];
        let picked = select_account(accounts, Some("acc2")).unwrap();
        assert_eq!(picked.id, "acc2");
    }

    #[test]
    fn extract_creds_happy_path() {
        let acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        let creds = extract_creds(&acc).unwrap();
        assert_eq!(creds.app_id, "cli_a");
        assert_eq!(creds.app_secret, "secret");
        assert_eq!(creds.domain, "feishu");
    }

    #[test]
    fn extract_creds_missing_app_id_errors() {
        let mut acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        acc.credentials = json!({"appSecret": "s", "domain": "feishu"});
        let err = extract_creds(&acc).unwrap_err();
        assert!(err.to_string().contains("appId"), "{}", err);
    }

    #[test]
    fn extract_creds_missing_app_secret_errors() {
        let mut acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        acc.credentials = json!({"appId": "cli_a", "domain": "feishu"});
        let err = extract_creds(&acc).unwrap_err();
        assert!(err.to_string().contains("appSecret"), "{}", err);
    }

    #[test]
    fn extract_creds_default_domain_when_missing() {
        let mut acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        acc.credentials = json!({"appId": "cli_a", "appSecret": "secret"});
        let creds = extract_creds(&acc).unwrap();
        assert_eq!(creds.domain, "feishu");
    }

    #[test]
    fn extract_creds_trims_whitespace() {
        let mut acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        acc.credentials = json!({
            "appId": "  cli_a  ",
            "appSecret": "  secret  ",
            "domain": "  lark  ",
        });
        let creds = extract_creds(&acc).unwrap();
        assert_eq!(creds.app_id, "cli_a");
        assert_eq!(creds.app_secret, "secret");
        assert_eq!(creds.domain, "lark");
    }

    #[test]
    fn extract_creds_empty_app_id_after_trim_errors() {
        let mut acc = make_account("acc1", ChannelId::Feishu, "cli_a");
        acc.credentials = json!({"appId": "   ", "appSecret": "s"});
        let err = extract_creds(&acc).unwrap_err();
        assert!(err.to_string().contains("appId"), "{}", err);
    }

    #[test]
    fn get_feishu_tools_returns_full_catalog() {
        // 35 tools across 8 modules; the assertion is a sanity floor not an
        // exact count so adding a tool doesn't churn the test.
        let tools = get_feishu_tools();
        assert!(
            tools.len() >= 30,
            "expected ≥30 feishu tools, got {}",
            tools.len()
        );
        assert!(tools.iter().all(|t| t.name.starts_with("feishu_")));
    }

    #[test]
    fn configured_tier_is_off_by_default_and_supports_deferred() {
        match configured_tier("test hint") {
            ToolTier::Configured {
                default_for_main,
                default_for_others,
                default_deferred,
                config_hint,
            } => {
                assert!(!default_for_main);
                assert!(!default_for_others);
                assert!(default_deferred);
                assert_eq!(config_hint, "test hint");
            }
            _ => panic!("must be Tier 3 Configured"),
        }
    }

    #[test]
    fn arg_helpers_validate_types() {
        let v = json!({"s": "x", "n": 5, "neg": -1, "obj": {"k": "v"}, "arr": ["a"]});
        assert_eq!(arg_str(&v, "s"), Some("x"));
        assert_eq!(arg_str(&v, "missing"), None);
        assert!(arg_required_str(&v, "missing").is_err());
        assert_eq!(arg_u32(&v, "n").unwrap(), Some(5));
        assert_eq!(arg_u32(&v, "missing").unwrap(), None);
        assert!(arg_u32(&v, "neg").is_err());
        assert!(arg_required_object(&v, "obj").is_ok());
        assert!(arg_required_object(&v, "s").is_err());
        assert!(arg_required_array(&v, "arr").is_ok());
        assert!(arg_required_string_array(&v, "arr").is_ok());
    }
}
