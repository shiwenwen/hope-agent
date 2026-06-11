use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config;
use crate::user_config;

/// Categories that exist in `read_category` (and the `get_settings` enum) but are
/// blocked from `update_settings` for security or stability reasons.
///
/// - `active_model` / `fallback_models`: model selection happens in the GUI, since
///   it must coordinate with provider state and runtime agent rebuilds.
/// - `channels`: the IM Channel `accounts[]` array carries bot tokens; per
///   `AGENTS.md` ("强制留在 GUI 的例外") the skill is read-only here.
/// - `mcp_servers`: the per-server config holds OAuth tokens, command paths and
///   trust acknowledgements; writes must go through the GUI which also drives
///   the trust dialog and 0600 credential write.
const BLOCKED_UPDATE_CATEGORIES: &[&str] = &[
    "active_model",
    "fallback_models",
    "channels",
    "mcp_servers",
    // Hooks run arbitrary commands / HTTP / sub-agents on lifecycle events —
    // letting the model write them is a privilege-escalation vector (it could
    // persist its own command execution). Read-only via this tool; writes go
    // through the GUI / scope config files under user supervision.
    "hooks",
    // STT subsystem — providers carry API keys + provider-specific secrets
    // (e.g. Volcengine app_id / access_key, iFlytek app_id), and the active /
    // fallback selection writes coordinate with the desktop voice-input
    // flow + runtime engine cache. Writes must go through Settings → STT.
    "stt_providers",
    "active_stt_model",
    "stt_fallback_models",
];

/// Risk classification for a settings category.
/// The skill / model uses this to decide whether to double-confirm with the user.
/// - `low`: cosmetic / preference changes, trivially reversible
/// - `medium`: behavioral changes that may affect cost, context, or output quality
/// - `high`: security, network exposure, global keybindings, or changes that require restart
fn risk_level(category: &str) -> &'static str {
    match category {
        // ── LOW ────────────────────────────────────────────────
        "user"
        | "theme"
        | "language"
        | "ui_effects"
        | "sidebar_ui"
        | "notification"
        | "startup_notification"
        | "canvas"
        | "image"
        | "pdf"
        | "image_generate"
        | "temperature"
        | "tool_timeout"
        | "default_agent"
        | "local_llm_auto_maintenance" => "low",

        // ── MEDIUM ─────────────────────────────────────────────
        "compact"
        | "session_title"
        | "memory_extract"
        | "memory_selection"
        | "memory_budget"
        | "embedding_cache"
        | "dedup"
        | "hybrid_search"
        | "temporal_decay"
        | "mmr"
        | "multimodal"
        | "dreaming"
        | "recap"
        | "awareness"
        | "web_fetch"
        | "web_search"
        | "deferred_tools"
        | "async_tools"
        | "approval"
        | "tool_result_disk_threshold"
        | "ask_user_question_timeout"
        | "plan"
        | "issue_reporting"
        | "skills_auto_review"
        | "recall_summary"
        | "tool_call_narration"
        | "teams"
        | "im_auto_transcribe"
        | "knowledge_passive_recall"
        | "sprite" => "medium",

        // ── HIGH ───────────────────────────────────────────────
        "proxy" | "embedding" | "shortcuts" | "skills" | "server" | "acp_control" | "skill_env"
        | "security" | "security.ssrf" | "smart_mode" | "mcp_global" | "filesystem"
        // Autonomous maintenance can write to the user's notes (auto_approve =
        // approval policy) — treat as HIGH so the skill confirms before changes.
        | "knowledge_maintenance" => "high",

        // Read-only categories — no risk since they can't be mutated here.
        // `channels` and `mcp_servers` are categorized "low" for read because
        // the response is redacted before it reaches the model.
        "active_model"
        | "fallback_models"
        | "channels"
        | "mcp_servers"
        | "hooks"
        | "stt_providers"
        | "active_stt_model"
        | "stt_fallback_models"
        | "all" => "low",

        _ => "medium",
    }
}

/// Human-readable note about side effects (e.g. "requires app restart").
fn side_effect_note(category: &str) -> Option<&'static str> {
    match category {
        "server" => Some("Changes take effect on next app restart."),
        "shortcuts" => Some("Global shortcut re-registration happens immediately; conflicts may silently fail."),
        "embedding" => {
            Some("Switching embedding provider/model may invalidate existing vector indexes.")
        }
        "proxy" => Some("Proxy change affects ALL outgoing HTTP requests immediately."),
        "skill_env" => Some("Environment variables may contain secrets; values are stored in plaintext in config.json."),
        "acp_control" => Some("Affects external agent delegation; restart recommended after backend changes."),
        "teams" => Some(
            "Team templates are rows in the team_templates DB table, not AppConfig fields. \
             To modify, pass values = { \"action\": \"save\", \"template\": {...} } or \
             { \"action\": \"delete\", \"templateId\": \"...\" }. A saved template becomes \
             discoverable by the model via team(action=\"list_templates\")."
        ),
        "memory_budget" => Some(
            "Reducing totalChars may hide parts of memory.md from the system prompt. \
             Full content is still retrievable via recall_memory / memory_get tools."
        ),
        "security" => Some(
            "⚠️ DANGEROUS MODE: when skipAllApprovals=true, every tool call (exec / write / edit / \
             apply_patch / channel tools / browser / canvas …) runs without any approval gate, \
             overriding all per-session and per-channel auto-approve settings. Plan Mode tool-type \
             restrictions still apply. Recommended only for fully-trusted local automation; never \
             enable on shared machines. Persists to config.json — toggle off in Settings → Security \
             when done."
        ),
        "skills" => Some(
            "⚠️ `allowRemoteInstall` gates the HTTP `POST /api/skills/{name}/install` route that \
             spawns `brew` / `npm -g` / `go install` / `uv tool install`. Enabling it turns any \
             valid API Key into a remote package-install primitive — only enable on trusted \
             deployments. Has no effect on the Tauri desktop shell."
        ),
        "channels" => Some(
            "Read-only via this tool. IM Channel accounts carry bot tokens (Telegram / WeChat / Feishu / QQ / Discord) and must be edited in Settings → Channels so the registry can drop and re-establish listeners under user supervision. The response from get_settings redacts credentials."
        ),
        "smart_mode" => Some(
            "Affects every Smart-mode session: changing strategy / judgeModel / fallback alters which tool calls are auto-approved. JudgeModel-based strategies issue an extra side_query per approvable call (5s hard timeout, 60s TTL cache)."
        ),
        "issue_reporting" => Some(
            "Controls the default GitHub repository and label mapping used by issue_report. The GitHub token is stored separately in the Settings UI; issue_report(action=\"create\") still asks the user before submitting."
        ),
        "mcp_global" => Some(
            "MCP subsystem master switch + concurrency / backoff caps. Flipping enabled=false disconnects every MCP server; loosening backoff caps can cause retry storms; deniedServers prevents users from re-adding listed server names."
        ),
        "mcp_servers" => Some(
            "Read-only via this tool. Server configs carry OAuth tokens, stdio command paths and trust acknowledgements; writes must go through Settings → MCP Servers which drives the trust dialog and writes credentials with 0600 permissions."
        ),
        "hooks" => Some(
            "Read-only via this tool. Hooks run arbitrary commands / HTTP / LLM prompts / sub-agents on lifecycle events, so a writable category would let the model persist its own command execution. Edit hooks in Settings → Hooks or in the scope files (user: config.json; project: <working_dir>/.hope-agent/hooks.json; local: hooks.local.json; managed: /etc/hope-agent/hooks.json). get_settings redacts http handler header values."
        ),
        "multimodal" => Some(
            "Switching modalities or raising maxFileBytes affects which attachments embed: enabling without a multimodal-capable embedding provider will produce empty vectors."
        ),
        "skills_auto_review" => Some(
            "Drives the five-gate skill auto-review pipeline. Trigger / quality-floor fields (cooldown, token / message / tool_use thresholds, min_reuse_probability, min_steps, …) are safe to tune. \
             ⚠️ `review_system_override` replaces the built-in review prompt verbatim and can lower the model-side quality bar — backend gates 2 / 4 / 5 still apply (rejection by deterministic heuristics, self-score floor, body lint), but a malformed override can silently drop dedup or reject-category instructions. `extra_reject_categories` is appended as free-form text to the prompt's reject list. Use `reset_skills_auto_review_config` to revert. See `skills/ha-settings/SKILL.md` for risk levels per field."
        ),
        "dreaming" => Some(
            "Dreaming runs offline LLM consolidation cycles. Disabling stops idle / cron triggers entirely; promotion thresholds gate which candidates get pinned into long-term memory."
        ),
        "knowledge_maintenance" => Some(
            "Layer-2 autonomous maintenance scans knowledge bases and queues note-maintenance proposals (auto-link, dedup merge, tagging, MOC, memory→note, …) for review. Changes take effect on the next cycle. ⚠️ `enabled` lets background cycles run; `autoApprove` makes approved-free writes to the user's notes happen automatically (skipping the review queue) — confirm with the user before enabling either."
        ),
        "sprite" => Some(
            "Knowledge-space sprite / inspiration mode: a proactive companion that, while the user works on a note, makes a bounded LLM call and may surface a transient suggestion bubble. ⚠️ `enabled` makes proactive (unprompted) LLM calls — has a cost. `proactive` (default true) biases it toward speaking vs. staying quiet. `triggers.*` toggle the occasions it may fire (editIdle / noteOpen / conversation / periodic / paste); `idleEditSecs` + `minChangeChars` gate edit-idle, `periodicSecs` the periodic streak, `pasteMinChars` the paste trigger. `cooldownSecs` / `maxPerSessionPerHour` throttle overall frequency; `senses.*` toggle which context (doc / edit / conversation / memory / awareness) is fused in."
        ),
        "stt_providers" => Some(
            "Read-only via this tool. STT provider configs carry API keys (apiKey / authProfiles[*].apiKey) plus provider-specific secrets in `extra` (Volcengine app_id / access_key, iFlytek app_id, Azure region key, etc.). The response from get_settings redacts every secret-bearing field — writes must go through Settings → Speech-to-Text so credentials stay out of conversation logs."
        ),
        "active_stt_model" => Some(
            "Read-only via this tool. STT active model selection — change it in Settings → Speech-to-Text so the desktop voice-input path picks up the new engine without an app restart."
        ),
        "stt_fallback_models" => Some(
            "Read-only via this tool. STT failover chain — change it in Settings → Speech-to-Text."
        ),
        "im_auto_transcribe" => Some(
            "Aggregate view + writer for IM-channel voice auto-transcribe. \
             Read returns `{ imFallbackModel, accounts: [{ id, label, channelId, autoTranscribeVoice }] }`. \
             Write accepts `{ imFallbackModel?: { providerId, modelId } | null, accounts?: [{ id, autoTranscribeVoice }] }`: \
             every field is independently optional, so the model can toggle a single account without restating the fallback or vice versa. \
             Enabling auto-transcribe consumes STT API quota for every inbound voice message; \
             without `imFallbackModel` (or `stt.activeModel` as fallback), the dispatcher logs a warning per message and forwards the original audio unchanged. \
             Original audio is always kept as an attachment alongside the transcript prefix."
        ),
        _ => None,
    }
}

