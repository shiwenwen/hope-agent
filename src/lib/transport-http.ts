/**
 * HTTP / WebSocket transport implementation.
 *
 * Used when the frontend runs outside of Tauri (standalone web mode).
 * Maps Tauri command names to REST endpoints and uses WebSockets for
 * streaming chat and backend events.
 */

import type { Transport, ChatStream } from "@/lib/transport";
import type { MediaItem } from "@/types/chat";

// ---------------------------------------------------------------------------
// Command → REST endpoint mapping
// ---------------------------------------------------------------------------

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE" | "PATCH";

interface EndpointDef {
  method: HttpMethod;
  /**
   * Path template. Use `{paramName}` for path parameters that will be
   * extracted from the `args` object.
   */
  path: string;
}

/**
 * Lookup table mapping Tauri command names to REST endpoints.
 *
 * Only the most commonly used commands are mapped here. For unmapped commands
 * `call()` will throw an explicit error. Extend this map as the HTTP backend
 * gains more routes.
 */
const COMMAND_MAP: Record<string, EndpointDef> = {
  // -- Projects --
  list_projects_cmd:               { method: "GET",    path: "/api/projects" },
  get_project_cmd:                 { method: "GET",    path: "/api/projects/{id}" },
  create_project_cmd:              { method: "POST",   path: "/api/projects" },
  update_project_cmd:              { method: "PATCH",  path: "/api/projects/{id}" },
  delete_project_cmd:              { method: "DELETE", path: "/api/projects/{id}" },
  archive_project_cmd:             { method: "POST",   path: "/api/projects/{id}/archive" },
  list_project_sessions_cmd:       { method: "GET",    path: "/api/projects/{id}/sessions" },
  move_session_to_project_cmd:     { method: "PATCH",  path: "/api/sessions/{sessionId}/project" },
  list_project_files_cmd:          { method: "GET",    path: "/api/projects/{projectId}/files" },
  upload_project_file_cmd:         { method: "POST",   path: "/api/projects/{projectId}/files" },
  delete_project_file_cmd:         { method: "DELETE", path: "/api/projects/{projectId}/files/{fileId}" },
  rename_project_file_cmd:         { method: "PATCH",  path: "/api/projects/{projectId}/files/{fileId}" },
  read_project_file_content_cmd:   { method: "GET",    path: "/api/projects/{projectId}/files/{fileId}/content" },
  list_project_memories_cmd:       { method: "GET",    path: "/api/projects/{id}/memories" },

  // -- Sessions --
  list_sessions_cmd:               { method: "GET",    path: "/api/sessions" },
  search_sessions_cmd:             { method: "GET",    path: "/api/sessions/search" },
  load_session_messages_latest_cmd:{ method: "GET",    path: "/api/sessions/{sessionId}/messages" },
  load_session_messages_around_cmd:{ method: "GET",    path: "/api/sessions/{sessionId}/messages/around" },
  get_session_stream_state:        { method: "GET",    path: "/api/sessions/{sessionId}/stream-state" },
  delete_session_cmd:              { method: "DELETE", path: "/api/sessions/{sessionId}" },
  rename_session_cmd:              { method: "PATCH",  path: "/api/sessions/{sessionId}" },
  mark_session_read_cmd:           { method: "POST",   path: "/api/sessions/{sessionId}/read" },
  mark_session_read_batch_cmd:     { method: "POST",   path: "/api/sessions/read-batch" },
  mark_all_sessions_read_cmd:      { method: "POST",   path: "/api/sessions/read-all" },
  compact_context_now:             { method: "POST",   path: "/api/sessions/{sessionId}/compact" },
  write_export_file:               { method: "POST",   path: "/api/misc/write-export-file" },

  // -- Chat --
  chat:                            { method: "POST",   path: "/api/chat" },
  stop_chat:                       { method: "POST",   path: "/api/chat/stop" },
  respond_to_approval:             { method: "POST",   path: "/api/chat/approval" },
  save_attachment:                  { method: "POST",   path: "/api/chat/attachment" },

  // -- Providers --
  get_providers:                   { method: "GET",    path: "/api/providers" },
  add_provider:                    { method: "POST",   path: "/api/providers" },
  update_provider:                 { method: "PUT",    path: "/api/providers/{providerId}" },
  delete_provider:                 { method: "DELETE", path: "/api/providers/{providerId}" },
  reorder_providers:               { method: "POST",   path: "/api/providers/reorder" },
  test_provider:                   { method: "POST",   path: "/api/providers/test" },
  test_embedding:                  { method: "POST",   path: "/api/providers/test-embedding" },
  test_image_generate:             { method: "POST",   path: "/api/providers/test-image" },

  // -- Models --
  get_available_models:            { method: "GET",    path: "/api/models" },
  get_active_model:                { method: "GET",    path: "/api/models/active" },
  set_active_model:                { method: "POST",   path: "/api/models/active" },
  set_fallback_models:             { method: "POST",   path: "/api/models/fallback" },
  set_reasoning_effort:            { method: "POST",   path: "/api/models/reasoning-effort" },
  get_current_settings:            { method: "GET",    path: "/api/models/settings" },
  set_global_temperature:          { method: "POST",   path: "/api/models/temperature" },

  // -- Agents --
  list_agents:                     { method: "GET",    path: "/api/agents" },
  get_agent_config:                { method: "GET",    path: "/api/agents/{id}" },
  save_agent_config_cmd:           { method: "PUT",    path: "/api/agents/{id}" },
  delete_agent:                    { method: "DELETE", path: "/api/agents/{id}" },
  save_agent_markdown:             { method: "PUT",    path: "/api/agents/{id}/markdown" },
  render_persona_to_soul_md:       { method: "POST",   path: "/api/agents/{id}/persona/render-soul-md" },
  save_agent_memory_md:            { method: "PUT",    path: "/api/agents/{id}/memory-md" },
  dreaming_run_now:                { method: "POST",   path: "/api/dreaming/run" },
  dreaming_list_diaries:           { method: "GET",    path: "/api/dreaming/diaries" },
  dreaming_read_diary:             { method: "GET",    path: "/api/dreaming/diaries/{filename}" },
  dreaming_is_running:             { method: "GET",    path: "/api/dreaming/status" },
  scan_openclaw_agents:            { method: "GET",    path: "/api/agents/openclaw/scan" },
  import_openclaw_agents:          { method: "POST",   path: "/api/agents/openclaw/import" },

  // -- User config --
  get_user_config:                 { method: "GET",    path: "/api/config/user" },
  save_user_config:                { method: "PUT",    path: "/api/config/user" },

  // -- Memory --
  memory_search:                   { method: "POST",   path: "/api/memory/search" },
  memory_list:                     { method: "GET",    path: "/api/memory" },
  memory_add:                      { method: "POST",   path: "/api/memory" },
  memory_update:                   { method: "PUT",    path: "/api/memory/{id}" },
  memory_delete:                   { method: "DELETE", path: "/api/memory/{id}" },
  memory_toggle_pin:               { method: "POST",   path: "/api/memory/{id}/pin" },
  memory_delete_batch:             { method: "POST",   path: "/api/memory/delete-batch" },
  memory_reembed:                  { method: "POST",   path: "/api/memory/reembed" },
  memory_get_import_from_ai_prompt:{ method: "GET",    path: "/api/memory/import-from-ai-prompt" },
  save_global_memory_md:           { method: "PUT",    path: "/api/memory/global-md" },

  // -- Memory config --
  get_embedding_config:            { method: "GET",    path: "/api/config/embedding" },
  save_embedding_config:           { method: "PUT",    path: "/api/config/embedding" },
  get_embedding_presets:           { method: "GET",    path: "/api/config/embedding/presets" },
  get_embedding_cache_config:      { method: "GET",    path: "/api/config/embedding-cache" },
  save_embedding_cache_config:     { method: "PUT",    path: "/api/config/embedding-cache" },
  get_dedup_config:                { method: "GET",    path: "/api/config/dedup" },
  save_dedup_config:               { method: "PUT",    path: "/api/config/dedup" },
  get_hybrid_search_config:        { method: "GET",    path: "/api/config/hybrid-search" },
  save_hybrid_search_config:       { method: "PUT",    path: "/api/config/hybrid-search" },
  get_mmr_config:                  { method: "GET",    path: "/api/config/mmr" },
  save_mmr_config:                 { method: "PUT",    path: "/api/config/mmr" },
  get_multimodal_config:           { method: "GET",    path: "/api/config/multimodal" },
  save_multimodal_config:          { method: "PUT",    path: "/api/config/multimodal" },
  get_temporal_decay_config:       { method: "GET",    path: "/api/config/temporal-decay" },
  save_temporal_decay_config:      { method: "PUT",    path: "/api/config/temporal-decay" },
  get_extract_config:              { method: "GET",    path: "/api/config/extract" },
  save_extract_config:             { method: "PUT",    path: "/api/config/extract" },

  // -- Context compaction --
  get_compact_config:              { method: "GET",    path: "/api/config/compact" },
  save_compact_config:             { method: "PUT",    path: "/api/config/compact" },

  // -- Behavior awareness --
  get_awareness_config:        { method: "GET",    path: "/api/config/awareness" },
  save_awareness_config:       { method: "PUT",    path: "/api/config/awareness" },
  get_session_awareness_override: { method: "GET", path: "/api/sessions/{sessionId}/awareness-config" },
  set_session_awareness_override: { method: "PATCH", path: "/api/sessions/{sessionId}/awareness-config" },

  // -- Plan mode --
  get_plan_mode:                   { method: "GET",    path: "/api/plan/{sessionId}/mode" },
  set_plan_mode:                   { method: "POST",   path: "/api/plan/{sessionId}/mode" },
  get_plan_steps:                  { method: "GET",    path: "/api/plan/{sessionId}/steps" },
  update_plan_step_status:         { method: "POST",   path: "/api/plan/{sessionId}/steps/update" },
  get_plan_content:                { method: "GET",    path: "/api/plan/{sessionId}/content" },
  save_plan_content:               { method: "PUT",    path: "/api/plan/{sessionId}/content" },
  get_plan_file_path:              { method: "GET",    path: "/api/plan/{sessionId}/file-path" },
  get_plan_checkpoint:             { method: "GET",    path: "/api/plan/{sessionId}/checkpoint" },
  get_plan_versions:               { method: "GET",    path: "/api/plan/{sessionId}/versions" },
  load_plan_version_content:       { method: "POST",   path: "/api/plan/version/load" },
  restore_plan_version:            { method: "POST",   path: "/api/plan/{sessionId}/version/restore" },
  plan_rollback:                   { method: "POST",   path: "/api/plan/{sessionId}/rollback" },
  cancel_plan_subagent:            { method: "POST",   path: "/api/plan/{sessionId}/cancel" },
  respond_ask_user_question:       { method: "POST",   path: "/api/ask_user/respond" },
  get_pending_ask_user_group:      { method: "GET",    path: "/api/plan/{sessionId}/pending-ask-user" },
  set_plan_subagent:               { method: "POST",   path: "/api/config/plan-subagent" },
  get_plan_subagent:               { method: "GET",    path: "/api/config/plan-subagent" },
  set_ask_user_question_timeout:   { method: "POST",   path: "/api/config/ask-user-question-timeout" },
  get_ask_user_question_timeout:   { method: "GET",    path: "/api/config/ask-user-question-timeout" },

  // -- Cron --
  cron_list_jobs:                  { method: "GET",    path: "/api/cron/jobs" },
  cron_get_job:                    { method: "GET",    path: "/api/cron/jobs/{id}" },
  cron_create_job:                 { method: "POST",   path: "/api/cron/jobs" },
  cron_update_job:                 { method: "PUT",    path: "/api/cron/jobs/{id}" },
  cron_toggle_job:                 { method: "POST",   path: "/api/cron/jobs/{id}/toggle" },
  cron_delete_job:                 { method: "DELETE", path: "/api/cron/jobs/{id}" },
  cron_run_now:                    { method: "POST",   path: "/api/cron/jobs/{id}/run" },
  cron_get_run_logs:               { method: "GET",    path: "/api/cron/jobs/{jobId}/logs" },
  cron_get_calendar_events:        { method: "GET",    path: "/api/cron/calendar" },

  // -- Dashboard --
  dashboard_overview:              { method: "POST",   path: "/api/dashboard/overview" },
  dashboard_token_usage:           { method: "POST",   path: "/api/dashboard/token-usage" },
  dashboard_tool_usage:            { method: "POST",   path: "/api/dashboard/tool-usage" },
  dashboard_sessions:              { method: "POST",   path: "/api/dashboard/sessions" },
  dashboard_errors:                { method: "POST",   path: "/api/dashboard/errors" },
  dashboard_tasks:                 { method: "POST",   path: "/api/dashboard/tasks" },
  dashboard_system_metrics:        { method: "GET",    path: "/api/dashboard/system-metrics" },
  dashboard_session_list:          { method: "POST",   path: "/api/dashboard/session-list" },
  dashboard_message_list:          { method: "POST",   path: "/api/dashboard/message-list" },
  dashboard_tool_call_list:        { method: "POST",   path: "/api/dashboard/tool-call-list" },
  dashboard_error_list:            { method: "POST",   path: "/api/dashboard/error-list" },
  dashboard_agent_list:            { method: "POST",   path: "/api/dashboard/agent-list" },

  // -- Async / Deferred tools + Memory selection --
  get_async_tools_config:          { method: "GET",    path: "/api/config/async-tools" },
  save_async_tools_config:         { method: "PUT",    path: "/api/config/async-tools" },
  get_deferred_tools_config:       { method: "GET",    path: "/api/config/deferred-tools" },
  save_deferred_tools_config:      { method: "PUT",    path: "/api/config/deferred-tools" },
  get_memory_selection_config:     { method: "GET",    path: "/api/config/memory-selection" },
  save_memory_selection_config:    { method: "PUT",    path: "/api/config/memory-selection" },
  get_memory_budget_config:        { method: "GET",    path: "/api/config/memory-budget" },
  save_memory_budget_config:       { method: "PUT",    path: "/api/config/memory-budget" },

  // -- Recap --
  get_recap_config:                { method: "GET",    path: "/api/config/recap" },
  save_recap_config:               { method: "PUT",    path: "/api/config/recap" },
  recap_generate:                  { method: "POST",   path: "/api/recap/generate" },
  recap_list_reports:              { method: "POST",   path: "/api/recap/reports" },
  recap_get_report:                { method: "GET",    path: "/api/recap/reports/{id}" },
  recap_delete_report:             { method: "DELETE", path: "/api/recap/reports/{id}" },
  recap_export_html:               { method: "POST",   path: "/api/recap/reports/{id}/export" },

  // -- Logging --
  query_logs_cmd:                  { method: "POST",   path: "/api/logs/query" },
  frontend_log:                    { method: "POST",   path: "/api/logs/frontend" },
  frontend_log_batch:              { method: "POST",   path: "/api/logs/frontend-batch" },
  get_log_stats_cmd:               { method: "GET",    path: "/api/logs/stats" },
  get_log_config_cmd:              { method: "GET",    path: "/api/logs/config" },
  save_log_config_cmd:             { method: "PUT",    path: "/api/logs/config" },
  list_log_files_cmd:              { method: "GET",    path: "/api/logs/files" },
  read_log_file_cmd:               { method: "GET",    path: "/api/logs/file" },
  get_log_file_path_cmd:           { method: "GET",    path: "/api/logs/file-path" },
  export_logs_cmd:                 { method: "POST",   path: "/api/logs/export" },
  clear_logs_cmd:                  { method: "POST",   path: "/api/logs/clear" },

  // -- Notifications --
  get_notification_config:         { method: "GET",    path: "/api/config/notification" },
  save_notification_config:        { method: "PUT",    path: "/api/config/notification" },

  // -- Server --
  get_server_config:               { method: "GET",    path: "/api/config/server" },
  save_server_config:              { method: "PUT",    path: "/api/config/server" },

  // -- Proxy --
  get_proxy_config:                { method: "GET",    path: "/api/config/proxy" },
  save_proxy_config:               { method: "PUT",    path: "/api/config/proxy" },

  // -- Shortcuts --
  get_shortcut_config:             { method: "GET",    path: "/api/config/shortcuts" },
  save_shortcut_config:            { method: "PUT",    path: "/api/config/shortcuts" },
  set_shortcuts_paused:            { method: "POST",   path: "/api/config/shortcuts/pause" },

  // -- Sandbox --
  get_sandbox_config:              { method: "GET",    path: "/api/config/sandbox" },
  set_sandbox_config:              { method: "PUT",    path: "/api/config/sandbox" },

  // -- Canvas --
  get_canvas_config:               { method: "GET",    path: "/api/config/canvas" },
  save_canvas_config:              { method: "PUT",    path: "/api/config/canvas" },
  canvas_submit_snapshot:          { method: "POST",   path: "/api/canvas/snapshot/{requestId}" },
  canvas_submit_eval_result:       { method: "POST",   path: "/api/canvas/eval/{requestId}" },
  show_canvas_panel:               { method: "POST",   path: "/api/canvas/show" },

  // -- Image generation --
  get_image_generate_config:       { method: "GET",    path: "/api/config/image-generate" },
  save_image_generate_config:      { method: "PUT",    path: "/api/config/image-generate" },

  // -- Web search --
  get_web_search_config:           { method: "GET",    path: "/api/config/web-search" },
  save_web_search_config:          { method: "PUT",    path: "/api/config/web-search" },

  // -- Web fetch --
  get_web_fetch_config:            { method: "GET",    path: "/api/config/web-fetch" },
  save_web_fetch_config:           { method: "PUT",    path: "/api/config/web-fetch" },

  // -- SSRF policy --
  get_ssrf_config:                 { method: "GET",    path: "/api/config/ssrf" },
  save_ssrf_config:                { method: "PUT",    path: "/api/config/ssrf" },

  // -- SearXNG Docker --
  searxng_docker_status:           { method: "GET",    path: "/api/searxng/status" },
  searxng_docker_deploy:           { method: "POST",   path: "/api/searxng/deploy" },
  searxng_docker_start:            { method: "POST",   path: "/api/searxng/start" },
  searxng_docker_stop:             { method: "POST",   path: "/api/searxng/stop" },
  searxng_docker_remove:           { method: "DELETE", path: "/api/searxng" },

  // -- Skills --
  get_skills:                      { method: "GET",    path: "/api/skills" },
  get_skill_detail:                { method: "GET",    path: "/api/skills/{name}" },
  toggle_skill:                    { method: "POST",   path: "/api/skills/{name}/toggle" },
  get_extra_skills_dirs:           { method: "GET",    path: "/api/skills/extra-dirs" },
  add_extra_skills_dir:            { method: "POST",   path: "/api/skills/extra-dirs" },
  remove_extra_skills_dir:         { method: "DELETE", path: "/api/skills/extra-dirs" },
  get_skill_env:                   { method: "GET",    path: "/api/skills/{name}/env" },
  set_skill_env_var:               { method: "POST",   path: "/api/skills/{skill}/env" },
  remove_skill_env_var:            { method: "DELETE", path: "/api/skills/{skill}/env" },
  get_skills_env_status:           { method: "GET",    path: "/api/skills/env-status" },
  get_skills_status:               { method: "GET",    path: "/api/skills/status" },
  get_skill_env_check:             { method: "GET",    path: "/api/skills/env-check" },
  set_skill_env_check:             { method: "PUT",    path: "/api/skills/env-check" },
  list_draft_skills:               { method: "GET",    path: "/api/skills/drafts" },
  activate_draft_skill:            { method: "POST",   path: "/api/skills/{name}/activate" },
  discard_draft_skill:             { method: "DELETE", path: "/api/skills/{name}/draft" },
  trigger_skill_review_now:        { method: "POST",   path: "/api/skills/review/run" },
  dashboard_learning_overview:     { method: "POST",   path: "/api/dashboard/learning/overview" },
  dashboard_learning_timeline:     { method: "POST",   path: "/api/dashboard/learning/timeline" },
  dashboard_top_skills:            { method: "POST",   path: "/api/dashboard/learning/top-skills" },
  dashboard_recall_stats:          { method: "POST",   path: "/api/dashboard/learning/recall-stats" },

  // -- Slash commands --
  list_slash_commands:             { method: "GET",    path: "/api/slash-commands" },
  execute_slash_command:           { method: "POST",   path: "/api/slash-commands/execute" },
  is_slash_command:                { method: "POST",   path: "/api/slash-commands/is-slash" },

  // -- Channels --
  channel_list_plugins:            { method: "GET",    path: "/api/channel/plugins" },
  channel_list_accounts:           { method: "GET",    path: "/api/channel/accounts" },
  channel_add_account:             { method: "POST",   path: "/api/channel/accounts" },
  channel_update_account:          { method: "PUT",    path: "/api/channel/accounts/{accountId}" },
  channel_remove_account:          { method: "DELETE", path: "/api/channel/accounts/{accountId}" },
  channel_start_account:           { method: "POST",   path: "/api/channel/accounts/{accountId}/start" },
  channel_stop_account:            { method: "POST",   path: "/api/channel/accounts/{accountId}/stop" },
  channel_health:                  { method: "GET",    path: "/api/channel/accounts/{accountId}/health" },
  channel_health_all:              { method: "GET",    path: "/api/channel/health" },
  channel_validate_credentials:    { method: "POST",   path: "/api/channel/validate" },
  channel_send_test_message:       { method: "POST",   path: "/api/channel/accounts/{accountId}/test-message" },
  channel_list_sessions:           { method: "GET",    path: "/api/channel/sessions" },
  channel_wechat_start_login:      { method: "POST",   path: "/api/channel/wechat/login/start" },
  channel_wechat_wait_login:       { method: "POST",   path: "/api/channel/wechat/login/wait" },

  // -- Subagent --
  list_subagent_runs:              { method: "GET",    path: "/api/subagent/runs" },
  get_subagent_run:                { method: "GET",    path: "/api/subagent/runs/{runId}" },
  get_subagent_runs_batch:         { method: "POST",   path: "/api/subagent/runs/batch" },
  kill_subagent:                   { method: "POST",   path: "/api/subagent/runs/{runId}/kill" },

  // -- Team --
  list_teams:                      { method: "GET",    path: "/api/teams" },
  create_team:                     { method: "POST",   path: "/api/teams" },
  get_team:                        { method: "GET",    path: "/api/teams/{teamId}" },
  get_team_members:                { method: "GET",    path: "/api/teams/{teamId}/members" },
  get_team_messages:               { method: "GET",    path: "/api/teams/{teamId}/messages" },
  get_team_tasks:                  { method: "GET",    path: "/api/teams/{teamId}/tasks" },
  send_user_team_message:          { method: "POST",   path: "/api/teams/{teamId}/messages" },
  pause_team:                      { method: "POST",   path: "/api/teams/{teamId}/pause" },
  resume_team:                     { method: "POST",   path: "/api/teams/{teamId}/resume" },
  dissolve_team:                   { method: "POST",   path: "/api/teams/{teamId}/dissolve" },
  list_team_templates:             { method: "GET",    path: "/api/team-templates" },
  save_team_template:              { method: "POST",   path: "/api/team-templates" },
  delete_team_template:            { method: "DELETE", path: "/api/team-templates/{templateId}" },

  // -- Weather --
  geocode_search:                  { method: "GET",    path: "/api/weather/geocode" },
  preview_weather:                 { method: "POST",   path: "/api/weather/preview" },
  detect_location:                 { method: "GET",    path: "/api/weather/detect-location" },
  get_current_weather:             { method: "GET",    path: "/api/weather/current" },
  refresh_weather:                 { method: "POST",   path: "/api/weather/refresh" },

  // -- URL preview --
  fetch_url_preview:               { method: "POST",   path: "/api/url-preview" },
  fetch_url_previews:              { method: "POST",   path: "/api/url-preview/batch" },

  // -- Embedded browser --
  browser_get_status:              { method: "GET",    path: "/api/browser/status" },
  browser_list_profiles:           { method: "GET",    path: "/api/browser/profiles" },
  browser_create_profile:          { method: "POST",   path: "/api/browser/profiles" },
  browser_delete_profile:          { method: "DELETE", path: "/api/browser/profiles/{name}" },
  browser_launch:                  { method: "POST",   path: "/api/browser/launch" },
  browser_connect:                 { method: "POST",   path: "/api/browser/connect" },
  browser_disconnect:              { method: "POST",   path: "/api/browser/disconnect" },

  // -- Theme / Language / UI --
  get_theme:                       { method: "GET",    path: "/api/config/theme" },
  set_theme:                       { method: "POST",   path: "/api/config/theme" },
  set_window_theme:                { method: "POST",   path: "/api/config/window-theme" },
  get_language:                    { method: "GET",    path: "/api/config/language" },
  set_language:                    { method: "POST",   path: "/api/config/language" },
  get_ui_effects_enabled:          { method: "GET",    path: "/api/config/ui-effects" },
  set_ui_effects_enabled:          { method: "POST",   path: "/api/config/ui-effects" },
  get_tool_call_narration_enabled: { method: "GET",    path: "/api/config/tool-call-narration" },
  set_tool_call_narration_enabled: { method: "POST",   path: "/api/config/tool-call-narration" },
  get_autostart_enabled:           { method: "GET",    path: "/api/config/autostart" },
  set_autostart_enabled:           { method: "POST",   path: "/api/config/autostart" },

  // -- Tools --
  get_tool_timeout:                { method: "GET",    path: "/api/config/tool-timeout" },
  set_tool_timeout:                { method: "POST",   path: "/api/config/tool-timeout" },
  get_approval_timeout:            { method: "GET",    path: "/api/config/approval-timeout" },
  set_approval_timeout:            { method: "POST",   path: "/api/config/approval-timeout" },
  get_approval_timeout_action:     { method: "GET",    path: "/api/config/approval-timeout-action" },
  set_approval_timeout_action:     { method: "POST",   path: "/api/config/approval-timeout-action" },
  get_tool_result_disk_threshold:  { method: "GET",    path: "/api/config/tool-result-threshold" },
  set_tool_result_disk_threshold:  { method: "POST",   path: "/api/config/tool-result-threshold" },
  get_tool_limits:                 { method: "GET",    path: "/api/config/tool-limits" },
  set_tool_limits:                 { method: "POST",   path: "/api/config/tool-limits" },

  // -- Crash / Recovery --
  get_crash_recovery_info:         { method: "GET",    path: "/api/crash/recovery-info" },
  get_crash_history:               { method: "GET",    path: "/api/crash/history" },
  clear_crash_history:             { method: "DELETE", path: "/api/crash/history" },
  list_backups_cmd:                { method: "GET",    path: "/api/crash/backups" },
  create_backup_cmd:               { method: "POST",   path: "/api/crash/backups" },
  restore_backup_cmd:              { method: "POST",   path: "/api/crash/backups/restore" },
  list_settings_backups_cmd:       { method: "GET",    path: "/api/settings/backups" },
  restore_settings_backup_cmd:     { method: "POST",   path: "/api/settings/backups/restore" },
  get_guardian_enabled:            { method: "GET",    path: "/api/crash/guardian" },
  set_guardian_enabled:            { method: "PUT",    path: "/api/crash/guardian" },
  request_app_restart:             { method: "POST",   path: "/api/system/restart" },

  // -- Developer (desktop-only, HTTP not implemented) --
  dev_clear_sessions:              { method: "POST",   path: "/api/dev/clear-sessions" },
  dev_clear_cron:                  { method: "POST",   path: "/api/dev/clear-cron" },
  dev_clear_memory:                { method: "POST",   path: "/api/dev/clear-memory" },
  dev_reset_config:                { method: "POST",   path: "/api/dev/reset-config" },
  dev_clear_all:                   { method: "POST",   path: "/api/dev/clear-all" },

  // -- ACP --
  acp_list_backends:               { method: "GET",    path: "/api/acp/backends" },
  acp_health_check:                { method: "GET",    path: "/api/acp/backends" },
  acp_refresh_backends:            { method: "POST",   path: "/api/acp/refresh" },
  acp_list_runs:                   { method: "GET",    path: "/api/acp/runs" },
  acp_kill_run:                    { method: "POST",   path: "/api/acp/runs/{runId}/kill" },
  acp_get_run_result:              { method: "GET",    path: "/api/acp/runs/{runId}/result" },
  acp_get_config:                  { method: "GET",    path: "/api/acp/config" },
  acp_set_config:                  { method: "PUT",    path: "/api/acp/config" },

  // -- Auth --
  start_codex_auth:                { method: "POST",   path: "/api/auth/codex/start" },
  finalize_codex_auth:             { method: "POST",   path: "/api/auth/codex/finalize" },

  // -- Desktop-only (no-op in web mode) --
  open_url:                        { method: "POST",   path: "/api/desktop/open-url" },
  open_directory:                  { method: "POST",   path: "/api/desktop/open-directory" },
  reveal_in_folder:                { method: "POST",   path: "/api/desktop/reveal-in-folder" },
  get_system_prompt:               { method: "POST",   path: "/api/system-prompt" },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Build the final URL by replacing `{param}` placeholders in the path
 * template with values from `args`, removing consumed keys.
 */
function buildUrl(
  baseUrl: string,
  def: EndpointDef,
  args: Record<string, unknown> | undefined,
): { url: string; remainingArgs: Record<string, unknown> } {
  const remaining = args ? { ...args } : {};
  let path = def.path;

  const paramRegex = /\{(\w+)\}/g;
  let match: RegExpExecArray | null;
  while ((match = paramRegex.exec(def.path)) !== null) {
    const key = match[1];
    const value = remaining[key];
    if (value === undefined || value === null) {
      throw new Error(
        `Missing required path parameter "${key}" for endpoint ${def.method} ${def.path}`,
      );
    }
    path = path.replace(`{${key}}`, encodeURIComponent(String(value)));
    delete remaining[key];
  }

  return { url: `${baseUrl}${path}`, remainingArgs: remaining };
}

/**
 * Append remaining args as query string parameters for GET / DELETE requests.
 */
function appendQueryParams(url: string, params: Record<string, unknown>): string {
  const entries = Object.entries(params).filter(
    ([, v]) => v !== undefined && v !== null,
  );
  if (entries.length === 0) return url;

  const qs = entries
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
    .join("&");
  return url.includes("?") ? `${url}&${qs}` : `${url}?${qs}`;
}

// ---------------------------------------------------------------------------
// WebSocket reconnection helper for the global events channel
// ---------------------------------------------------------------------------

interface EventSubscription {
  eventName: string;
  handler: (payload: unknown) => void;
}

// ---------------------------------------------------------------------------
// HttpTransport
// ---------------------------------------------------------------------------

export class HttpTransport implements Transport {
  private readonly baseUrl: string;
  private apiKey: string | null;

  /** Persistent WebSocket for backend-pushed events. */
  private eventWs: WebSocket | null = null;
  private eventWsConnecting = false;
  private eventSubscriptions: EventSubscription[] = [];

  /** Reconnection state. */
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempts = 0;
  private readonly maxReconnectDelay = 30_000; // 30 s cap

  constructor(baseUrl: string, apiKey?: string | null) {
    // Strip trailing slash.
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.apiKey = apiKey ?? null;
  }

  /** Update the API key at runtime. */
  setApiKey(key: string | null): void {
    this.apiKey = key;
  }

  /** Build a WebSocket URL with token query param if API key is set. */
  private wsUrl(path: string): string {
    const wsBase = this.baseUrl.replace(/^http/, "ws");
    const url = `${wsBase}${path}`;
    return this.apiKey ? `${url}${url.includes("?") ? "&" : "?"}token=${encodeURIComponent(this.apiKey)}` : url;
  }

  // ----- prepareFileData -----

  prepareFileData(buffer: ArrayBuffer, mimeType: string): Blob {
    return new Blob([buffer], { type: mimeType });
  }

  // ----- call -----

  async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    // --- Special cases: binary uploads use multipart/form-data ---
    if (command === "save_attachment" && args) {
      return this.uploadMultipart<T>("/api/chat/attachment", args);
    }
    if (command === "upload_project_file_cmd" && args) {
      const projectId = args.projectId as string;
      const rest = { ...args };
      delete rest.projectId;
      return this.uploadMultipart<T>(`/api/projects/${encodeURIComponent(projectId)}/files`, rest);
    }

    const def = COMMAND_MAP[command];
    if (!def) {
      throw new Error(
        `[HttpTransport] No REST mapping for command "${command}". ` +
          "Add it to COMMAND_MAP in transport-http.ts.",
      );
    }

    const { url: rawUrl, remainingArgs } = buildUrl(this.baseUrl, def, args);

    const isBodyMethod = def.method === "POST" || def.method === "PUT" || def.method === "PATCH";
    const url = isBodyMethod ? rawUrl : appendQueryParams(rawUrl, remainingArgs);

    const headers: Record<string, string> = {};
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }
    let body: string | undefined;

    if (isBodyMethod) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(remainingArgs);
    }

    const response = await fetch(url, {
      method: def.method,
      headers,
      body,
    });

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(
        `[HttpTransport] ${def.method} ${url} returned ${response.status}: ${text}`,
      );
    }

    // Some endpoints return no body (204, or empty 200).
    const contentType = response.headers.get("content-type") ?? "";
    if (
      response.status === 204 ||
      !contentType.includes("application/json")
    ) {
      return undefined as unknown as T;
    }

    return (await response.json()) as T;
  }

  /**
   * Upload a file using multipart/form-data instead of JSON.
   * Avoids the ~4× blow-up of encoding raw bytes as a JSON number array.
   *
   * The `data` arg may be a `Blob` (zero-copy) or a legacy `number[]`.
   * All other args are sent as text form fields.
   */
  private async uploadMultipart<T>(path: string, args: Record<string, unknown>): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const form = new FormData();

    const rawData = args.data;
    const fileName = (args.fileName as string) ?? "attachment";
    const mimeType = (args.mimeType as string) ?? "application/octet-stream";

    let blob: Blob;
    if (rawData instanceof Blob) {
      blob = rawData;
    } else if (Array.isArray(rawData)) {
      // Legacy fallback: number[] → binary Blob
      blob = new Blob([new Uint8Array(rawData)], { type: mimeType });
    } else {
      throw new Error("[HttpTransport] multipart upload: data must be a Blob or number[]");
    }

    form.append("file", blob, fileName);
    // Forward remaining string args as text fields.
    for (const [k, v] of Object.entries(args)) {
      if (k === "data") continue;
      if (v !== undefined && v !== null) form.append(k, String(v));
    }

    const headers: Record<string, string> = {};
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }
    // Do NOT set Content-Type — browser sets multipart boundary automatically.

    const response = await fetch(url, { method: "POST", headers, body: form });

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(`[HttpTransport] POST ${url} returned ${response.status}: ${text}`);
    }

    return (await response.json()) as T;
  }

  // ----- openChatStream -----

  openChatStream(
    sessionId: string | null,
    onEvent: (event: string) => void,
  ): ChatStream {
    const path = sessionId
      ? `/ws/chat/${encodeURIComponent(sessionId)}`
      : "/ws/chat";
    const ws = new WebSocket(this.wsUrl(path));

    ws.onmessage = (ev) => {
      if (typeof ev.data === "string") {
        onEvent(ev.data);
      }
    };

    ws.onerror = (err) => {
      console.error("[HttpTransport] Chat WebSocket error", err);
    };

    return {
      close() {
        ws.close();
      },
    };
  }

  // ----- media -----

  resolveMediaUrl(item: MediaItem): string | null {
    const url = item.url;
    if (!url) return null;
    if (url.startsWith("http://") || url.startsWith("https://")) return url;
    // The HTTP sink has already stamped `?token=` onto logical
    // `/api/attachments/...` URLs; we only prepend the base.
    if (url.startsWith("/")) return `${this.baseUrl}${url}`;
    // Absolute filesystem path — not reachable from a browser.
    return null;
  }

  async openMedia(item: MediaItem): Promise<void> {
    const href = this.resolveMediaUrl(item);
    if (!href) return;
    // Transient anchor click so the browser honors the server's
    // Content-Disposition (inline preview vs download prompt).
    const a = document.createElement("a");
    a.href = href;
    a.download = item.name || "";
    a.rel = "noopener";
    a.target = "_blank";
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  async revealMedia(_item: MediaItem): Promise<void> {
    // No-op in HTTP mode — there's no OS file manager on the client side.
  }

  supportsLocalFileOps(): boolean {
    return false;
  }

  // ----- listen -----

  listen(eventName: string, handler: (payload: unknown) => void): () => void {
    const sub: EventSubscription = { eventName, handler };
    this.eventSubscriptions.push(sub);
    this.ensureEventWs();

    return () => {
      const idx = this.eventSubscriptions.indexOf(sub);
      if (idx !== -1) this.eventSubscriptions.splice(idx, 1);

      // Disconnect the events WebSocket when nobody is listening.
      if (this.eventSubscriptions.length === 0) {
        this.teardownEventWs();
      }
    };
  }

  // ----- Events WebSocket internals -----

  private ensureEventWs(): void {
    if (this.eventWs || this.eventWsConnecting) return;
    this.eventWsConnecting = true;

    const ws = new WebSocket(this.wsUrl("/ws/events"));

    ws.onopen = () => {
      this.eventWsConnecting = false;
      this.eventWs = ws;
      this.reconnectAttempts = 0;
    };

    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return;
      try {
        const envelope = JSON.parse(ev.data) as {
          name: string;
          payload: unknown;
        };
        for (const sub of this.eventSubscriptions) {
          if (sub.eventName === envelope.name) {
            sub.handler(envelope.payload);
          }
        }
      } catch {
        // Ignore malformed messages.
      }
    };

    ws.onerror = () => {
      // onclose will handle reconnection.
    };

    ws.onclose = () => {
      this.eventWs = null;
      this.eventWsConnecting = false;

      // Reconnect only if there are active subscribers.
      if (this.eventSubscriptions.length > 0) {
        this.scheduleReconnect();
      }
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return;

    // Exponential back-off: 1s, 2s, 4s, 8s, ... capped at maxReconnectDelay.
    const delay = Math.min(
      1000 * Math.pow(2, this.reconnectAttempts),
      this.maxReconnectDelay,
    );
    this.reconnectAttempts++;

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.ensureEventWs();
    }, delay);
  }

  private teardownEventWs(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.reconnectAttempts = 0;

    if (this.eventWs) {
      this.eventWs.close();
      this.eventWs = null;
    }
    this.eventWsConnecting = false;
  }
}
