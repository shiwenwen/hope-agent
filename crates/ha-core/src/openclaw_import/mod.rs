//! OpenClaw → Hope Agent one-shot import.
//!
//! `scan_openclaw_full` reads `~/.openclaw/openclaw.json` (with `.clawdbot`
//! legacy fallback), inspects per-agent `auth-profiles.json` and `MEMORY.md`
//! files, and returns a preview of providers + agents + memories that the
//! frontend can render for review. `import_openclaw_full` then writes the
//! user-selected subset into the live config + agent loader + memory backend.
//!
//! v1 only handles markdown memory; OpenClaw's vector SQLite store is left
//! for v2 (see `memory.rs`).

mod agents;
mod memory;
mod paths;
mod providers;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

pub use agents::{ImportAgentRequest, ImportResult, OpenClawAgentPreview};
pub use providers::{CredentialKind, ProviderPreview, ProviderProfilePreview};

use crate::memory::types::{MemoryScope, NewMemory};

// ── Public types (UI-facing) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPreview {
    pub global_md_present: bool,
    /// (agent_id, estimated entry count) — only includes agents whose
    /// MEMORY.md actually exists on disk.
    pub agent_md_counts: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawImportPreview {
    /// Resolved state directory path (display only).
    pub state_dir: String,
    /// True when the state dir contains no openclaw.json / clawdbot.json.
    /// Frontend uses this to render the "OpenClaw not detected" branch.
    pub state_dir_present: bool,
    pub providers: Vec<ProviderPreview>,
    pub agents: Vec<OpenClawAgentPreview>,
    pub memories: MemoryPreview,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawImportRequest {
    /// `ProviderPreview.source_key` values the user opted in to import.
    pub import_provider_keys: Vec<String>,
    pub import_agents: Vec<ImportAgentRequest>,
    pub import_global_memory: bool,
    /// Agent IDs (target_id from ImportAgentRequest) whose MEMORY.md should be imported.
    pub import_agent_memories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenClawImportSummary {
    /// IDs (UUIDs) of newly added providers.
    pub providers_added: Vec<String>,
    pub agents: Vec<ImportResult>,
    pub memories_added: usize,
    pub warnings: Vec<String>,
}

// ── Public API ───────────────────────────────────────────────────

/// Resolve OpenClaw's state directory. Re-exported for tests + diagnostics.
pub fn resolve_openclaw_state_dir() -> Result<PathBuf> {
    paths::resolve_openclaw_state_dir()
}

/// Full scan: providers + agents + memory inventory in one pass.
pub fn scan_openclaw_full() -> Result<OpenClawImportPreview> {
    let state_dir = paths::resolve_openclaw_state_dir()?;
    let state_dir_str = state_dir.to_string_lossy().to_string();

    let mut warnings: Vec<String> = Vec::new();

    let Some(root) = providers::read_root_config(&state_dir)? else {
        return Ok(OpenClawImportPreview {
            state_dir: state_dir_str,
            state_dir_present: false,
            providers: Vec::new(),
            agents: Vec::new(),
            memories: MemoryPreview {
                global_md_present: false,
                agent_md_counts: Vec::new(),
            },
            warnings,
        });
    };

    let creds = providers::collect_credentials(&state_dir);

    let existing_provider_names: HashSet<String> = crate::config::cached_config()
        .providers
        .iter()
        .map(|p| p.name.clone())
        .collect();

    let (provider_previews, _resolved) =
        providers::build_providers(&root, &creds, &existing_provider_names, &mut warnings);

    let agents_root = read_agents_root(&state_dir)?;
    let agent_previews = agents::build_previews(&state_dir, &agents_root);

    let memories = build_memory_preview(&state_dir, &agents_root);

    Ok(OpenClawImportPreview {
        state_dir: state_dir_str,
        state_dir_present: true,
        providers: provider_previews,
        agents: agent_previews,
        memories,
        warnings,
    })
}

/// Full import. Order: providers → agents (with primary-model wiring) → memory.
pub fn import_openclaw_full(req: &OpenClawImportRequest) -> Result<OpenClawImportSummary> {
    let state_dir = paths::resolve_openclaw_state_dir()?;

    let mut warnings: Vec<String> = Vec::new();

    let root = providers::read_root_config(&state_dir)?.ok_or_else(|| {
        anyhow::anyhow!("OpenClaw config not found under {}", state_dir.display())
    })?;
    let creds = providers::collect_credentials(&state_dir);

    let existing_provider_names: HashSet<String> = crate::config::cached_config()
        .providers
        .iter()
        .map(|p| p.name.clone())
        .collect();

    let (_previews, all_resolved) =
        providers::build_providers(&root, &creds, &existing_provider_names, &mut warnings);

    let selected_keys: HashSet<&str> = req
        .import_provider_keys
        .iter()
        .map(String::as_str)
        .collect();
    let chosen_providers: Vec<providers::ResolvedProvider> = all_resolved
        .into_iter()
        .filter(|r| selected_keys.contains(r.source_key.as_str()))
        .collect();

    let providers_added: Vec<String> = chosen_providers
        .iter()
        .map(|r| r.config.id.clone())
        .collect();

    if !chosen_providers.is_empty() {
        let to_push: Vec<crate::provider::ProviderConfig> =
            chosen_providers.iter().map(|r| r.config.clone()).collect();
        let n = to_push.len();
        crate::config::mutate_config(("providers.add", "openclaw-import"), move |cfg| {
            for p in to_push {
                cfg.providers.push(p);
            }
            Ok(())
        })?;
        app_info!(
            "openclaw_import",
            "import",
            "imported {} provider(s) from OpenClaw",
            n
        );
    }

    let mut model_lookup = agents::build_model_lookup(&chosen_providers);
    let configured_providers = crate::config::cached_config().providers.clone();
    agents::extend_model_lookup_from_provider_configs(&mut model_lookup, &configured_providers);

    let agents_root = read_agents_root(&state_dir)?;
    let source_map: std::collections::HashMap<&str, &agents::OpenClawAgent> = agents_root
        .agents
        .list
        .iter()
        .map(|a| (a.id.as_str(), a))
        .collect();

    let mut agent_results: Vec<ImportResult> = Vec::new();
    let mut successful_agent_targets: HashSet<String> = HashSet::new();
    for areq in &req.import_agents {
        let result = match source_map.get(areq.source_id.as_str()) {
            Some(source) => {
                agents::import_single_agent(&state_dir, source, areq, &model_lookup, &mut warnings)
            }
            None => Err(anyhow::anyhow!(
                "Agent '{}' not found in OpenClaw config",
                areq.source_id
            )),
        };
        match result {
            Ok(()) => {
                successful_agent_targets.insert(areq.target_id.clone());
                agent_results.push(ImportResult {
                    source_id: areq.source_id.clone(),
                    imported_id: areq.target_id.clone(),
                    name: areq.name.clone(),
                    success: true,
                    error: None,
                });
            }
            Err(e) => agent_results.push(ImportResult {
                source_id: areq.source_id.clone(),
                imported_id: areq.target_id.clone(),
                name: areq.name.clone(),
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }

    let mut memories_added = 0usize;
    let mut all_entries: Vec<NewMemory> = Vec::new();

    if req.import_global_memory {
        let path = memory::global_memory_path(&state_dir);
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let entries = memory::parse_openclaw_memory_md(&content, MemoryScope::Global);
                    all_entries.extend(entries);
                }
                Err(e) => warnings.push(format!(
                    "Failed to read global MEMORY.md at {}: {}",
                    path.display(),
                    e
                )),
            }
        }
    }

    if !req.import_agent_memories.is_empty() {
        // Build (target_id → source_id) so we can find the right MEMORY.md.
        let request_pairs: std::collections::HashMap<&str, &str> = req
            .import_agents
            .iter()
            .map(|a| (a.target_id.as_str(), a.source_id.as_str()))
            .collect();

        for target_id in &req.import_agent_memories {
            let Some(source_id) = request_pairs.get(target_id.as_str()) else {
                warnings.push(format!(
                    "Memory selected for target agent '{}' but no matching import request",
                    target_id
                ));
                continue;
            };
            if !successful_agent_targets.contains(target_id) {
                warnings.push(format!(
                    "Skipped MEMORY.md for target agent '{}' because that agent was not imported successfully",
                    target_id
                ));
                continue;
            }
            let Some(path) = memory::agent_memory_path(&state_dir, source_id) else {
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let scope = MemoryScope::Agent {
                        id: target_id.clone(),
                    };
                    let entries = memory::parse_openclaw_memory_md(&content, scope);
                    all_entries.extend(entries);
                }
                Err(e) => warnings.push(format!(
                    "Failed to read MEMORY.md at {}: {}",
                    path.display(),
                    e
                )),
            }
        }
    }

    if !all_entries.is_empty() {
        if let Some(backend) = crate::globals::get_memory_backend() {
            match backend.import_entries(all_entries, true) {
                Ok(result) => {
                    memories_added = result.created;
                    if result.skipped_duplicate > 0 {
                        warnings.push(format!(
                            "{} memory entries skipped as duplicates",
                            result.skipped_duplicate
                        ));
                    }
                    if result.failed > 0 {
                        warnings.push(format!("{} memory entries failed to import", result.failed));
                    }
                    for e in &result.errors {
                        warnings.push(format!("Memory import error: {}", e));
                    }
                }
                Err(e) => {
                    warnings.push(format!("Memory backend rejected import: {}", e));
                }
            }
        } else {
            warnings.push("Memory backend not initialized; skipped memory import".to_string());
        }
    }

    Ok(OpenClawImportSummary {
        providers_added,
        agents: agent_results,
        memories_added,
        warnings,
    })
}

// ── Legacy shims (kept for backwards-compat) ─────────────────────

/// Legacy entry — only returns the agents portion of a full scan.
pub fn scan_openclaw_agents() -> Result<Vec<OpenClawAgentPreview>> {
    let preview = scan_openclaw_full()?;
    if !preview.state_dir_present {
        anyhow::bail!(
            "OpenClaw config not found at {} or its legacy fallback",
            preview.state_dir
        );
    }
    Ok(preview.agents)
}

/// Legacy entry — only imports agents, no providers / memory.
pub fn import_openclaw_agents(requests: &[ImportAgentRequest]) -> Result<Vec<ImportResult>> {
    let req = OpenClawImportRequest {
        import_provider_keys: Vec::new(),
        import_agents: requests.to_vec(),
        import_global_memory: false,
        import_agent_memories: Vec::new(),
    };
    let summary = import_openclaw_full(&req)?;
    Ok(summary.agents)
}

// ── Internal helpers ─────────────────────────────────────────────

fn read_agents_root(state_dir: &std::path::Path) -> Result<agents::OpenClawAgentsRoot> {
    let config_path = paths::resolve_openclaw_config_path(state_dir);
    let data = match std::fs::read_to_string(&config_path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(agents::OpenClawAgentsRoot::default());
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to read {}: {}",
                config_path.display(),
                e
            ))
        }
    };
    serde_json::from_str(&data).with_context(|| {
        format!(
            "Failed to parse {} as OpenClaw agents config",
            config_path.display()
        )
    })
}