/// Redact API keys + `auth_profiles[*].apiKey` + each `extra` value from a
/// `SttConfig.providers` JSON tree. `extra` keys (e.g. `app_id`, `region`) are
/// preserved so the model can still describe what's configured; only the
/// per-key value gets masked via `redact_string_field`.
fn redact_stt_providers_value(mut value: Value) -> Value {
    let Some(providers) = value.as_array_mut() else {
        return value;
    };
    for provider in providers.iter_mut() {
        let Some(obj) = provider.as_object_mut() else {
            continue;
        };
        redact_string_field(obj, "apiKey");
        if let Some(profiles) = obj.get_mut("authProfiles").and_then(|v| v.as_array_mut()) {
            for profile in profiles.iter_mut() {
                if let Some(p) = profile.as_object_mut() {
                    redact_string_field(p, "apiKey");
                }
            }
        }
        if let Some(extra) = obj.get_mut("extra").and_then(|v| v.as_object_mut()) {
            let keys: Vec<String> = extra.keys().cloned().collect();
            for k in keys {
                redact_string_field(extra, &k);
            }
        }
    }
    value
}

/// Redact secret-bearing fields from a `ChannelStoreConfig` JSON tree before
/// returning it to the model. Strips `accounts[*].credentials`, replaces
/// `settings` with a redacted marker (some channels stash tokens there too),
/// and leaves only structural / display fields visible.
fn redact_channels_value(mut value: Value) -> Value {
    if let Some(accounts) = value.get_mut("accounts").and_then(|v| v.as_array_mut()) {
        for acc in accounts.iter_mut() {
            if let Some(obj) = acc.as_object_mut() {
                if obj.contains_key("credentials") {
                    obj.insert("credentials".into(), json!("[REDACTED]"));
                }
                if obj.contains_key("settings") {
                    obj.insert("settings".into(), json!("[REDACTED]"));
                }
            }
        }
    }
    value
}

/// Redact OAuth tokens / env / headers from `mcp_servers` entries before
/// returning to the model.
fn redact_mcp_servers_value(mut value: Value) -> Value {
    if let Some(arr) = value.as_array_mut() {
        for entry in arr.iter_mut() {
            if let Some(obj) = entry.as_object_mut() {
                for key in ["env", "headers", "oauth"] {
                    if obj.contains_key(key) {
                        obj.insert(key.into(), json!("[REDACTED]"));
                    }
                }
            }
        }
    }
    value
}

