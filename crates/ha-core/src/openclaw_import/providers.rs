//! Map OpenClaw `models.providers` + per-agent `auth-profiles.json` into
//! Hope Agent's `ProviderConfig` + `AuthProfile` shape.
//!
//! Source of truth for OpenClaw schema:
//! - openclaw/src/config/types.models.ts → `ModelProviderConfig`
//! - openclaw/src/agents/auth-profiles/types.ts → `AuthProfileCredential`

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::provider::{ApiType, AuthProfile, ModelConfig, ProviderConfig, ThinkingStyle};

use super::paths;

// ── Raw deserialization shapes ──────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct OpenClawConfigRoot {
    pub models: Option<OpenClawModelsConfig>,
    pub auth: Option<OpenClawAuthConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct OpenClawModelsConfig {
    pub providers: BTreeMap<String, OpenClawModelProviderConfig>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct OpenClawModelProviderConfig {
    pub base_url: String,
    pub api_key: Option<serde_json::Value>,
    pub api: Option<String>,
    pub models: Vec<OpenClawModelDef>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct OpenClawModelDef {
    pub id: String,
    pub name: Option<String>,
    pub api: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    pub cost: Option<OpenClawCost>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct OpenClawCost {
    pub input: f64,
    pub output: f64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct OpenClawAuthConfig {
    pub profiles: BTreeMap<String, OpenClawAuthProfileMeta>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct OpenClawAuthProfileMeta {
    pub provider: Option<String>,
    /// `"api_key" | "oauth" | "token"`. OpenClaw also accepts the legacy alias `mode`.
    #[serde(rename = "type", alias = "mode")]
    pub kind: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

/// Per-agent `auth-profiles.json` shape.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub(super) struct AuthProfilesFile {
    pub profiles: BTreeMap<String, AuthCredentialEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct AuthCredentialEntry {
    /// `"api_key" | "oauth" | "token"`. Legacy alias `mode` accepted.
    #[serde(rename = "type", alias = "mode")]
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub key: Option<String>,
    pub key_ref: Option<SecretRef>,
    pub token: Option<String>,
    pub token_ref: Option<SecretRef>,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub(super) struct SecretRef {
    /// `"env" | "file" | "exec"`
    pub source: Option<String>,
    /// env var name, file path, or command — interpretation depends on `source`
    pub id: Option<String>,
}

// ── Public preview / output types ───────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialKind {
    /// Plain string API key — usable directly
    ApiKeyPlain,
    /// `keyRef.source == "env"` — resolved at scan via std::env::var
    ApiKeyEnvRef,
    /// OAuth credential — never imported (user must re-login)
    OAuth,
    /// Static bearer token (plaintext)
    Token,
    /// Anything we cannot import (file/exec ref, missing key)
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfilePreview {
    pub source_profile_id: String,
    pub label: String,
    pub credential_kind: CredentialKind,
    pub email: Option<String>,
    /// Whether this profile will be written into the new ProviderConfig.
    pub will_import: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreview {
    pub source_key: String,
    pub suggested_name: String,
    pub api_type: ApiType,
    pub base_url: String,
    pub model_count: usize,
    pub profiles: Vec<ProviderProfilePreview>,
    pub name_conflicts_existing: bool,
    pub api_type_warning: Option<String>,
}

// ── Internal "ready to write" shape ─────────────────────────────

/// Scanned + resolved provider, ready to push into AppConfig on import.
#[derive(Debug, Clone)]
pub(super) struct ResolvedProvider {
    pub source_key: String,
    pub config: ProviderConfig,
    pub model_ids: Vec<String>,
}

// ── Scan ───────────────────────────────────────────────────────

/// Parse `openclaw.json` (or legacy `clawdbot.json`) into the raw root.
/// Returns Ok(None) when no config file exists at all so callers can branch
/// on "OpenClaw not detected" without consuming a stack of error strings.
pub(super) fn read_root_config(state_dir: &Path) -> Result<Option<OpenClawConfigRoot>> {
    let path = paths::resolve_openclaw_config_path(state_dir);
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::anyhow!("Failed to read {}: {}", path.display(), e)),
    };
    let root: OpenClawConfigRoot = serde_json::from_str(&data).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse {} as OpenClaw config: {}",
            path.display(),
            e
        )
    })?;
    Ok(Some(root))
}

/// Walk `~/.openclaw/agents/*/agent/auth-profiles.json` and union the
/// credentials by `(provider, profileId)`. A profile that appears in multiple
/// agent dirs collapses to one entry — we keep the first one with a usable
/// credential.
pub(super) fn collect_credentials(
    state_dir: &Path,
) -> BTreeMap<(String, String), AuthCredentialEntry> {
    let agents_root = state_dir.join("agents");
    let mut out: BTreeMap<(String, String), AuthCredentialEntry> = BTreeMap::new();
    let Ok(read_dir) = std::fs::read_dir(&agents_root) else {
        return out;
    };
    for entry in read_dir.flatten() {
        let agent_id = entry.file_name();
        let Some(agent_id) = agent_id.to_str() else {
            continue;
        };
        let path = paths::auth_profiles_path(state_dir, agent_id);
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed: AuthProfilesFile = match serde_json::from_str(&data) {
            Ok(p) => p,
            Err(_) => continue,
        };
        for (profile_id, cred) in parsed.profiles {
            let provider = cred.provider.clone().unwrap_or_default();
            if provider.is_empty() {
                continue;
            }
            let key = (provider, profile_id);
            let incoming_usable = credential_has_usable_key(&cred);
            match out.get(&key) {
                Some(existing) if credential_has_usable_key(existing) => continue,
                _ if incoming_usable => {
                    out.insert(key, cred);
                }
                _ => {
                    out.entry(key).or_insert(cred);
                }
            }
        }
    }
    out
}

fn credential_has_usable_key(cred: &AuthCredentialEntry) -> bool {
    let kind = cred.kind.as_deref().unwrap_or("");
    match kind {
        "api_key" => cred.key.as_ref().is_some_and(|k| !k.is_empty()),
        "token" => cred.token.as_ref().is_some_and(|k| !k.is_empty()),
        _ => false,
    }
}

/// Build provider previews + the resolved (importable) provider list.
///
/// Logic (per ProviderKey):
///   ApiType ← map_api_type(provider.api ?? "anthropic-messages")
///   For every (provider, profileId) credential whose provider == ProviderKey:
///     - Plus any plain-key in `models.providers[k].apiKey` as an extra default profile
///     - Each becomes one `AuthProfile` (UUID id), enabled iff key was extractable
///   Suggested name = providerKey, with " (Imported)" suffix on conflict
pub(super) fn build_providers(
    root: &OpenClawConfigRoot,
    creds: &BTreeMap<(String, String), AuthCredentialEntry>,
    existing_provider_names: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> (Vec<ProviderPreview>, Vec<ResolvedProvider>) {
    let mut previews = Vec::new();
    let mut resolved = Vec::new();

    let Some(models) = root.models.as_ref() else {
        return (previews, resolved);
    };

    let mut taken_names: HashSet<String> = existing_provider_names.clone();

    for (key, raw_provider) in &models.providers {
        let api_type_raw = raw_provider.api.as_deref().unwrap_or("");
        let (api_type, api_warning) = map_api_type(api_type_raw, key);
        if let Some(w) = api_warning.clone() {
            warnings.push(w);
        }

        let suggested_name = next_unique_name(key, &mut taken_names);
        let conflicts = suggested_name != *key;

        let mut profile_previews: Vec<ProviderProfilePreview> = Vec::new();
        let mut auth_profiles: Vec<AuthProfile> = Vec::new();

        for ((cred_provider, profile_id), cred) in creds.iter() {
            if cred_provider != key {
                continue;
            }
            let preview = build_profile_preview(profile_id, cred, warnings);
            if preview.will_import {
                let api_key = extract_api_key_for_import(cred);
                let label = preview.label.clone();
                auth_profiles.push(AuthProfile::new(label, api_key, None));
            }
            profile_previews.push(preview);
        }

        // Top-level `models.providers[k].apiKey` becomes one extra default profile
        // alongside the per-agent ones — OpenClaw treats both as equivalent.
        if let Some(extra_key) = plain_string_key(raw_provider.api_key.as_ref()) {
            if !extra_key.is_empty() {
                let label = format!("{} default", key);
                profile_previews.push(ProviderProfilePreview {
                    source_profile_id: format!("{}:default", key),
                    label: label.clone(),
                    credential_kind: CredentialKind::ApiKeyPlain,
                    email: None,
                    will_import: true,
                    note: None,
                });
                auth_profiles.push(AuthProfile::new(label, extra_key, None));
            }
        }

        let mut provider_cfg = ProviderConfig::new(
            suggested_name.clone(),
            api_type.clone(),
            raw_provider.base_url.clone(),
            String::new(),
        );
        provider_cfg.thinking_style = thinking_style_for(&api_type);
        provider_cfg.allow_private_network = is_private_url(&raw_provider.base_url);
        provider_cfg.auth_profiles = auth_profiles;

        provider_cfg.models = raw_provider
            .models
            .iter()
            .map(map_model)
            .collect::<Vec<_>>();

        let model_ids: Vec<String> = provider_cfg.models.iter().map(|m| m.id.clone()).collect();

        previews.push(ProviderPreview {
            source_key: key.clone(),
            suggested_name: suggested_name.clone(),
            api_type: api_type.clone(),
            base_url: raw_provider.base_url.clone(),
            model_count: provider_cfg.models.len(),
            profiles: profile_previews,
            name_conflicts_existing: conflicts,
            api_type_warning: api_warning,
        });

        resolved.push(ResolvedProvider {
            source_key: key.clone(),
            config: provider_cfg,
            model_ids,
        });
    }

    (previews, resolved)
}

fn plain_string_key(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn next_unique_name(base: &str, taken: &mut HashSet<String>) -> String {
    if !taken.contains(base) {
        taken.insert(base.to_string());
        return base.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{} (Imported)", base)
        } else {
            format!("{} (Imported {})", base, n)
        };
        if !taken.contains(&candidate) {
            taken.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

fn build_profile_preview(
    profile_id: &str,
    cred: &AuthCredentialEntry,
    warnings: &mut Vec<String>,
) -> ProviderProfilePreview {
    let label = cred
        .display_name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| profile_id.to_string());
    let kind = cred.kind.as_deref().unwrap_or("");
    let email = cred.email.clone().filter(|s| !s.is_empty());

    match kind {
        "oauth" => {
            warnings.push(format!(
                "OAuth profile '{}'{} 不会导入，请在 Hope Agent 中重新登录",
                profile_id,
                email
                    .as_ref()
                    .map(|e| format!(" ({})", e))
                    .unwrap_or_default()
            ));
            ProviderProfilePreview {
                source_profile_id: profile_id.to_string(),
                label,
                credential_kind: CredentialKind::OAuth,
                email,
                will_import: false,
                note: Some("OAuth: re-authenticate in Hope Agent".to_string()),
            }
        }
        "api_key" => match resolve_api_key(cred, warnings, profile_id) {
            Some(_resolved) => ProviderProfilePreview {
                source_profile_id: profile_id.to_string(),
                label,
                credential_kind: if cred.key.as_ref().is_some_and(|k| !k.is_empty()) {
                    CredentialKind::ApiKeyPlain
                } else {
                    CredentialKind::ApiKeyEnvRef
                },
                email,
                will_import: true,
                note: None,
            },
            None => ProviderProfilePreview {
                source_profile_id: profile_id.to_string(),
                label,
                credential_kind: CredentialKind::Missing,
                email,
                will_import: false,
                note: Some("API key not extractable (file/exec ref or missing)".to_string()),
            },
        },
        "token" => {
            if cred.token.as_ref().is_some_and(|t| !t.is_empty()) {
                ProviderProfilePreview {
                    source_profile_id: profile_id.to_string(),
                    label,
                    credential_kind: CredentialKind::Token,
                    email,
                    will_import: true,
                    note: Some("Static bearer token".to_string()),
                }
            } else {
                ProviderProfilePreview {
                    source_profile_id: profile_id.to_string(),
                    label,
                    credential_kind: CredentialKind::Missing,
                    email,
                    will_import: false,
                    note: Some("Token missing or only ref-based".to_string()),
                }
            }
        }
        other => {
            warnings.push(format!(
                "Profile '{}' has unknown credential type '{}', skipped",
                profile_id, other
            ));
            ProviderProfilePreview {
                source_profile_id: profile_id.to_string(),
                label,
                credential_kind: CredentialKind::Missing,
                email,
                will_import: false,
                note: Some(format!("Unknown type: {}", other)),
            }
        }
    }
}

fn resolve_api_key(
    cred: &AuthCredentialEntry,
    warnings: &mut Vec<String>,
    profile_id: &str,
) -> Option<String> {
    if let Some(plain) = cred.key.as_ref() {
        if !plain.is_empty() {
            return Some(plain.clone());
        }
    }
    if let Some(secret_ref) = cred.key_ref.as_ref() {
        let source = secret_ref.source.as_deref().unwrap_or("");
        match source {
            "env" => {
                let id = secret_ref.id.as_deref().unwrap_or("");
                if id.is_empty() {
                    warnings.push(format!(
                        "Profile '{}' has env keyRef without id, skipped",
                        profile_id
                    ));
                    return None;
                }
                match std::env::var(id) {
                    Ok(v) if !v.is_empty() => Some(v),
                    _ => {
                        warnings.push(format!(
                            "Profile '{}' references env var ${} which is not set; importing with empty key",
                            profile_id, id
                        ));
                        None
                    }
                }
            }
            "file" => {
                warnings.push(format!(
                    "Profile '{}' uses file keyRef (not supported in Hope Agent import); please paste the key manually after import",
                    profile_id
                ));
                None
            }
            "exec" => {
                warnings.push(format!(
                    "Profile '{}' uses exec keyRef (refused for security); please paste the key manually after import",
                    profile_id
                ));
                None
            }
            _ => None,
        }
    } else {
        None
    }
}

fn extract_api_key_for_import(cred: &AuthCredentialEntry) -> String {
    let kind = cred.kind.as_deref().unwrap_or("");
    if kind == "api_key" {
        if let Some(plain) = cred.key.as_ref() {
            if !plain.is_empty() {
                return plain.clone();
            }
        }
        if let Some(secret_ref) = cred.key_ref.as_ref() {
            if secret_ref.source.as_deref() == Some("env") {
                if let Some(id) = secret_ref.id.as_deref() {
                    if let Ok(v) = std::env::var(id) {
                        return v;
                    }
                }
            }
        }
    } else if kind == "token" {
        if let Some(token) = cred.token.as_ref() {
            if !token.is_empty() {
                return token.clone();
            }
        }
    }
    String::new()
}

/// Map OpenClaw's ModelApi enum to Hope Agent's ApiType.
///
/// Returns `(ApiType, optional warning string)`. When the OpenClaw API kind
/// has no direct counterpart we fall back to OpenaiChat — covers self-hosted
/// gateways like Ollama or LiteLLM that speak OpenAI Chat-Completions
/// dialect.
pub fn map_api_type(api: &str, source_key: &str) -> (ApiType, Option<String>) {
    match api {
        "anthropic-messages" => (ApiType::Anthropic, None),
        "openai-responses" | "azure-openai-responses" => (ApiType::OpenaiResponses, None),
        // openai-codex-responses is *not* the Hope Agent Codex provider (which
        // is built-in OAuth-only). Map to OpenaiResponses so an external API
        // key still works.
        "openai-codex-responses" => (
            ApiType::OpenaiResponses,
            Some(format!(
                "Provider '{}' uses openai-codex-responses; importing as OpenAI Responses (Hope Agent's Codex type is OAuth-only)",
                source_key
            )),
        ),
        "openai-completions" => (ApiType::OpenaiChat, None),
        "ollama" => (
            ApiType::OpenaiChat,
            Some(format!(
                "Provider '{}' is Ollama; imported as OpenAI Chat Completions (private network access enabled)",
                source_key
            )),
        ),
        "github-copilot" | "google-generative-ai" | "bedrock-converse-stream" => (
            ApiType::OpenaiChat,
            Some(format!(
                "Provider '{}' uses '{}' protocol; mapped to OpenAI Chat Completions compatibility path — requests may not work without manual tweaks",
                source_key, api
            )),
        ),
        "" => (
            ApiType::OpenaiChat,
            Some(format!(
                "Provider '{}' has no api kind; defaulted to OpenAI Chat Completions",
                source_key
            )),
        ),
        other => (
            ApiType::OpenaiChat,
            Some(format!(
                "Provider '{}' has unrecognized api '{}'; defaulted to OpenAI Chat Completions",
                source_key, other
            )),
        ),
    }
}

fn thinking_style_for(api_type: &ApiType) -> ThinkingStyle {
    match api_type {
        ApiType::Anthropic => ThinkingStyle::Anthropic,
        ApiType::OpenaiChat | ApiType::OpenaiResponses | ApiType::Codex => ThinkingStyle::Openai,
    }
}

fn map_model(raw: &OpenClawModelDef) -> ModelConfig {
    let mut input_types = Vec::new();
    for kind in &raw.input {
        let v = kind.to_lowercase();
        if (v == "text" || v == "image") && !input_types.contains(&v) {
            input_types.push(v);
        }
    }
    if input_types.is_empty() {
        input_types.push("text".to_string());
    }

    let (cost_input, cost_output) = match raw.cost.as_ref() {
        Some(c) => normalize_costs(c.input, c.output),
        None => (0.0, 0.0),
    };

    ModelConfig {
        id: raw.id.clone(),
        name: raw.name.clone().unwrap_or_else(|| raw.id.clone()),
        input_types,
        context_window: raw.context_window.unwrap_or(200_000) as u32,
        max_tokens: raw.max_tokens.unwrap_or(8_192) as u32,
        reasoning: raw.reasoning,
        thinking_style: None,
        cost_input,
        cost_output,
    }
}

/// OpenClaw cost values are in USD per million tokens (matches Hope Agent).
/// Some legacy configs accidentally store per-token values; if both costs
/// look like per-token (< 0.01 USD) we scale them up.
fn normalize_costs(input: f64, output: f64) -> (f64, f64) {
    if input > 0.0 && output > 0.0 && input < 0.01 && output < 0.01 {
        (input * 1_000_000.0, output * 1_000_000.0)
    } else {
        (input, output)
    }
}

fn is_private_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("localhost") || lower.contains("127.0.0.1") || lower.contains("0.0.0.0")
}
