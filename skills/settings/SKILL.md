---
name: settings
description: "Manage OpenComputer application settings through conversation. Use when the user wants to view or change: theme, language, proxy, temperature, notifications, tool timeout, context compaction, web search, memory, embedding, recap, cross-session awareness, or any other app configuration. Trigger phrases: 'change settings', 'configure proxy', 'set theme to dark', 'turn off notifications', 'adjust temperature', 'show my settings'. Trigger even when the user doesn't explicitly say 'settings' — any intent to adjust app behavior qualifies."
always: true
---

# Settings — Application Configuration Management

Use `get_settings` and `update_settings` tools to read and modify application settings. **Never edit config files directly.**

## Workflow

1. **Understand intent**: does the user want to view or modify? Which setting category?
2. **Read current values**: call `get_settings(category)` to see current configuration
3. **Confirm changes**: tell the user what you're about to change and the new values, wait for confirmation
4. **Apply changes**: call `update_settings(category, values)` to write
5. **Confirm result**: show the updated configuration

## Tool Usage

### get_settings

```json
{ "category": "theme" }        // Read a single category
{ "category": "all" }          // Overview of all settings
```

### update_settings

```json
{
  "category": "theme",
  "values": { "theme": "dark" }
}
```

`values` uses partial merge — only include fields you want to change, the rest are preserved.

## Settings Category Reference

| Category | Description | Common Fields |
|----------|-------------|---------------|
| `user` | User profile | `name`, `role`, `timezone`, `language`, `responseStyle`, `weatherEnabled`, `weatherCity` |
| `theme` | UI theme | `theme` (`auto`/`light`/`dark`) |
| `language` | Interface language | `language` (`auto`/`zh`/`en`/…) |
| `ui_effects` | Background effects | `uiEffectsEnabled` (bool) |
| `temperature` | LLM temperature | `temperature` (0.0–2.0, null = API default) |
| `proxy` | HTTP proxy | `mode`, `url` |
| `web_search` | Search engine | `provider`, `searxngUrl`, `tavilyApiKey` |
| `web_fetch` | Web fetch tool | `enabled`, `maxBytes` |
| `compact` | Context compaction | `enabled`, `cacheTtlSecs` |
| `notification` | Notifications | `enabled` (bool) |
| `tool_timeout` | Tool execution timeout | `toolTimeout` (seconds, 0=unlimited) |
| `approval` | Tool approval timeout | `approvalTimeoutSecs`, `approvalTimeoutAction` (`deny`/`proceed`) |
| `image_generate` | AI image generation | `provider`, `model` |
| `canvas` | Canvas tool | `enabled` |
| `image` | Image tool | `maxImages` |
| `pdf` | PDF tool | `maxPdfs`, `maxVisionPages` |
| `async_tools` | Async tool execution | `enabled`, `autoBackgroundSecs`, `maxJobSecs` |
| `deferred_tools` | Deferred tool loading | `enabled` |
| `memory_extract` | Auto memory extraction | `enabled`, `cooldownSecs`, `tokenThreshold` |
| `memory_selection` | LLM memory selection | `enabled`, `candidateThreshold`, `maxSelected` |
| `embedding` | Vector embedding model | `provider`, `model`, `dimensions` |
| `embedding_cache` | Embedding cache | `enabled`, `maxEntries` |
| `dedup` | Memory deduplication | `enabled`, `threshold` |
| `hybrid_search` | Hybrid search weights | `keywordWeight`, `vectorWeight` |
| `temporal_decay` | Memory temporal decay | `enabled`, `halfLifeDays` |
| `mmr` | MMR reranking | `enabled`, `lambda` |
| `recap` | Recap reports | `analysisAgent`, `defaultRangeDays`, `facetConcurrency` |
| `cross_session` | Cross-session awareness | `enabled`, `mode` (`structured`/`llm_digest`) |
| `shortcuts` | Global keyboard shortcuts | `bindings` (array) |
| `skills` | Skill management | `extraSkillsDirs`, `disabledSkills`, `skillEnvCheck` |

### Read-Only Categories (cannot be modified via this tool)

| Category | Description |
|----------|-------------|
| `active_model` | Current primary model |
| `fallback_models` | Fallback model chain |

Model selection involves Provider and API Key configuration — guide the user to the Settings UI.

## Important Notes

- **Read before write**: always use `get_settings` to check current values before modifying
- **Confirm before write**: tell the user what you'll change and to what, get confirmation before calling `update_settings`
- **Field names are camelCase**: JSON field names use camelCase (e.g., `softRatio`, `toolTimeout`)
- **Security restrictions**: cannot modify Provider list, Channel configs, or API Keys through this tool
- **Some fields require restart**: changes to `server`, `shortcuts`, etc. may require an app restart — inform the user after modifying