/// Replace a non-empty string field with `"[REDACTED]"`. `null`, missing, and
/// the empty-string sentinel are left untouched — the model still needs to
/// distinguish "configured but cleared" (`""`) from "never set" (`null`). Any
/// non-string non-null value (object / array / number) is also masked
/// defensively in case a future schema swap embeds a richer secret payload.
///
/// Used to scrub `providers[*].api_key` style fields from web_search /
/// image_generate read responses without dropping the structural metadata
/// (id, enabled, baseUrl) that lets the model describe what's configured.
/// Redact `http` hook handler header values (they can carry bearer tokens) from
/// a serialized `HooksConfig` tree (`{ Event: [ { hooks: [ {type, headers} ] } ] }`).
/// Non-secret fields (commands, urls, prompts, matchers) are preserved so the
/// model can still describe what's configured.
fn redact_hooks_value(mut value: Value) -> Value {
    if let Some(events) = value.as_object_mut() {
        for groups in events.values_mut() {
            let Some(groups) = groups.as_array_mut() else {
                continue;
            };
            for group in groups {
                let Some(hooks) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) else {
                    continue;
                };
                for hook in hooks {
                    if hook.get("type").and_then(|t| t.as_str()) != Some("http") {
                        continue;
                    }
                    if let Some(headers) = hook.get_mut("headers").and_then(|h| h.as_object_mut()) {
                        for v in headers.values_mut() {
                            if v.as_str().is_some_and(|s| !s.is_empty()) {
                                *v = Value::String("[REDACTED]".to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    value
}

fn redact_string_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(existing) = obj.get(key) {
        let should_mask = match existing {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            _ => true,
        };
        if should_mask {
            obj.insert(key.into(), json!("[REDACTED]"));
        }
    }
}

/// Redact `providers[*].api_key` / `api_key2` from a `WebSearchConfig` JSON
/// tree. Other fields (provider id, enabled flag, base_url) are preserved.
fn redact_web_search_value(mut value: Value) -> Value {
    if let Some(providers) = value.get_mut("providers").and_then(|v| v.as_array_mut()) {
        for entry in providers.iter_mut() {
            if let Some(obj) = entry.as_object_mut() {
                redact_string_field(obj, "apiKey");
                redact_string_field(obj, "apiKey2");
            }
        }
    }
    value
}

/// Redact `providers[*].api_key` from an `ImageGenConfig` JSON tree.
fn redact_image_generate_value(mut value: Value) -> Value {
    if let Some(providers) = value.get_mut("providers").and_then(|v| v.as_array_mut()) {
        for entry in providers.iter_mut() {
            if let Some(obj) = entry.as_object_mut() {
                redact_string_field(obj, "apiKey");
            }
        }
    }
    value
}

/// Redact `apiKey` from an `EmbeddedServerConfig` JSON tree. The bind
/// address / public base URL stay visible so the model can describe how
/// the daemon is exposed.
fn redact_server_value(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        redact_string_field(obj, "apiKey");
    }
    value
}

/// Redact `backends[*].env` from an `AcpControlConfig` JSON tree — env vars
/// frequently contain `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / similar.
fn redact_acp_control_value(mut value: Value) -> Value {
    if let Some(backends) = value.get_mut("backends").and_then(|v| v.as_array_mut()) {
        for entry in backends.iter_mut() {
            if let Some(obj) = entry.as_object_mut() {
                if obj
                    .get("env")
                    .map(|v| v.as_object().is_some_and(|o| !o.is_empty()))
                    .unwrap_or(false)
                {
                    obj.insert("env".into(), json!("[REDACTED]"));
                }
            }
        }
    }
    value
}

// ── get_settings ────────────────────────────────────────────────

pub(crate) async fn tool_get_settings(args: &Value) -> Result<String> {
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    if category == "all" {
        return get_all_overview();
    }

    let value = read_category(category)?;
    let mut response = json!({
        "category": category,
        "riskLevel": risk_level(category),
        "settings": value,
    });
    if let Some(note) = side_effect_note(category) {
        response["sideEffect"] = json!(note);
    }
    Ok(serde_json::to_string_pretty(&response)?)
}

fn read_category(category: &str) -> Result<Value> {
    let cfg = config::cached_config();

    match category {
        "user" => {
            let uc = user_config::load_user_config()?;
            Ok(serde_json::to_value(&uc)?)
        }
        "theme" => Ok(json!({ "theme": cfg.theme })),
        "language" => Ok(json!({ "language": cfg.language })),
        "default_agent" => Ok(json!({ "defaultAgentId": cfg.default_agent_id })),
        "ui_effects" => Ok(json!({ "uiEffectsEnabled": cfg.ui_effects_enabled })),
        "sidebar_ui" => Ok(json!({
            "sidebarUiMode": config::normalize_sidebar_ui_mode(&cfg.sidebar_ui_mode)
        })),
        "proxy" => Ok(serde_json::to_value(&cfg.proxy)?),
        "web_search" => Ok(redact_web_search_value(serde_json::to_value(
            &cfg.web_search,
        )?)),
        "web_fetch" => Ok(serde_json::to_value(&cfg.web_fetch)?),
        "security" => Ok(json!({
            "skipAllApprovals": cfg.permission.global_yolo,
        })),
        "security.ssrf" => Ok(serde_json::to_value(&cfg.ssrf)?),
        "compact" => Ok(serde_json::to_value(&cfg.compact)?),
        "session_title" => Ok(serde_json::to_value(&cfg.session_title)?),
        "notification" => Ok(serde_json::to_value(&cfg.notification)?),
        "startup_notification" => Ok(serde_json::to_value(&cfg.startup_notification)?),
        "temperature" => Ok(json!({ "temperature": cfg.temperature })),
        "tool_timeout" => Ok(json!({ "toolTimeout": cfg.tool_timeout })),
        "approval" => Ok(json!({
            "approvalTimeoutEnabled": cfg.permission.approval_timeout_enabled,
            "approvalTimeoutSecs": cfg.permission.approval_timeout_secs,
            "approvalTimeoutAction": cfg.permission.approval_timeout_action,
        })),
        "image_generate" => Ok(redact_image_generate_value(serde_json::to_value(
            &cfg.image_generate,
        )?)),
        "canvas" => Ok(serde_json::to_value(&cfg.canvas)?),
        "image" => Ok(serde_json::to_value(&cfg.image)?),
        "pdf" => Ok(serde_json::to_value(&cfg.pdf)?),
        "async_tools" => Ok(serde_json::to_value(&cfg.async_tools)?),
        "deferred_tools" => Ok(serde_json::to_value(&cfg.deferred_tools)?),
        "memory_extract" => Ok(serde_json::to_value(&cfg.memory_extract)?),
        "memory_selection" => Ok(serde_json::to_value(&cfg.memory_selection)?),
        "memory_budget" => Ok(serde_json::to_value(&cfg.memory_budget)?),
        "embedding" => Ok(serde_json::to_value(&cfg.embedding)?),
        "embedding_cache" => Ok(serde_json::to_value(&cfg.embedding_cache)?),
        "dedup" => Ok(serde_json::to_value(&cfg.dedup)?),
        "hybrid_search" => Ok(serde_json::to_value(&cfg.hybrid_search)?),
        "temporal_decay" => Ok(serde_json::to_value(&cfg.temporal_decay)?),
        "mmr" => Ok(serde_json::to_value(&cfg.mmr)?),
        "recap" => Ok(serde_json::to_value(&cfg.recap)?),
        "awareness" => Ok(serde_json::to_value(&cfg.awareness)?),
        "shortcuts" => Ok(serde_json::to_value(&cfg.shortcuts)?),
        "active_model" => Ok(serde_json::to_value(&cfg.active_model)?),
        "fallback_models" => Ok(serde_json::to_value(&cfg.fallback_models)?),
        "skills" => Ok(json!({
            "extraSkillsDirs": cfg.extra_skills_dirs,
            "disabledSkills": cfg.disabled_skills,
            "skillEnvCheck": cfg.skill_env_check,
            "allowRemoteInstall": cfg.skills.allow_remote_install,
        })),
        "server" => Ok(redact_server_value(serde_json::to_value(&cfg.server)?)),
        "acp_control" => Ok(redact_acp_control_value(serde_json::to_value(
            &cfg.acp_control,
        )?)),
        "skill_env" => Ok(serde_json::to_value(&cfg.skill_env)?),
        "tool_result_disk_threshold" => Ok(json!({
            "toolResultDiskThreshold": cfg.tool_result_disk_threshold,
        })),
        "ask_user_question_timeout" => Ok(json!({
            "askUserQuestionTimeoutEnabled": cfg.ask_user_question_timeout_enabled,
            "askUserQuestionTimeoutSecs": cfg.ask_user_question_timeout_secs,
        })),
        "plan" => Ok(json!({
            "planSubagent": cfg.plan_subagent,
            "plansDirectory": cfg.plans_directory,
        })),
        "skills_auto_review" => Ok(serde_json::to_value(&cfg.skills.auto_review)?),
        "recall_summary" => Ok(serde_json::to_value(&cfg.recall_summary)?),
        "tool_call_narration" => Ok(json!({
            "toolCallNarrationEnabled": cfg.tool_call_narration_enabled,
        })),
        "issue_reporting" => Ok(json!({
            "config": cfg.issue_reporting,
            "hasToken": crate::issue_reporting::has_token(),
        })),
        "channels" => Ok(redact_channels_value(serde_json::to_value(&cfg.channels)?)),
        "local_llm_auto_maintenance" => Ok(serde_json::to_value(&cfg.local_llm)?),
        "smart_mode" => Ok(serde_json::to_value(&cfg.permission.smart)?),
        "filesystem" => Ok(serde_json::to_value(&cfg.filesystem)?),
        "multimodal" => Ok(serde_json::to_value(&cfg.multimodal)?),
        "dreaming" => Ok(serde_json::to_value(&cfg.dreaming)?),
        "knowledge_maintenance" => Ok(serde_json::to_value(&cfg.knowledge_maintenance)?),
        "knowledge_passive_recall" => Ok(serde_json::to_value(&cfg.knowledge_passive_recall)?),
        "sprite" => Ok(serde_json::to_value(&cfg.sprite)?),
        "mcp_global" => Ok(serde_json::to_value(&cfg.mcp_global)?),
        "mcp_servers" => Ok(redact_mcp_servers_value(serde_json::to_value(
            &cfg.mcp_servers,
        )?)),
        "hooks" => {
            let hooks = redact_hooks_value(serde_json::to_value(&cfg.hooks)?);
            Ok(json!({
                "disableAllHooks": cfg.disable_all_hooks,
                "allowProjectScope": cfg.hooks_allow_project_scope,
                "hooks": hooks,
            }))
        }
        "teams" => {
            let db = crate::globals::get_session_db()
                .ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;
            let templates = db.list_team_templates()?;
            Ok(serde_json::to_value(&templates)?)
        }
        "stt_providers" => Ok(redact_stt_providers_value(serde_json::to_value(
            &cfg.stt.providers,
        )?)),
        "active_stt_model" => Ok(json!({ "activeSttModel": cfg.stt.active_model })),
        "stt_fallback_models" => Ok(json!({ "fallbackModels": cfg.stt.fallback_models })),
        "im_auto_transcribe" => {
            let accounts: Vec<Value> = cfg
                .channels
                .accounts
                .iter()
                .map(|a| {
                    json!({
                        "id": a.id,
                        "label": a.label,
                        "channelId": a.channel_id.to_string(),
                        "autoTranscribeVoice": a.auto_transcribe_voice(),
                    })
                })
                .collect();
            Ok(json!({
                "imFallbackModel": cfg.stt.im_fallback_model,
                "accounts": accounts,
            }))
        }
        _ => bail!("Unknown settings category: '{category}'"),
    }
}

fn get_all_overview() -> Result<String> {
    let cfg = config::cached_config();
    let uc = user_config::load_user_config().unwrap_or_default();

    let overview = json!({
        "user": {
            "name": uc.name,
            "role": uc.role,
            "language": uc.language,
            "timezone": uc.timezone,
            "weatherEnabled": uc.weather_enabled,
            "weatherCity": uc.weather_city,
        },
        "theme": cfg.theme,
        "language": cfg.language,
        "uiEffectsEnabled": cfg.ui_effects_enabled,
        "sidebarUiMode": config::normalize_sidebar_ui_mode(&cfg.sidebar_ui_mode),
        "defaultAgentId": cfg.default_agent_id,
        "temperature": cfg.temperature,
        "toolTimeout": cfg.tool_timeout,
        "approvalTimeoutEnabled": cfg.permission.approval_timeout_enabled,
        "approvalTimeoutSecs": cfg.permission.approval_timeout_secs,
        "notification": {
            "enabled": cfg.notification.enabled,
            "showChatContent": cfg.notification.show_chat_content,
        },
        "proxy": {
            "mode": cfg.proxy.mode,
            "url": cfg.proxy.url,
        },
        "compact": {
            "enabled": cfg.compact.enabled,
            "cacheTtlSecs": cfg.compact.cache_ttl_secs,
            "reactiveMicrocompactEnabled": cfg.compact.reactive_microcompact_enabled,
            "reactiveTriggerRatio": cfg.compact.reactive_trigger_ratio,
        },
        "sessionTitle": cfg.session_title,
        "asyncTools": { "enabled": cfg.async_tools.enabled },
        "issueReporting": {
            "enabled": cfg.issue_reporting.enabled,
            "owner": cfg.issue_reporting.owner,
            "repo": cfg.issue_reporting.repo,
            "hasToken": crate::issue_reporting::has_token(),
        },
        "deferredTools": {
            "enabled": cfg.deferred_tools.enabled,
            "toolNames": cfg.deferred_tools.tool_names,
        },
        "awareness": { "enabled": cfg.awareness.enabled },
        "security": {
            "skipAllApprovals": cfg.permission.global_yolo,
            "ssrfDefaultPolicy": cfg.ssrf.default_policy,
            "trustedHostsCount": cfg.ssrf.trusted_hosts.len(),
        },
        "activeModel": cfg.active_model,
        "fallbackModels": cfg.fallback_models.len(),
        "skills": {
            "extraDirs": cfg.extra_skills_dirs.len(),
            "disabled": cfg.disabled_skills,
            "allowRemoteInstall": cfg.skills.allow_remote_install,
        },
        "smartMode": {
            "strategy": cfg.permission.smart.strategy,
            "fallback": cfg.permission.smart.fallback,
            "judgeModelConfigured": cfg.permission.smart.judge_model.is_some(),
        },
        "mcp": {
            "enabled": cfg.mcp_global.enabled,
            "serverCount": cfg.mcp_servers.len(),
            "deniedServerCount": cfg.mcp_global.denied_servers.len(),
            "maxConcurrentCalls": cfg.mcp_global.max_concurrent_calls,
        },
        "multimodal": { "enabled": cfg.multimodal.enabled },
        "dreaming": {
            "enabled": cfg.dreaming.enabled,
            "idleTriggerEnabled": cfg.dreaming.idle_trigger.enabled,
            "cronTriggerEnabled": cfg.dreaming.cron_trigger.enabled,
        },
        "channels": {
            "accountCount": cfg.channels.accounts.len(),
            "defaultAgentId": cfg.channels.default_agent_id,
        },
        "stt": {
            "providerCount": cfg.stt.providers.len(),
            "activeModel": cfg.stt.active_model,
            "fallbackCount": cfg.stt.fallback_models.len(),
            "imFallbackConfigured": cfg.stt.im_fallback_model.is_some(),
            "imAutoTranscribeAccountCount":
                cfg.channels.accounts.iter().filter(|a| a.auto_transcribe_voice()).count(),
        },
    });

    // Expose risk classification so the model can decide when to double-confirm.
    let risk_levels = json!({
        "low": [
            "user", "theme", "language", "ui_effects", "sidebar_ui", "notification", "startup_notification",
            "canvas", "image", "pdf", "image_generate", "temperature", "tool_timeout",
            "default_agent"
        ],
        "medium": [
            "compact", "session_title", "memory_extract", "memory_selection", "memory_budget",
            "embedding_cache", "dedup", "hybrid_search", "temporal_decay",
            "mmr", "multimodal", "dreaming", "recap", "awareness", "web_fetch", "web_search",
            "deferred_tools", "async_tools", "approval",
            "tool_result_disk_threshold", "ask_user_question_timeout", "plan",
            "issue_reporting", "skills_auto_review", "recall_summary", "tool_call_narration",
            "teams", "im_auto_transcribe", "knowledge_passive_recall", "sprite"
        ],
        "high": [
            "proxy", "embedding", "shortcuts", "skills", "server",
            "acp_control", "skill_env", "security", "security.ssrf",
            "smart_mode", "mcp_global", "knowledge_maintenance"
        ],
        "read_only": [
            "active_model", "fallback_models", "channels", "mcp_servers",
            "stt_providers", "active_stt_model", "stt_fallback_models"
        ],
    });

    Ok(serde_json::to_string_pretty(&json!({
        "category": "all",
        "overview": overview,
        "riskLevels": risk_levels,
        "hint": "Use get_settings with a specific category for full details. HIGH-risk categories require explicit user confirmation before calling update_settings.",
    }))?)
}

// ── update_settings ─────────────────────────────────────────────

pub(crate) async fn tool_update_settings(args: &Value) -> Result<String> {
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: category"))?;

    let values = args
        .get("values")
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: values"))?;

    if !values.is_object() {
        bail!("'values' must be a JSON object");
    }

    if BLOCKED_UPDATE_CATEGORIES.contains(&category) {
        bail!(
            "Category '{category}' cannot be modified through this tool for safety reasons. \
             Please guide the user to change it in the Settings UI.",
        );
    }

    if category == "all" {
        bail!("Cannot update 'all' — specify a single category.");
    }

    if category == "user" {
        return update_user_config(values);
    }

    if category == "session_title" {
        return update_session_title_config(values);
    }

    if category == "im_auto_transcribe" {
        return update_im_auto_transcribe(values);
    }

    update_app_config(category, values).await
}

/// Update STT IM auto-transcribe config: the global fallback model and any
/// number of per-account `autoTranscribeVoice` toggles. Both top-level keys
/// are optional and processed independently.
fn update_im_auto_transcribe(values: &Value) -> Result<String> {
    use crate::stt::ActiveSttModel;

    // imFallbackModel can be missing (skip), `null` (clear), or `{providerId, modelId}` (set).
    if let Some(fallback) = values.get("imFallbackModel") {
        if fallback.is_null() {
            crate::stt::set_im_fallback_stt_model(None, "skill")
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        } else {
            let sel: ActiveSttModel = serde_json::from_value(fallback.clone())
                .map_err(|e| anyhow::anyhow!("imFallbackModel: {}", e))?;
            crate::stt::set_im_fallback_stt_model(Some(sel), "skill")
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
    }

    if let Some(accounts) = values.get("accounts").and_then(|v| v.as_array()) {
        for entry in accounts {
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("accounts entry missing `id`"))?;
            let Some(on) = entry.get("autoTranscribeVoice").and_then(|v| v.as_bool()) else {
                continue;
            };
            crate::channel::accounts::set_account_auto_transcribe_voice(id, on, "skill")?;
        }
    }

    let updated_value = read_category("im_auto_transcribe")?;
    Ok(serde_json::to_string_pretty(&json!({
        "category": "im_auto_transcribe",
        "riskLevel": risk_level("im_auto_transcribe"),
        "updated": true,
        "settings": updated_value,
    }))?)
}

fn update_user_config(values: &Value) -> Result<String> {
    let uc = user_config::load_user_config()?;
    let mut uc_json = serde_json::to_value(&uc)?;
    crate::merge_json(&mut uc_json, values.clone());
    let updated: user_config::UserConfig = serde_json::from_value(uc_json.clone())?;
    // Tag the autosave snapshot so rollback listings know this came from the skill.
    let _reason = crate::backup::scope_save_reason("user", "skill");
    user_config::save_user_config_to_disk(&updated)?;
    drop(_reason);

    // Notify frontend about user config change
    if let Some(bus) = crate::get_event_bus() {
        bus.emit("config:changed", serde_json::json!({ "category": "user" }));
    }

    // Hot-reload: refresh weather cache if weather-related fields changed
    trigger_weather_refresh_if_needed(values);

    Ok(serde_json::to_string_pretty(&json!({
        "category": "user",
        "updated": true,
        "settings": uc_json,
    }))?)
}

fn update_session_title_config(values: &Value) -> Result<String> {
    config::mutate_config(("session_title", "skill"), |store| {
        merge_field(&mut store.session_title, values)
    })?;

    let updated_value = read_category("session_title")?;
    Ok(serde_json::to_string_pretty(&json!({
        "category": "session_title",
        "riskLevel": risk_level("session_title"),
        "updated": true,
        "settings": updated_value,
    }))?)
}

async fn update_app_config(category: &str, values: &Value) -> Result<String> {
    let mut store = config::load_config()?;

    match category {
        "theme" => {
            if let Some(v) = values.get("theme").and_then(|v| v.as_str()) {
                match v {
                    "auto" | "light" | "dark" => store.theme = v.to_string(),
                    _ => bail!("Invalid theme: '{v}'. Must be auto/light/dark."),
                }
            }
        }
        "language" => {
            if let Some(v) = values.get("language").and_then(|v| v.as_str()) {
                store.language = v.to_string();
            }
        }
        "default_agent" => {
            if let Some(v) = values.get("defaultAgentId") {
                if v.is_null() {
                    store.default_agent_id = None;
                } else if let Some(s) = v.as_str() {
                    store.default_agent_id =
                        crate::agent::resolver::normalize_default_agent_id(Some(s));
                } else {
                    bail!("default_agent.defaultAgentId must be a string or null");
                }
            }
        }
        "ui_effects" => {
            if let Some(v) = values.get("uiEffectsEnabled").and_then(|v| v.as_bool()) {
                store.ui_effects_enabled = v;
            }
        }
        "sidebar_ui" => {
            if let Some(v) = values.get("sidebarUiMode").and_then(|v| v.as_str()) {
                store.sidebar_ui_mode = config::normalize_sidebar_ui_mode(v);
            }
        }
        "temperature" => {
            if let Some(v) = values.get("temperature") {
                if v.is_null() {
                    store.temperature = None;
                } else if let Some(t) = v.as_f64() {
                    if !(0.0..=2.0).contains(&t) {
                        bail!("Temperature must be between 0.0 and 2.0, got {t}");
                    }
                    store.temperature = Some(t);
                }
            }
        }
        "tool_timeout" => {
            if let Some(v) = values.get("toolTimeout").and_then(|v| v.as_u64()) {
                store.tool_timeout = v;
            }
        }
        "approval" => {
            if let Some(v) = values
                .get("approvalTimeoutEnabled")
                .and_then(|v| v.as_bool())
            {
                store.permission.approval_timeout_enabled = v;
            }
            if let Some(v) = values.get("approvalTimeoutSecs").and_then(|v| v.as_u64()) {
                store.permission.approval_timeout_secs = v;
            }
            if let Some(v) = values.get("approvalTimeoutAction") {
                store.permission.approval_timeout_action = serde_json::from_value(v.clone())?;
            }
        }
        "proxy" => merge_field(&mut store.proxy, values)?,
        "web_search" => merge_field(&mut store.web_search, values)?,
        "web_fetch" => merge_field(&mut store.web_fetch, values)?,
        "security" => {
            if let Some(v) = values.get("skipAllApprovals").and_then(|v| v.as_bool()) {
                store.permission.global_yolo = v;
            }
        }
        "security.ssrf" => merge_field(&mut store.ssrf, values)?,
        "compact" => merge_field(&mut store.compact, values)?,
        "session_title" => merge_field(&mut store.session_title, values)?,
        "notification" => merge_field(&mut store.notification, values)?,
        "startup_notification" => merge_field(&mut store.startup_notification, values)?,
        "image_generate" => merge_field(&mut store.image_generate, values)?,
        "canvas" => merge_field(&mut store.canvas, values)?,
        "image" => merge_field(&mut store.image, values)?,
        "pdf" => merge_field(&mut store.pdf, values)?,
        "async_tools" => merge_field(&mut store.async_tools, values)?,
        "deferred_tools" => merge_field(&mut store.deferred_tools, values)?,
        "memory_extract" => merge_field(&mut store.memory_extract, values)?,
        "memory_selection" => merge_field(&mut store.memory_selection, values)?,
        "memory_budget" => merge_field(&mut store.memory_budget, values)?,
        "embedding" => merge_field(&mut store.embedding, values)?,
        "embedding_cache" => merge_field(&mut store.embedding_cache, values)?,
        "dedup" => merge_field(&mut store.dedup, values)?,
        "hybrid_search" => merge_field(&mut store.hybrid_search, values)?,
        "temporal_decay" => merge_field(&mut store.temporal_decay, values)?,
        "mmr" => merge_field(&mut store.mmr, values)?,
        "recap" => merge_field(&mut store.recap, values)?,
        "awareness" => merge_field(&mut store.awareness, values)?,
        "shortcuts" => merge_field(&mut store.shortcuts, values)?,
        "skills" => {
            if let Some(v) = values.get("extraSkillsDirs") {
                store.extra_skills_dirs = serde_json::from_value(v.clone())?;
            }
            if let Some(v) = values.get("disabledSkills") {
                store.disabled_skills = serde_json::from_value(v.clone())?;
            }
            if let Some(v) = values.get("skillEnvCheck").and_then(|v| v.as_bool()) {
                store.skill_env_check = v;
            }
            if let Some(v) = values.get("allowRemoteInstall").and_then(|v| v.as_bool()) {
                store.skills.allow_remote_install = v;
            }
        }
        "server" => merge_field(&mut store.server, values)?,
        "acp_control" => merge_field(&mut store.acp_control, values)?,
        "skill_env" => {
            // Per-skill env vars: support full replace via `skillEnv` or per-skill
            // patches via `set` / `remove` to avoid forcing the model to echo
            // every skill's entire env block.
            if let Some(v) = values.get("skillEnv") {
                store.skill_env = serde_json::from_value(v.clone())?;
            }
            if let Some(set) = values.get("set").and_then(|v| v.as_object()) {
                for (skill, vars) in set {
                    let entry = store.skill_env.entry(skill.clone()).or_default();
                    if let Some(vars_obj) = vars.as_object() {
                        for (k, val) in vars_obj {
                            if let Some(s) = val.as_str() {
                                entry.insert(k.clone(), s.to_string());
                            } else if val.is_null() {
                                entry.remove(k);
                            } else {
                                bail!(
                                    "skill_env.set[{skill}].{k} must be a string or null, got {val}"
                                );
                            }
                        }
                    }
                }
            }
            if let Some(remove) = values.get("remove").and_then(|v| v.as_array()) {
                for item in remove {
                    if let Some(skill) = item.as_str() {
                        store.skill_env.remove(skill);
                    }
                }
            }
        }
        "tool_result_disk_threshold" => {
            if let Some(v) = values.get("toolResultDiskThreshold") {
                if v.is_null() {
                    store.tool_result_disk_threshold = None;
                } else if let Some(n) = v.as_u64() {
                    store.tool_result_disk_threshold = Some(n as usize);
                } else {
                    bail!("toolResultDiskThreshold must be a non-negative integer or null");
                }
            }
        }
        "ask_user_question_timeout" => {
            if let Some(v) = values
                .get("askUserQuestionTimeoutEnabled")
                .and_then(|v| v.as_bool())
            {
                store.ask_user_question_timeout_enabled = v;
            }
            if let Some(v) = values
                .get("askUserQuestionTimeoutSecs")
                .and_then(|v| v.as_u64())
            {
                store.ask_user_question_timeout_secs = v;
            }
        }
        "plan" => {
            if let Some(v) = values.get("planSubagent").and_then(|v| v.as_bool()) {
                store.plan_subagent = v;
            }
            if let Some(v) = values.get("plansDirectory") {
                if v.is_null() {
                    store.plans_directory = None;
                } else if let Some(s) = v.as_str() {
                    store.plans_directory = Some(s.to_string());
                } else {
                    bail!("plansDirectory must be a string or null");
                }
            }
        }
        "skills_auto_review" => merge_field(&mut store.skills.auto_review, values)?,
        "recall_summary" => merge_field(&mut store.recall_summary, values)?,
        "tool_call_narration" => {
            if let Some(v) = values
                .get("toolCallNarrationEnabled")
                .and_then(|v| v.as_bool())
            {
                store.tool_call_narration_enabled = v;
            }
        }
        "issue_reporting" => merge_field(&mut store.issue_reporting, values)?,
        "smart_mode" => merge_field(&mut store.permission.smart, values)?,
        "filesystem" => merge_field(&mut store.filesystem, values)?,
        "multimodal" => merge_field(&mut store.multimodal, values)?,
        "dreaming" => merge_field(&mut store.dreaming, values)?,
        "knowledge_maintenance" => {
            merge_field(&mut store.knowledge_maintenance, values)?;
            // Clamp so a skill write can't persist out-of-range values (the GUI path
            // clamps in `service::set_maintenance_config`).
            store.knowledge_maintenance = store.knowledge_maintenance.clamped();
        }
        "knowledge_passive_recall" => {
            merge_field(&mut store.knowledge_passive_recall, values)?;
            // Clamp (mirrors `service::set_passive_recall_config`).
            store.knowledge_passive_recall = store.knowledge_passive_recall.clamped();
        }
        "sprite" => {
            merge_field(&mut store.sprite, values)?;
            // Clamp so a skill write can't hammer the LLM (mirrors `sprite::set_config`).
            store.sprite = store.sprite.clamped();
        }
        "mcp_global" => merge_field(&mut store.mcp_global, values)?,
        "local_llm_auto_maintenance" => {
            // Only the `enabled` toggle is writable through the skill —
            // `userStoppedModels` is owned by the preload/stop UI flow and
            // must not be silently rewritten via natural-language requests.
            if let Some(v) = values.get("enabled").and_then(|v| v.as_bool()) {
                store.local_llm.auto_maintenance.enabled = v;
            }
        }
        "teams" => {
            // Teams are DB rows, not AppConfig fields. Perform CRUD directly on the
            // team_templates table and return early (skip save_config / hot reload).
            return update_team_templates(values);
        }
        _ => bail!("Unknown settings category: '{category}'"),
    }

    // Tag the autosave snapshot so rollback listings carry (category, source).
    let _reason = crate::backup::scope_save_reason(category, "skill");
    config::save_config(&store)?;
    drop(_reason);

    // Notify frontend about config change so UI can react immediately
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": category }),
        );
    }

    // Backend hot-reload: trigger side-effects for categories that cache state
    trigger_backend_hot_reload(category, &store).await?;

    // Return the saved value directly from the mutated store (avoids re-reading cache)
    let updated_value = read_category(category)?;
    let mut response = json!({
        "category": category,
        "riskLevel": risk_level(category),
        "updated": true,
        "settings": updated_value,
    });
    if let Some(note) = side_effect_note(category) {
        response["sideEffect"] = json!(note);
    }
    Ok(serde_json::to_string_pretty(&response)?)
}

/// Handle CRUD on the `team_templates` DB table. This category bypasses the
/// usual AppConfig read-modify-save path because templates live in SQLite.
fn update_team_templates(values: &Value) -> Result<String> {
    let action = values
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "teams: missing 'action'. Expected 'save' (with 'template') or 'delete' (with 'templateId')."
            )
        })?;

    let db = crate::globals::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;

    match action {
        "save" => {
            let payload = values
                .get("template")
                .ok_or_else(|| anyhow::anyhow!("teams.save: missing 'template' payload"))?;
            let template: crate::team::TeamTemplate = serde_json::from_value(payload.clone())?;
            if template.template_id.trim().is_empty() {
                bail!("teams.save: template.templateId must not be empty");
            }
            let saved = crate::team::templates::save_template(&db, template)?;
            Ok(serde_json::to_string_pretty(&json!({
                "category": "teams",
                "riskLevel": risk_level("teams"),
                "action": "save",
                "updated": true,
                "template": saved,
                "sideEffect": side_effect_note("teams"),
            }))?)
        }
        "delete" => {
            let template_id = values
                .get("templateId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("teams.delete: missing 'templateId'"))?;
            crate::team::templates::delete_template(&db, template_id)?;
            Ok(serde_json::to_string_pretty(&json!({
                "category": "teams",
                "riskLevel": risk_level("teams"),
                "action": "delete",
                "updated": true,
                "templateId": template_id,
            }))?)
        }
        other => bail!("teams: unknown action '{other}'. Expected 'save' or 'delete'."),
    }
}

/// Trigger backend hot-reload side-effects for categories that cache state in memory.
async fn trigger_backend_hot_reload(category: &str, store: &config::AppConfig) -> Result<()> {
    match category {
        "embedding" => {
            // Re-initialize embedding provider when config changes
            if let Some(backend) = crate::get_memory_backend() {
                if store.embedding.enabled {
                    match crate::memory::create_embedding_provider(&store.embedding) {
                        Ok(provider) => {
                            backend.set_embedder(provider);
                            app_info!(
                                "settings",
                                "hot_reload",
                                "Embedding provider re-initialized after config change"
                            );
                        }
                        Err(e) => {
                            app_warn!(
                                "settings",
                                "hot_reload",
                                "Failed to re-initialize embedding provider: {}",
                                e
                            );
                        }
                    }
                } else {
                    backend.clear_embedder();
                    app_info!(
                        "settings",
                        "hot_reload",
                        "Embedding provider cleared (disabled)"
                    );
                }
            }
        }
        "web_search" => {
            // SearXNG config may affect Docker container — no cached state to invalidate,
            // but weather system may use web search indirectly. No action needed.
        }
        "smart_mode" => {
            // Smart mode reads PermissionGlobalConfig.smart fresh on every approval
            // decision via cached_config(); no in-memory cache to invalidate.
        }
        "mcp_global" => {
            if let Some(manager) = crate::mcp::McpManager::global() {
                manager
                    .reconcile(store.mcp_global.clone(), store.mcp_servers.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            crate::app_info!(
                "settings",
                "hot_reload",
                "mcp_global hot-reloaded into the MCP manager"
            );
        }
        "multimodal" | "dreaming" => {
            // Both are consumed lazily by their own pipelines on the next
            // trigger; no cached state to refresh.
        }
        _ => {} // Other categories: config cache (ArcSwap) already updated by save_config
    }
    Ok(())
}

/// Trigger weather cache refresh when user_config weather settings change.
fn trigger_weather_refresh_if_needed(values: &Value) {
    let dominated_keys = [
        "weather_enabled",
        "weatherEnabled",
        "weather_city",
        "weatherCity",
        "weather_latitude",
        "weatherLatitude",
        "weather_longitude",
        "weatherLongitude",
    ];
    let needs_refresh = dominated_keys.iter().any(|k| values.get(k).is_some());
    if needs_refresh {
        tokio::spawn(async {
            if let Err(e) = crate::weather::force_refresh_weather().await {
                app_warn!(
                    "settings",
                    "hot_reload",
                    "Failed to refresh weather after user config change: {}",
                    e
                );
            }
        });
    }
}

/// Merge `patch` into a serializable field using deep JSON merge, then deserialize back.
fn merge_field<T>(field: &mut T, patch: &Value) -> Result<()>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut current = serde_json::to_value(&*field)?;
    crate::merge_json(&mut current, patch.clone());
    *field = serde_json::from_value(current)?;
    Ok(())
}

// ── list_settings_backups ───────────────────────────────────────

pub(crate) async fn tool_list_settings_backups(args: &Value) -> Result<String> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(200) as usize;
    let kind_filter = args.get("kind").and_then(|v| v.as_str());

    let mut entries = crate::backup::list_autosaves().map_err(|e| anyhow::anyhow!(e))?;
    if let Some(k) = kind_filter {
        entries.retain(|e| e.kind == k);
    }
    entries.truncate(limit);

    Ok(serde_json::to_string_pretty(&json!({
        "count": entries.len(),
        "backups": entries,
        "hint": "Use restore_settings_backup({id}) to roll back. A pre-restore snapshot is created automatically so the rollback itself is reversible.",
    }))?)
}

// ── restore_settings_backup ─────────────────────────────────────

pub(crate) async fn tool_restore_settings_backup(args: &Value) -> Result<String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: id"))?;

    let entry = crate::backup::restore_autosave(id).map_err(|e| anyhow::anyhow!(e))?;

    app_info!(
        "settings",
        "rollback",
        "Restored autosave id={} kind={} category={}",
        entry.id,
        entry.kind,
        entry.category
    );

    Ok(serde_json::to_string_pretty(&json!({
        "restored": true,
        "entry": entry,
        "note": "A pre-restore snapshot of the previous state was also saved so you can undo this rollback.",
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_level_high_categories() {
        for cat in [
            "proxy",
            "embedding",
            "shortcuts",
            "skills",
            "server",
            "acp_control",
            "skill_env",
            "security",
            "security.ssrf",
            "smart_mode",
            "mcp_global",
            "knowledge_maintenance",
        ] {
            assert_eq!(risk_level(cat), "high", "{cat} should be high risk");
        }
    }

    #[test]
    fn risk_level_medium_includes_new_categories() {
        for cat in ["multimodal", "dreaming", "sprite"] {
            assert_eq!(risk_level(cat), "medium", "{cat} should be medium risk");
        }
    }

    #[test]
    fn risk_level_read_only_categories_low() {
        // Read-only categories report `low` because the model cannot mutate them
        // through this tool — the BLOCKED_UPDATE_CATEGORIES check rejects writes
        // before risk_level is even consulted.
        for cat in ["active_model", "fallback_models", "channels", "mcp_servers"] {
            assert_eq!(risk_level(cat), "low", "{cat} should be low (read-only)");
        }
    }

    #[test]
    fn blocked_update_includes_channels_and_mcp_servers() {
        for cat in ["active_model", "fallback_models", "channels", "mcp_servers"] {
            assert!(
                BLOCKED_UPDATE_CATEGORIES.contains(&cat),
                "{cat} must be in BLOCKED_UPDATE_CATEGORIES"
            );
        }
    }

    #[test]
    fn redact_channels_strips_credentials_and_settings() {
        let original = json!({
            "accounts": [
                {
                    "id": "acc-1",
                    "channelId": "telegram",
                    "label": "primary",
                    "enabled": true,
                    "credentials": { "token": "secret-bot-token-do-not-leak" },
                    "settings": { "transport": "polling", "secretChat": "leak-me" },
                    "autoApproveTools": false
                },
                {
                    "id": "acc-2",
                    "channelId": "discord",
                    "label": "fallback",
                    "enabled": false,
                    "credentials": { "token": "another-token" },
                    "settings": { "guildId": "12345" }
                }
            ],
            "defaultAgentId": "ha-main",
            "defaultModel": null
        });

        let redacted = redact_channels_value(original);
        let arr = redacted["accounts"].as_array().unwrap();
        for acc in arr {
            assert_eq!(acc["credentials"], json!("[REDACTED]"));
            assert_eq!(acc["settings"], json!("[REDACTED]"));
        }
        // Non-secret fields preserved.
        assert_eq!(arr[0]["id"], "acc-1");
        assert_eq!(arr[0]["channelId"], "telegram");
        assert_eq!(arr[0]["enabled"], true);
        assert_eq!(arr[0]["autoApproveTools"], false);
        assert_eq!(redacted["defaultAgentId"], "ha-main");
    }

    #[test]
    fn redact_channels_handles_missing_optional_fields() {
        let original = json!({
            "accounts": [
                { "id": "acc-1", "channelId": "telegram", "label": "primary", "enabled": true }
            ]
        });
        // No credentials/settings → nothing to redact, but call must not panic
        // and the surviving fields stay intact.
        let redacted = redact_channels_value(original);
        assert_eq!(redacted["accounts"][0]["id"], "acc-1");
        assert!(redacted["accounts"][0].get("credentials").is_none());
        assert!(redacted["accounts"][0].get("settings").is_none());
    }

    #[test]
    fn redact_channels_no_panic_on_empty_or_unexpected_shape() {
        // Empty accounts.
        let v = redact_channels_value(json!({ "accounts": [] }));
        assert_eq!(v["accounts"].as_array().unwrap().len(), 0);
        // Missing accounts key entirely.
        let v = redact_channels_value(json!({}));
        assert!(v.is_object());
        // accounts not an array → leave untouched.
        let v = redact_channels_value(json!({ "accounts": "not-an-array" }));
        assert_eq!(v["accounts"], "not-an-array");
    }

    #[test]
    fn redact_mcp_servers_strips_secrets() {
        let original = json!([
            {
                "id": "github-mcp",
                "name": "GitHub",
                "enabled": true,
                "transport": "stdio",
                "env": { "GITHUB_TOKEN": "ghp_secretdonotleak" },
                "headers": { "Authorization": "Bearer leakme" },
                "oauth": { "refresh_token": "very-secret" },
                "trust_level": "trusted"
            },
            {
                "id": "no-auth",
                "name": "PublicMcp",
                "enabled": true
            }
        ]);

        let redacted = redact_mcp_servers_value(original);
        let arr = redacted.as_array().unwrap();
        assert_eq!(arr[0]["env"], json!("[REDACTED]"));
        assert_eq!(arr[0]["headers"], json!("[REDACTED]"));
        assert_eq!(arr[0]["oauth"], json!("[REDACTED]"));
        // Non-sensitive fields preserved.
        assert_eq!(arr[0]["id"], "github-mcp");
        assert_eq!(arr[0]["trust_level"], "trusted");
        // Server with no secret fields untouched.
        assert!(arr[1].get("env").is_none());
    }

    #[test]
    fn redact_hooks_masks_http_headers_keeps_commands() {
        let original = json!({
            "PreToolUse": [
                { "matcher": "Bash", "hooks": [
                    { "type": "command", "command": "./audit.sh" },
                    { "type": "http", "url": "https://h/x", "headers": { "Authorization": "Bearer leakme", "X-Empty": "" } }
                ] }
            ]
        });
        let redacted = redact_hooks_value(original);
        let hook = &redacted["PreToolUse"][0]["hooks"];
        // Command preserved (not a secret — it's what runs).
        assert_eq!(hook[0]["command"], "./audit.sh");
        // http header value with content redacted; empty header left as-is.
        assert_eq!(hook[1]["headers"]["Authorization"], json!("[REDACTED]"));
        assert_eq!(hook[1]["headers"]["X-Empty"], json!(""));
        // url preserved.
        assert_eq!(hook[1]["url"], "https://h/x");
    }

    #[test]
    fn redact_web_search_masks_provider_keys() {
        let original = json!({
            "providers": [
                {"id": "Brave", "enabled": true, "apiKey": "BSA_xxx_secret", "apiKey2": null, "baseUrl": null},
                {"id": "Searxng", "enabled": true, "apiKey": null, "apiKey2": null, "baseUrl": "http://localhost:8888"},
                {"id": "Google", "enabled": false, "apiKey": "AIza_secret", "apiKey2": "cse_id_secret", "baseUrl": null}
            ],
            "defaultResultCount": 5,
            "timeoutSeconds": 30
        });
        let r = redact_web_search_value(original);
        let arr = r["providers"].as_array().unwrap();
        // Non-empty key → redacted.
        assert_eq!(arr[0]["apiKey"], json!("[REDACTED]"));
        assert!(arr[0]["apiKey2"].is_null());
        // Null key untouched.
        assert!(arr[1]["apiKey"].is_null());
        // Both keys redacted on the multi-key provider.
        assert_eq!(arr[2]["apiKey"], json!("[REDACTED]"));
        assert_eq!(arr[2]["apiKey2"], json!("[REDACTED]"));
        // Structural fields preserved.
        assert_eq!(arr[0]["id"], "Brave");
        assert_eq!(arr[0]["enabled"], true);
        assert_eq!(arr[1]["baseUrl"], "http://localhost:8888");
        assert_eq!(r["defaultResultCount"], 5);
    }

    #[test]
    fn redact_web_search_handles_empty_or_missing() {
        // Empty providers array.
        let r = redact_web_search_value(json!({ "providers": [] }));
        assert_eq!(r["providers"].as_array().unwrap().len(), 0);
        // Missing providers entirely.
        let r = redact_web_search_value(json!({ "defaultResultCount": 5 }));
        assert_eq!(r["defaultResultCount"], 5);
        // Empty-string apiKey is not a secret — leave as-is so the model can
        // distinguish "configured but cleared" from "never set" (null).
        let r = redact_web_search_value(json!({
            "providers": [{ "id": "Brave", "apiKey": "" }]
        }));
        assert_eq!(r["providers"][0]["apiKey"], json!(""));
    }

    #[test]
    fn redact_image_generate_masks_provider_keys() {
        let original = json!({
            "providers": [
                {"id": "openai", "enabled": true, "apiKey": "sk-secret"},
                {"id": "stability", "enabled": false, "apiKey": null}
            ],
            "defaultSize": "1024x1024"
        });
        let r = redact_image_generate_value(original);
        assert_eq!(r["providers"][0]["apiKey"], json!("[REDACTED]"));
        assert!(r["providers"][1]["apiKey"].is_null());
        assert_eq!(r["providers"][0]["enabled"], true);
        assert_eq!(r["defaultSize"], "1024x1024");
    }

    #[test]
    fn redact_server_masks_api_key() {
        let r = redact_server_value(json!({
            "bindAddr": "127.0.0.1:8420",
            "apiKey": "long-bearer-token",
            "publicBaseUrl": null
        }));
        assert_eq!(r["apiKey"], json!("[REDACTED]"));
        assert_eq!(r["bindAddr"], "127.0.0.1:8420");
        // Null api_key (server unauthenticated) stays null.
        let r = redact_server_value(json!({ "bindAddr": "127.0.0.1:8420", "apiKey": null }));
        assert!(r["apiKey"].is_null());
    }

    #[test]
    fn redact_acp_control_masks_backend_env() {
        let original = json!({
            "enabled": true,
            "backends": [
                {
                    "id": "claude-code",
                    "name": "Claude Code",
                    "binary": "claude",
                    "enabled": true,
                    "env": { "ANTHROPIC_API_KEY": "sk-ant-secret", "PATH": "/usr/local/bin" }
                },
                {
                    "id": "no-env",
                    "name": "Plain",
                    "binary": "agent",
                    "enabled": true,
                    "env": {}
                }
            ],
            "maxConcurrentSessions": 5
        });
        let r = redact_acp_control_value(original);
        assert_eq!(r["backends"][0]["env"], json!("[REDACTED]"));
        // Empty env stays empty (nothing to leak).
        assert_eq!(r["backends"][1]["env"], json!({}));
        // Structural fields preserved on the redacted entry.
        assert_eq!(r["backends"][0]["id"], "claude-code");
        assert_eq!(r["backends"][0]["enabled"], true);
        assert_eq!(r["enabled"], true);
        assert_eq!(r["maxConcurrentSessions"], 5);
    }

    #[test]
    fn side_effect_notes_present_for_new_high_risk_categories() {
        for cat in [
            "smart_mode",
            "mcp_global",
            "mcp_servers",
            "channels",
            "multimodal",
            "dreaming",
            "knowledge_maintenance",
        ] {
            assert!(
                side_effect_note(cat).is_some(),
                "{cat} should expose a side_effect note"
            );
        }
    }
}