fn build_memory_preview(
    state_dir: &std::path::Path,
    agents_root: &agents::OpenClawAgentsRoot,
) -> MemoryPreview {
    let global = memory::global_memory_path(state_dir);
    let global_md_present = global.exists();

    let mut agent_md_counts = Vec::new();
    for agent in &agents_root.agents.list {
        if agent.id.is_empty() {
            continue;
        }
        if let Some(path) = memory::agent_memory_path(state_dir, &agent.id) {
            let count = std::fs::read_to_string(&path)
                .map(|s| memory::estimate_entries(&s))
                .unwrap_or(0);
            if count > 0 {
                agent_md_counts.push((agent.id.clone(), count));
            }
        }
    }

    MemoryPreview {
        global_md_present,
        agent_md_counts,
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, content).expect("write file");
    }

    /// Build a minimal but realistic OpenClaw state dir under `dir`.
    fn make_mock_openclaw(dir: &Path) {
        let openclaw_json = serde_json::json!({
            "agents": {
                "list": [
                    {
                        "id": "default",
                        "name": "Default Agent",
                        "model": { "primary": "claude-sonnet-4-6" },
                        "identity": { "name": "Lily", "emoji": "🌸" }
                    }
                ]
            },
            "models": {
                "providers": {
                    "anthropic": {
                        "baseUrl": "https://api.anthropic.com",
                        "apiKey": "sk-ant-fallback",
                        "api": "anthropic-messages",
                        "models": [
                            {
                                "id": "claude-sonnet-4-6",
                                "name": "Claude Sonnet 4.6",
                                "input": ["text", "image"],
                                "reasoning": false,
                                "cost": { "input": 3.0, "output": 15.0, "cacheRead": 0.3, "cacheWrite": 3.75 },
                                "contextWindow": 200000,
                                "maxTokens": 8192
                            }
                        ]
                    },
                    "ollama-local": {
                        "baseUrl": "http://127.0.0.1:11434",
                        "api": "ollama",
                        "models": [
                            {
                                "id": "llama3:8b",
                                "name": "Llama 3 8B",
                                "input": ["text"],
                                "reasoning": false,
                                "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
                                "contextWindow": 8192,
                                "maxTokens": 4096
                            }
                        ]
                    }
                }
            },
            "auth": {
                "profiles": {
                    "anthropic:work": { "provider": "anthropic", "type": "api_key", "displayName": "Work Key" },
                    "anthropic:cli":  { "provider": "anthropic", "type": "oauth",   "displayName": "Claude CLI", "email": "user@example.com" }
                }
            }
        });
        write(&dir.join("openclaw.json"), &openclaw_json.to_string());

        let auth_profiles = serde_json::json!({
            "version": 1,
            "profiles": {
                "anthropic:work": {
                    "type": "api_key",
                    "provider": "anthropic",
                    "key": "sk-ant-real-work-key",
                    "displayName": "Work Key"
                },
                "anthropic:cli": {
                    "type": "oauth",
                    "provider": "anthropic",
                    "access": "ignore-me",
                    "refresh": "ignore-me",
                    "expires": 0,
                    "displayName": "Claude CLI",
                    "email": "user@example.com"
                }
            }
        });
        write(
            &dir.join("agents/default/agent/auth-profiles.json"),
            &auth_profiles.to_string(),
        );

        write(
            &dir.join("agents/default/agent/MEMORY.md"),
            "# Notes\n\n- I prefer concise responses\n- I work in Beijing time\n",
        );
        write(
            &dir.join("MEMORY.md"),
            "- Global preference: dark mode\n- Always cite sources\n",
        );
    }

    #[test]
    fn parse_memory_md_handles_bullets_and_paragraphs() {
        let content = "# Heading\n\n- bullet one\n- bullet two\n\nA paragraph that\nspans two lines.\n\n* Asterisk bullet\n";
        let entries = memory::parse_openclaw_memory_md(content, MemoryScope::Global);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].content, "bullet one");
        assert_eq!(entries[1].content, "bullet two");
        assert_eq!(entries[2].content, "A paragraph that spans two lines.");
        assert_eq!(entries[3].content, "Asterisk bullet");
        assert_eq!(entries[0].source, "import");
    }

    #[test]
    fn map_api_type_canonical_kinds() {
        let (t, w) = providers::map_api_type("anthropic-messages", "x");
        assert_eq!(t, crate::provider::ApiType::Anthropic);
        assert!(w.is_none());

        let (t, _) = providers::map_api_type("openai-codex-responses", "x");
        // Must NOT be ApiType::Codex (which is OAuth-only).
        assert_eq!(t, crate::provider::ApiType::OpenaiResponses);

        let (t, w) = providers::map_api_type("ollama", "x");
        assert_eq!(t, crate::provider::ApiType::OpenaiChat);
        assert!(w.is_some());

        let (t, w) = providers::map_api_type("bogus-protocol", "x");
        assert_eq!(t, crate::provider::ApiType::OpenaiChat);
        assert!(w.is_some());
    }

    #[test]
    fn scan_returns_not_present_when_empty_dir() {
        let temp = tempdir().expect("tempdir");
        std::env::set_var("OPENCLAW_STATE_DIR", temp.path());

        let preview = scan_openclaw_full().expect("scan");
        assert!(!preview.state_dir_present);
        assert!(preview.providers.is_empty());
        assert!(preview.agents.is_empty());

        std::env::remove_var("OPENCLAW_STATE_DIR");
    }

    #[test]
    fn scan_with_mock_openclaw_state() {
        let temp = tempdir().expect("tempdir");
        make_mock_openclaw(temp.path());
        std::env::set_var("OPENCLAW_STATE_DIR", temp.path());

        let preview = scan_openclaw_full().expect("scan");
        assert!(preview.state_dir_present);

        // 2 providers
        assert_eq!(preview.providers.len(), 2);
        let anth = preview
            .providers
            .iter()
            .find(|p| p.source_key == "anthropic")
            .expect("anthropic provider preview");
        assert_eq!(anth.api_type, crate::provider::ApiType::Anthropic);
        // Two profiles: api_key (will_import=true), oauth (will_import=false)
        assert_eq!(anth.profiles.len(), 3); // work, cli, plus the apiKey-derived "anthropic default"
        let importable = anth.profiles.iter().filter(|p| p.will_import).count();
        // Work key + apiKey fallback (2). OAuth is excluded.
        assert_eq!(importable, 2);
        let oauth_profile = anth
            .profiles
            .iter()
            .find(|p| p.credential_kind == CredentialKind::OAuth)
            .expect("oauth profile preview");
        assert!(!oauth_profile.will_import);

        let ollama = preview
            .providers
            .iter()
            .find(|p| p.source_key == "ollama-local")
            .expect("ollama provider preview");
        assert!(ollama.api_type_warning.is_some());

        // 1 agent
        assert_eq!(preview.agents.len(), 1);
        assert_eq!(preview.agents[0].id, "default");
        // emoji + identity name
        assert_eq!(preview.agents[0].name, "Lily");

        // Memory inventory
        assert!(preview.memories.global_md_present);
        assert_eq!(preview.memories.agent_md_counts.len(), 1);

        // OAuth warning recorded
        let has_oauth_warn = preview.warnings.iter().any(|w| w.contains("OAuth profile"));
        assert!(has_oauth_warn);

        std::env::remove_var("OPENCLAW_STATE_DIR");
    }

    #[test]
    fn name_collision_appends_imported_suffix() {
        // Single provider in mock + simulate that "anthropic" already exists.
        let temp = tempdir().expect("tempdir");
        make_mock_openclaw(temp.path());

        let mut existing = HashSet::new();
        existing.insert("anthropic".to_string());

        let root = providers::read_root_config(temp.path())
            .expect("read")
            .unwrap();
        let creds = providers::collect_credentials(temp.path());
        let mut warnings = Vec::new();
        let (previews, _) = providers::build_providers(&root, &creds, &existing, &mut warnings);

        let anth = previews
            .iter()
            .find(|p| p.source_key == "anthropic")
            .expect("anth");
        assert!(anth.name_conflicts_existing);
        assert_eq!(anth.suggested_name, "anthropic (Imported)");
    }

    #[test]
    fn model_lookup_falls_back_to_existing_provider_configs() {
        let mut provider = crate::provider::ProviderConfig::new(
            "anthropic".to_string(),
            crate::provider::ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            String::new(),
        );
        provider.id = "existing-provider".to_string();
        provider.models.push(crate::provider::ModelConfig {
            id: "claude-sonnet-4-6".to_string(),
            name: "Claude Sonnet 4.6".to_string(),
            input_types: vec!["text".to_string()],
            context_window: 200_000,
            max_tokens: 8_192,
            reasoning: false,
            thinking_style: None,
            cost_input: 3.0,
            cost_output: 15.0,
        });

        let mut lookup = agents::build_model_lookup(&[]);
        agents::extend_model_lookup_from_provider_configs(&mut lookup, &[provider]);

        let found = lookup
            .get("claude-sonnet-4-6")
            .expect("existing provider model should be available");
        assert_eq!(found.provider_uuid, "existing-provider");
    }
}
