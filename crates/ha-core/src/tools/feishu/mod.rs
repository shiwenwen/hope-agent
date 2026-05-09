//! Feishu business tools — shared resolver + tools-vec scaffold.
//!
//! All `feishu_*` tools (docx / bitable / drive / wiki / approval / calendar
//! / contact / hire — landing in PR C1-C9) share one entry point:
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
//! [`tools::dispatch::ALL_DISPATCHABLE_TOOLS`]. PR C1 onwards each append
//! their `feishu_<module>_*` tools to the returned vec.

pub mod bitable;
pub mod docx;
pub mod drive;
pub mod wiki;

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::channel::feishu::api::FeishuApi;
use crate::channel::feishu::auth::FeishuAuth;
use crate::channel::types::{ChannelAccountConfig, ChannelId};
use crate::config::cached_config;
use crate::tools::definitions::ToolDefinition;

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

type AuthCache = Mutex<HashMap<String, (CredsSnapshot, Arc<FeishuAuth>)>>;

fn auth_cache() -> &'static AuthCache {
    static CELL: OnceLock<AuthCache> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
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

    let mut cache = auth_cache().lock().await;
    let auth = match cache.get(&target.id) {
        Some((cached_creds, cached_auth)) if *cached_creds == creds => cached_auth.clone(),
        _ => {
            let fresh = Arc::new(FeishuAuth::new(
                &creds.app_id,
                &creds.app_secret,
                &creds.domain,
            ));
            cache.insert(target.id.clone(), (creds, fresh.clone()));
            fresh
        }
    };

    Ok(Arc::new(FeishuApi::new(auth)))
}

// ── Tools list ──────────────────────────────────────────────────

/// Returns the full set of Feishu business tool definitions.
///
/// Each sub-system landed in its own PR appends to this vec; the dispatch
/// layer extends [`tools::dispatch::ALL_DISPATCHABLE_TOOLS`] from here
/// once and the vec grows monotonically as v0.2.0 progresses (C1 docx
/// → C9 hire).
pub fn get_feishu_tools() -> Vec<ToolDefinition> {
    vec![
        // C1 — docx
        docx::create_tool(),
        docx::get_blocks_tool(),
        docx::append_block_tool(),
        docx::update_block_text_tool(),
        // C2 — bitable
        bitable::list_records_tool(),
        bitable::search_records_tool(),
        bitable::create_record_tool(),
        bitable::batch_update_records_tool(),
        // C3 — drive
        drive::list_files_tool(),
        drive::upload_media_tool(),
        drive::download_media_tool(),
        // C4 — wiki
        wiki::get_node_tool(),
    ]
}

/// Whether at least one Feishu account is configured. The dispatcher reads
/// this in `is_globally_configured` so all `feishu_*` tools fall to
/// `HintOnly` when the user has the agent capability enabled but no
/// account configured yet.
pub fn has_any_account_configured() -> bool {
    !enumerate_feishu_accounts().is_empty()
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
        assert!(err.to_string().contains("'acc2' is not configured"), "{}", err);
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
    fn scaffold_returns_no_tools() {
        assert!(get_feishu_tools().is_empty());
    }
}
