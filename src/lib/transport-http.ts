/**
 * HTTP / WebSocket transport implementation.
 *
 * Used when the frontend runs outside of Tauri (standalone web mode).
 * Maps Tauri command names to REST endpoints and uses WebSockets for
 * streaming chat and backend events.
 */

import type {
  Transport,
  ChatStartArgs,
  PickedImage,
  DirListing,
  FileSearchResponse,
  ExportSessionArgs,
  ExportSessionResult,
  ExtractedContent,
  FileTextContent,
  ProjectFsScope,
  UploadResult,
  SaveResult,
  SessionArtifacts,
  WorkspaceEnvironmentSnapshot,
} from "@/lib/transport";
import type { FileChangesMetadata, MediaItem } from "@/types/chat";
import { dispatchAuthRequired, setStoredApiKey } from "@/lib/api-key-storage";
import { downloadBlob } from "@/lib/fileDownload";

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
  reorder_projects_cmd:            { method: "POST",   path: "/api/projects/reorder" },
  list_project_sessions_cmd:       { method: "GET",    path: "/api/projects/{id}/sessions" },
  mark_project_sessions_read_cmd:  { method: "POST",   path: "/api/projects/{projectId}/read" },
  move_session_to_project_cmd:     { method: "PATCH",  path: "/api/sessions/{sessionId}/project" },
  list_project_memories_cmd:       { method: "GET",    path: "/api/projects/{id}/memories" },

  // -- Knowledge Base (Knowledge Space) --
  list_kbs_cmd:                    { method: "GET",    path: "/api/knowledge" },
  get_kb_cmd:                      { method: "GET",    path: "/api/knowledge/{id}" },
  create_kb_cmd:                   { method: "POST",   path: "/api/knowledge" },
  update_kb_cmd:                   { method: "PATCH",  path: "/api/knowledge/{id}" },
  delete_kb_cmd:                   { method: "DELETE", path: "/api/knowledge/{id}" },
  reindex_kb_cmd:                  { method: "POST",   path: "/api/knowledge/{id}/reindex" },
  kb_source_import_cmd:            { method: "POST",   path: "/api/knowledge/{kbId}/sources" },
  kb_source_import_browser_cmd:    { method: "POST",   path: "/api/knowledge/{kbId}/sources/browser" },
  kb_source_import_session_attachment_cmd:{ method: "POST",   path: "/api/knowledge/{kbId}/sources/session-attachment" },
  kb_source_import_batch_cmd:      { method: "POST",   path: "/api/knowledge/{kbId}/sources/batch" },
  kb_source_list_cmd:              { method: "GET",    path: "/api/knowledge/{kbId}/sources" },
  kb_source_import_runs_list_cmd:  { method: "GET",    path: "/api/knowledge/{kbId}/sources/import-runs" },
  kb_source_import_run_detail_cmd: { method: "GET",    path: "/api/knowledge/{kbId}/sources/import-runs/{runId}" },
  kb_source_import_retry_failed_cmd:{ method: "POST",  path: "/api/knowledge/{kbId}/sources/import-runs/{runId}/retry-failed" },
  kb_source_ocr_pages_cmd:         { method: "GET",    path: "/api/knowledge/{kbId}/sources/{sourceId}/ocr-pages" },
  kb_source_ocr_retry_cmd:         { method: "POST",   path: "/api/knowledge/{kbId}/sources/{sourceId}/ocr-retry" },
  kb_source_similarity_groups_cmd: { method: "GET",    path: "/api/knowledge/{kbId}/sources/similar" },
  kb_source_similarity_dismiss_cmd:{ method: "POST",   path: "/api/knowledge/{kbId}/sources/similar/dismiss" },
  kb_source_similarity_resolve_cmd:{ method: "POST",   path: "/api/knowledge/{kbId}/sources/similar/resolve" },
  kb_source_sync_external_raw_cmd: { method: "POST",   path: "/api/knowledge/{kbId}/sources/sync-external-raw" },
  kb_source_read_cmd:              { method: "GET",    path: "/api/knowledge/{kbId}/sources/{sourceId}" },
  kb_source_asset_link_cmd:         { method: "GET",    path: "/api/knowledge/{kbId}/sources/{sourceId}/assets/{kind}/link" },
  kb_source_refresh_cmd:           { method: "POST",   path: "/api/knowledge/{kbId}/sources/{sourceId}/refresh" },
  kb_source_versions_cmd:          { method: "GET",    path: "/api/knowledge/{kbId}/sources/{sourceId}/versions" },
  kb_source_diff_cmd:              { method: "GET",    path: "/api/knowledge/{kbId}/sources/{sourceId}/diff" },
  kb_source_reextract_cmd:         { method: "POST",   path: "/api/knowledge/{kbId}/sources/{sourceId}/reextract" },
  kb_source_delete_cmd:            { method: "DELETE", path: "/api/knowledge/{kbId}/sources/{sourceId}" },
  kb_compile_start_cmd:            { method: "POST",   path: "/api/knowledge/{kbId}/compile-runs" },
  kb_compile_status_cmd:           { method: "GET",    path: "/api/knowledge/{kbId}/compile-runs/{runId}" },
  kb_compile_runs_list_cmd:        { method: "GET",    path: "/api/knowledge/{kbId}/compile-runs" },
  kb_compile_proposals_list_cmd:   { method: "GET",    path: "/api/knowledge/{kbId}/compile-proposals" },
  kb_compile_proposal_approve_cmd: { method: "POST",   path: "/api/knowledge/{kbId}/compile-proposals/{id}/approve" },
  kb_compile_proposal_reject_cmd:  { method: "POST",   path: "/api/knowledge/{kbId}/compile-proposals/{id}/reject" },
  kb_compile_run_cancel_cmd:       { method: "POST",   path: "/api/knowledge/{kbId}/compile-runs/{runId}/cancel" },
  kb_query_file_cmd:               { method: "POST",   path: "/api/knowledge/{kbId}/query-file" },
  kb_schema_profile_cmd:           { method: "GET",    path: "/api/knowledge/{kbId}/schema-profile" },
  kb_schema_issues_cmd:            { method: "GET",    path: "/api/knowledge/{kbId}/schema-issues" },
  kb_note_source_refs_cmd:         { method: "GET",    path: "/api/knowledge/{kbId}/note/source-refs" },
  kb_evidence_coverage_cmd:        { method: "GET",    path: "/api/knowledge/{kbId}/evidence/coverage" },
  kb_evidence_source_claims_cmd:   { method: "GET",    path: "/api/knowledge/{kbId}/evidence/sources/{sourceId}/claims" },
  kb_evidence_rebuild_cmd:         { method: "POST",   path: "/api/knowledge/{kbId}/evidence/rebuild" },
  knowledge_agent_search_cmd:      { method: "POST",   path: "/api/knowledge/agent/search" },
  knowledge_agent_read_cmd:        { method: "POST",   path: "/api/knowledge/agent/read" },
  knowledge_agent_expand_cmd:      { method: "POST",   path: "/api/knowledge/agent/expand" },
  knowledge_agent_sources_cmd:     { method: "POST",   path: "/api/knowledge/agent/sources" },
  knowledge_agent_compile_propose_cmd: { method: "POST", path: "/api/knowledge/agent/compile/propose" },
  attach_session_kb_cmd:           { method: "POST",   path: "/api/knowledge/attach" },
  detach_session_kb_cmd:           { method: "POST",   path: "/api/knowledge/detach" },
  attach_project_kb_cmd:           { method: "POST",   path: "/api/knowledge/attach" },
  detach_project_kb_cmd:           { method: "POST",   path: "/api/knowledge/detach" },
  list_session_kbs_cmd:            { method: "GET",    path: "/api/knowledge/attachments" },
  list_project_kbs_cmd:            { method: "GET",    path: "/api/knowledge/project-attachments" },
  list_kb_notes_cmd:               { method: "GET",    path: "/api/knowledge/{kbId}/notes" },
  kb_note_read_cmd:                { method: "GET",    path: "/api/knowledge/{kbId}/note" },
  kb_note_save_cmd:                { method: "PUT",    path: "/api/knowledge/{kbId}/note" },
  kb_note_delete_cmd:              { method: "DELETE", path: "/api/knowledge/{kbId}/note" },
  kb_note_rename_cmd:              { method: "POST",   path: "/api/knowledge/{kbId}/note/rename" },
  kb_list_dirs_cmd:                { method: "GET",    path: "/api/knowledge/{kbId}/dirs" },
  kb_list_tags_cmd:                { method: "GET",    path: "/api/knowledge/{kbId}/tags" },
  knowledge_embedding_get_cmd:     { method: "GET",    path: "/api/knowledge/embedding" },
  knowledge_embedding_set_default_cmd: { method: "POST", path: "/api/knowledge/embedding/set-default" },
  knowledge_embedding_disable_cmd: { method: "POST",   path: "/api/knowledge/embedding/disable" },
  knowledge_embedding_rebuild_cmd: { method: "POST",   path: "/api/knowledge/embedding/rebuild" },
  knowledge_chunk_get_cmd:         { method: "GET",    path: "/api/knowledge/chunk" },
  knowledge_chunk_set_cmd:         { method: "POST",   path: "/api/knowledge/chunk" },
  knowledge_search_config_get_cmd: { method: "GET",    path: "/api/knowledge/search-config" },
  knowledge_search_config_set_cmd: { method: "POST",   path: "/api/knowledge/search-config" },
  reindex_note_cmd:                { method: "POST",   path: "/api/knowledge/{kbId}/note/reindex" },
  reindex_dir_cmd:                 { method: "POST",   path: "/api/knowledge/{kbId}/dir/reindex" },
  list_referenceable_notes_cmd:    { method: "POST",   path: "/api/knowledge/referenceable-notes" },
  kb_mkdir_cmd:                    { method: "POST",   path: "/api/knowledge/{kbId}/dir" },
  kb_rename_dir_cmd:               { method: "POST",   path: "/api/knowledge/{kbId}/dir/rename" },
  kb_delete_dir_cmd:               { method: "DELETE", path: "/api/knowledge/{kbId}/dir" },
  kb_backlinks_cmd:                { method: "GET",    path: "/api/knowledge/{kbId}/backlinks" },
  kb_broken_links_cmd:             { method: "GET",    path: "/api/knowledge/{kbId}/broken-links" },
  kb_orphans_cmd:                  { method: "GET",    path: "/api/knowledge/{kbId}/orphans" },
  kb_graph_cmd:                    { method: "GET",    path: "/api/knowledge/{kbId}/graph" },
  kb_graph_layout_get_cmd:         { method: "GET",    path: "/api/knowledge/{kbId}/graph/layout" },
  kb_graph_layout_save_cmd:        { method: "POST",   path: "/api/knowledge/{kbId}/graph/layout" },
  kb_chat_thread_get_cmd:          { method: "GET",    path: "/api/knowledge/{kbId}/chat/thread" },
  kb_chat_threads_list_cmd:        { method: "GET",    path: "/api/knowledge/{kbId}/chat/threads" },
  kb_ai_rewrite_cmd:               { method: "POST",   path: "/api/knowledge/ai/rewrite" },
  kb_rewrite_log_cmd:              { method: "POST",   path: "/api/knowledge/rewrite/log" },
  kb_maintenance_run_cmd:          { method: "POST",   path: "/api/knowledge/maintenance/run" },
  kb_maintenance_status_cmd:       { method: "GET",    path: "/api/knowledge/maintenance/status" },
  kb_maintenance_list_cmd:         { method: "GET",    path: "/api/knowledge/{kbId}/maintenance/proposals" },
  kb_maintenance_pending_count_cmd:{ method: "GET",    path: "/api/knowledge/{kbId}/maintenance/pending-count" },
  kb_maintenance_approve_cmd:      { method: "POST",   path: "/api/knowledge/maintenance/proposals/{id}/approve" },
  kb_maintenance_reject_cmd:       { method: "POST",   path: "/api/knowledge/maintenance/proposals/{id}/reject" },
  kb_maintenance_reject_all_cmd:   { method: "POST",   path: "/api/knowledge/{kbId}/maintenance/reject-all" },
  kb_maintenance_config_get_cmd:   { method: "GET",    path: "/api/knowledge/maintenance/config" },
  kb_maintenance_config_set_cmd:   { method: "POST",   path: "/api/knowledge/maintenance/config" },
  knowledge_compile_config_get_cmd:{ method: "GET",    path: "/api/knowledge/compile/config" },
  knowledge_compile_config_set_cmd:{ method: "POST",   path: "/api/knowledge/compile/config" },
  knowledge_vision_config_get_cmd: { method: "GET",    path: "/api/knowledge/vision/config" },
  knowledge_vision_config_set_cmd: { method: "POST",   path: "/api/knowledge/vision/config" },
  note_tools_config_get_cmd:       { method: "GET",    path: "/api/knowledge/note-tools/config" },
  note_tools_config_set_cmd:       { method: "POST",   path: "/api/knowledge/note-tools/config" },
  kb_passive_recall_config_get_cmd:{ method: "GET",    path: "/api/knowledge/passive-recall/config" },
  kb_passive_recall_config_set_cmd:{ method: "POST",   path: "/api/knowledge/passive-recall/config" },
  knowledge_media_retention_config_get_cmd: { method: "GET", path: "/api/knowledge/media-retention/config" },
  knowledge_media_retention_config_set_cmd: { method: "POST", path: "/api/knowledge/media-retention/config" },
  kb_sprite_observe_cmd:           { method: "POST",   path: "/api/knowledge/sprite/observe" },
  sprite_config_get_cmd:           { method: "GET",    path: "/api/knowledge/sprite/config" },
  sprite_config_set_cmd:           { method: "POST",   path: "/api/knowledge/sprite/config" },
  kb_note_read_ref_cmd:            { method: "GET",    path: "/api/knowledge/{kbId}/note/resolve" },
  kb_search_cmd:                   { method: "GET",    path: "/api/knowledge/search" },
  kb_file_read_cmd:                { method: "GET",    path: "/api/knowledge/{kbId}/files/read" },
  kb_file_extract_cmd:             { method: "GET",    path: "/api/knowledge/{kbId}/files/extract" },

  // -- Project file browser (workspace-scoped filesystem) --
  project_fs_list:                 { method: "GET",    path: "/api/fs/list" },
  project_fs_read_text:            { method: "GET",    path: "/api/fs/read" },
  project_fs_extract:              { method: "GET",    path: "/api/fs/extract" },
  project_fs_search:               { method: "GET",    path: "/api/fs/search" },
  project_git_info:                { method: "GET",    path: "/api/fs/git" },
  project_fs_write_text:           { method: "PUT",    path: "/api/fs/file" },
  project_fs_delete:               { method: "DELETE", path: "/api/fs/entry" },
  project_fs_rename:               { method: "POST",   path: "/api/fs/rename" },
  project_fs_mkdir:                { method: "POST",   path: "/api/fs/mkdir" },
  // Preview by absolute path (file-operations unification). Session-scoped +
  // authorized server-side; `{sessionId}` is interpolated, `path` → query.
  preview_read_text:               { method: "GET",    path: "/api/sessions/{sessionId}/files/read" },
  preview_extract:                 { method: "GET",    path: "/api/sessions/{sessionId}/files/extract" },

  // -- Sessions --
  list_sessions_cmd:               { method: "GET",    path: "/api/sessions" },
  create_session_cmd:              { method: "POST",   path: "/api/sessions" },
  get_session_cmd:                 { method: "GET",    path: "/api/sessions/{sessionId}" },
  set_session_pinned_cmd:          { method: "PATCH",  path: "/api/sessions/{sessionId}/pinned" },
  set_session_incognito:           { method: "PATCH",  path: "/api/sessions/{sessionId}/incognito" },
  set_session_working_dir:         { method: "PATCH",  path: "/api/sessions/{sessionId}/working-dir" },
  update_session_agent_cmd:        { method: "PATCH",  path: "/api/sessions/{sessionId}/agent" },
  set_session_model:               { method: "PATCH",  path: "/api/sessions/{sessionId}/model" },
  purge_session_if_incognito:      { method: "POST",   path: "/api/sessions/{sessionId}/purge-if-incognito" },
  search_sessions_cmd:             { method: "GET",    path: "/api/sessions/search" },
  search_session_messages_cmd:     { method: "GET",    path: "/api/sessions/{sessionId}/messages/search" },
  load_session_artifacts_cmd:      { method: "GET",    path: "/api/sessions/{sessionId}/artifacts" },
  list_background_jobs:            { method: "GET",    path: "/api/sessions/{sessionId}/background-jobs" },
  get_background_job:              { method: "GET",    path: "/api/background-jobs/{jobId}" },
  load_session_environment_cmd:    { method: "GET",    path: "/api/sessions/{sessionId}/environment" },
  load_session_git_diff_cmd:       { method: "GET",    path: "/api/sessions/{sessionId}/git-diff" },
  load_session_messages_latest_cmd:{ method: "GET",    path: "/api/sessions/{sessionId}/messages" },
  load_session_messages_around_cmd:{ method: "GET",    path: "/api/sessions/{sessionId}/messages/around" },
  load_session_messages_before_cmd:{ method: "GET",    path: "/api/sessions/{sessionId}/messages/before" },
  load_session_messages_after_cmd: { method: "GET",    path: "/api/sessions/{sessionId}/messages/after" },
  get_session_stream_state:        { method: "GET",    path: "/api/sessions/{sessionId}/stream-state" },
  delete_session_cmd:              { method: "DELETE", path: "/api/sessions/{sessionId}" },
  rename_session_cmd:              { method: "PATCH",  path: "/api/sessions/{sessionId}" },
  mark_session_read_cmd:           { method: "POST",   path: "/api/sessions/{sessionId}/read" },
  mark_session_read_batch_cmd:     { method: "POST",   path: "/api/sessions/read-batch" },
  mark_all_sessions_read_cmd:      { method: "POST",   path: "/api/sessions/read-all" },
  compact_context_now:             { method: "POST",   path: "/api/sessions/{sessionId}/compact" },
  write_export_file:               { method: "POST",   path: "/api/misc/write-export-file" },
  get_dangerous_mode_status:       { method: "GET",    path: "/api/security/dangerous-status" },
  set_dangerous_skip_all_approvals: { method: "POST",  path: "/api/security/dangerous-skip-all-approvals" },

  // -- Chat --
  chat:                            { method: "POST",   path: "/api/chat" },
  queue_turn_user_message:         { method: "POST",   path: "/api/chat/turn-message" },
  cancel_queued_turn_user_message: { method: "POST",   path: "/api/chat/turn-message/cancel" },
  stop_chat:                       { method: "POST",   path: "/api/chat/stop" },
  cancel_runtime_task:             { method: "POST",   path: "/api/runtime-tasks/cancel" },

  // -- Session-scoped tasks (TaskProgressPanel user controls) --
  list_session_tasks:              { method: "GET",    path: "/api/sessions/{sessionId}/tasks" },
  update_task_status:              { method: "PATCH",  path: "/api/tasks/{id}/status" },
  delete_task:                     { method: "DELETE", path: "/api/tasks/{id}" },
  set_permission_mode:             { method: "POST",   path: "/api/chat/permission-mode" },
  set_sandbox_mode:                { method: "POST",   path: "/api/chat/sandbox-mode" },
  respond_to_approval:             { method: "POST",   path: "/api/chat/approval" },
  save_attachment:                  { method: "POST",   path: "/api/chat/attachment" },
  list_builtin_tools:              { method: "GET",    path: "/api/chat/tools" },

  // -- Providers --
  get_providers:                   { method: "GET",    path: "/api/providers" },
  add_provider:                    { method: "POST",   path: "/api/providers" },
  update_provider:                 { method: "PUT",    path: "/api/providers/{providerId}" },
  delete_provider:                 { method: "DELETE", path: "/api/providers/{providerId}" },
  reorder_providers:               { method: "POST",   path: "/api/providers/reorder" },
  test_provider:                   { method: "POST",   path: "/api/providers/test" },
  test_embedding:                  { method: "POST",   path: "/api/providers/test-embedding" },
  test_image_generate:             { method: "POST",   path: "/api/providers/test-image" },
  test_model:                      { method: "POST",   path: "/api/providers/test-model" },
  test_proxy:                      { method: "POST",   path: "/api/config/proxy/test" },
  has_providers:                   { method: "GET",    path: "/api/providers/has-any" },
  get_system_timezone:             { method: "GET",    path: "/api/system/timezone" },
  list_local_embedding_models:     { method: "GET",    path: "/api/memory/local-embedding-models" },
  check_auth_status:               { method: "GET",    path: "/api/auth/codex/status" },
  logout_codex:                    { method: "POST",   path: "/api/auth/codex/logout" },
  try_restore_session:             { method: "POST",   path: "/api/auth/session/restore" },
  list_canvas_projects:            { method: "GET",    path: "/api/canvas/projects" },
  get_canvas_project:              { method: "GET",    path: "/api/canvas/projects/{projectId}" },
  delete_canvas_project:           { method: "DELETE", path: "/api/canvas/projects/{projectId}" },

  // -- Design Space --
  list_design_projects_cmd:          { method: "GET",    path: "/api/design/projects" },
  create_design_project_cmd:         { method: "POST",   path: "/api/design/projects" },
  update_design_project_cmd:         { method: "PUT",    path: "/api/design/projects" },
  get_design_project_cmd:            { method: "GET",    path: "/api/design/projects/{id}" },
  delete_design_project_cmd:         { method: "DELETE", path: "/api/design/projects/{id}" },
  duplicate_design_project_cmd:      { method: "POST",   path: "/api/design/projects/{id}/duplicate" },
  list_design_artifacts_cmd:         { method: "GET",    path: "/api/design/projects/{projectId}/artifacts" },
  design_review_artifact_cmd:        { method: "POST",   path: "/api/design/artifacts/{artifactId}/review" },
  get_design_system_kit_cmd:         { method: "GET",    path: "/api/design/systems/{id}/kit" },
  design_chat_thread_get_cmd:        { method: "GET",    path: "/api/design/projects/{projectId}/chat/thread" },
  design_chat_threads_list_cmd:      { method: "GET",    path: "/api/design/projects/{projectId}/chat/threads" },
  create_design_artifact_cmd:        { method: "POST",   path: "/api/design/artifacts" },
  import_design_image_cmd:           { method: "POST",   path: "/api/design/artifacts/import-image" },
  generate_design_artifact_cmd:      { method: "POST",   path: "/api/design/artifacts/generate" },
  design_ffmpeg_doctor_cmd:          { method: "GET",    path: "/api/design/ffmpeg/doctor" },
  design_install_ffmpeg_cmd:         { method: "POST",   path: "/api/design/ffmpeg/install" },
  design_browser_doctor_cmd:         { method: "GET",    path: "/api/design/browser/doctor" },
  design_install_browser_cmd:        { method: "POST",   path: "/api/design/browser/install" },
  list_all_design_artifacts_cmd:     { method: "GET",    path: "/api/design/artifacts" },
  get_design_artifact_cmd:           { method: "GET",    path: "/api/design/artifacts/{id}" },
  ensure_design_artifact_fresh_cmd:  { method: "POST",   path: "/api/design/artifacts/{id}/ensure-fresh" },
  delete_design_artifact_cmd:        { method: "DELETE", path: "/api/design/artifacts/{id}" },
  rename_design_artifact_cmd:        { method: "PUT",    path: "/api/design/artifacts/{id}/title" },
  duplicate_design_artifact_cmd:     { method: "POST",   path: "/api/design/artifacts/{id}/duplicate" },
  reorder_design_artifacts_cmd:      { method: "POST",   path: "/api/design/projects/{projectId}/artifacts/reorder" },
  list_design_folders_cmd:           { method: "GET",    path: "/api/design/projects/{projectId}/folders" },
  create_design_folder_cmd:          { method: "POST",   path: "/api/design/projects/{projectId}/folders" },
  rename_design_folder_cmd:          { method: "PUT",    path: "/api/design/projects/{projectId}/folders" },
  delete_design_folder_cmd:          { method: "DELETE", path: "/api/design/projects/{projectId}/folders" },
  move_design_artifact_cmd:          { method: "PUT",    path: "/api/design/artifacts/{id}/folder" },
  list_design_artifact_versions_cmd: { method: "GET",    path: "/api/design/artifacts/{id}/versions" },
  get_design_artifact_version_html_cmd: { method: "GET", path: "/api/design/artifacts/{artifactId}/versions/{versionNumber}/html" },
  create_design_share_cmd:           { method: "POST",   path: "/api/design/artifacts/{artifactId}/share" },
  get_design_share_cmd:              { method: "GET",    path: "/api/design/artifacts/{artifactId}/share" },
  revoke_design_share_cmd:           { method: "DELETE", path: "/api/design/artifacts/{artifactId}/share" },
  save_cf_deploy_config_cmd:         { method: "PUT",    path: "/api/design/deploy/config" },
  get_cf_deploy_config_cmd:          { method: "GET",    path: "/api/design/deploy/config" },
  deploy_design_artifact_cmd:        { method: "POST",   path: "/api/design/artifacts/{artifactId}/deploy" },
  bind_design_domain_cmd:            { method: "POST",   path: "/api/design/artifacts/{artifactId}/domains" },
  list_design_domains_cmd:           { method: "GET",    path: "/api/design/artifacts/{artifactId}/domains" },
  preflight_design_deploy_cmd:       { method: "GET",    path: "/api/design/artifacts/{artifactId}/deploy/preflight" },
  list_design_deployments_cmd:       { method: "GET",    path: "/api/design/artifacts/{artifactId}/deployments" },
  save_vercel_deploy_config_cmd:     { method: "PUT",    path: "/api/design/deploy/vercel/config" },
  get_vercel_deploy_config_cmd:      { method: "GET",    path: "/api/design/deploy/vercel/config" },
  deploy_design_artifact_vercel_cmd: { method: "POST",   path: "/api/design/artifacts/{artifactId}/deploy/vercel" },
  restore_design_version_cmd:        { method: "POST",   path: "/api/design/artifacts/{artifactId}/restore" },
  patch_design_element_cmd:          { method: "POST",   path: "/api/design/patch" },
  export_design_artifact_cmd:        { method: "GET",    path: "/api/design/artifacts/{id}/export" },
  export_design_handoff_cmd:         { method: "GET",    path: "/api/design/artifacts/{id}/handoff" },
  bind_design_code_project_cmd:      { method: "POST",   path: "/api/design/bindings" },
  sync_design_code_binding_cmd:      { method: "POST",   path: "/api/design/bindings/{id}/sync" },
  list_design_code_bindings_cmd:     { method: "GET",    path: "/api/design/bindings" },
  unbind_design_code_project_cmd:    { method: "DELETE", path: "/api/design/bindings/{id}" },
  export_design_pptx_cmd:            { method: "POST",   path: "/api/design/pptx" },
  export_design_zip_cmd:             { method: "POST",   path: "/api/design/zip" },
  export_design_selected_zip_cmd:    { method: "POST",   path: "/api/design/zip/selected" },
  import_design_md_cmd:              { method: "POST",   path: "/api/design/systems/import" },
  import_figma_system_cmd:           { method: "POST",   path: "/api/design/systems/figma" },
  export_design_md_cmd:              { method: "GET",    path: "/api/design/systems/{systemId}/design-md" },
  export_design_tokens_cmd:          { method: "GET",    path: "/api/design/systems/{systemId}/tokens/export" },
  critique_design_artifact_cmd:      { method: "POST",   path: "/api/design/artifacts/{id}/critique" },
  restyle_design_artifact_cmd:       { method: "POST",   path: "/api/design/artifacts/{id}/restyle" },
  list_design_systems_cmd:           { method: "GET",    path: "/api/design/systems" },
  get_design_system_cmd:             { method: "GET",    path: "/api/design/systems/{id}" },
  save_design_system_cmd:            { method: "POST",   path: "/api/design/systems" },
  extract_design_system_cmd:         { method: "POST",   path: "/api/design/systems/extract" },
  propose_design_directions_cmd:     { method: "POST",   path: "/api/design/directions" },
  list_design_recipes_cmd:           { method: "GET",    path: "/api/design/recipes" },
  export_design_native_cmd:          { method: "GET",    path: "/api/design/artifacts/{id}/native" },
  delete_design_system_cmd:          { method: "DELETE", path: "/api/design/systems/{id}" },
  get_design_config_cmd:             { method: "GET",    path: "/api/config/design" },
  save_design_config_cmd:            { method: "PUT",    path: "/api/config/design" },
  design_comment_add_cmd:            { method: "POST",   path: "/api/design/artifacts/{artifactId}/comments" },
  design_comment_list_cmd:           { method: "GET",    path: "/api/design/artifacts/{artifactId}/comments" },
  design_comment_relocate_cmd:       { method: "POST",   path: "/api/design/artifacts/{artifactId}/comments/{commentId}/relocate" },
  design_comment_update_cmd:         { method: "PUT",    path: "/api/design/artifacts/{artifactId}/comments/{commentId}" },
  design_comment_resolve_cmd:        { method: "POST",   path: "/api/design/artifacts/{artifactId}/comments/{commentId}/resolve" },
  design_comment_delete_cmd:         { method: "DELETE", path: "/api/design/artifacts/{artifactId}/comments/{commentId}" },
  design_comment_refine_cmd:         { method: "POST",   path: "/api/design/artifacts/{artifactId}/comments/{commentId}/refine" },

  // -- MCP servers --
  mcp_list_servers:                { method: "GET",    path: "/api/mcp/servers" },
  mcp_add_server:                  { method: "POST",   path: "/api/mcp/servers" },
  mcp_reorder_servers:             { method: "POST",   path: "/api/mcp/servers/reorder" },
  mcp_update_server:               { method: "PUT",    path: "/api/mcp/servers/{id}" },
  mcp_remove_server:               { method: "DELETE", path: "/api/mcp/servers/{id}" },
  mcp_get_server_status:           { method: "GET",    path: "/api/mcp/servers/{id}/status" },
  mcp_test_connection:             { method: "POST",   path: "/api/mcp/servers/{id}/test" },
  mcp_reconnect_server:            { method: "POST",   path: "/api/mcp/servers/{id}/reconnect" },
  mcp_start_oauth:                 { method: "POST",   path: "/api/mcp/servers/{id}/oauth/start" },
  mcp_sign_out:                    { method: "POST",   path: "/api/mcp/servers/{id}/oauth/sign-out" },
  mcp_list_tools:                  { method: "GET",    path: "/api/mcp/servers/{id}/tools" },
  mcp_get_recent_logs:             { method: "GET",    path: "/api/mcp/servers/{id}/logs" },
  mcp_import_claude_desktop_config:{ method: "POST",   path: "/api/mcp/import/claude-desktop" },
  mcp_get_global_settings:         { method: "GET",    path: "/api/mcp/global" },
  mcp_update_global_settings:      { method: "PUT",    path: "/api/mcp/global" },

  // -- Models --
  get_available_models:            { method: "GET",    path: "/api/models" },
  get_active_model:                { method: "GET",    path: "/api/models/active" },
  set_active_model:                { method: "POST",   path: "/api/models/active" },
  get_fallback_models:             { method: "GET",    path: "/api/models/fallback" },
  set_fallback_models:             { method: "POST",   path: "/api/models/fallback" },
  get_vision_model:                { method: "GET",    path: "/api/models/vision" },
  set_vision_model:                { method: "PUT",    path: "/api/models/vision" },
  get_automation_model_chain:      { method: "GET",    path: "/api/models/automation" },
  set_automation_model_chain:      { method: "PUT",    path: "/api/models/automation" },
  set_reasoning_effort:            { method: "POST",   path: "/api/models/reasoning-effort" },
  get_current_settings:            { method: "GET",    path: "/api/models/settings" },
  get_global_temperature:          { method: "GET",    path: "/api/models/temperature" },
  set_global_temperature:          { method: "POST",   path: "/api/models/temperature" },

  // -- Agents --
  list_agents:                     { method: "GET",    path: "/api/agents" },
  reorder_agents:                  { method: "POST",   path: "/api/agents/reorder" },
  get_agent_template:              { method: "GET",    path: "/api/agents/template" },
  initialize_agent:                { method: "POST",   path: "/api/agents/initialize" },
  get_agent_config:                { method: "GET",    path: "/api/agents/{id}" },
  save_agent_config_cmd:           { method: "PUT",    path: "/api/agents/{id}" },
  delete_agent:                    { method: "DELETE", path: "/api/agents/{id}" },
  get_agent_markdown:              { method: "GET",    path: "/api/agents/{id}/markdown" },
  save_agent_markdown:             { method: "PUT",    path: "/api/agents/{id}/markdown" },
  render_persona_to_soul_md:       { method: "POST",   path: "/api/agents/{id}/persona/render-soul-md" },
  get_agent_memory_md:             { method: "GET",    path: "/api/agents/{id}/memory-md" },
  save_agent_memory_md:            { method: "PUT",    path: "/api/agents/{id}/memory-md" },
  dreaming_run_now:                { method: "POST",   path: "/api/dreaming/run" },
  dreaming_run_resolver:           { method: "POST",   path: "/api/dreaming/resolver" },
  dreaming_run_profile:            { method: "POST",   path: "/api/dreaming/profile/run" },
  dreaming_list_profile_snapshots: { method: "GET",    path: "/api/dreaming/profile" },
  dreaming_list_diaries:           { method: "GET",    path: "/api/dreaming/diaries" },
  dreaming_read_diary:             { method: "GET",    path: "/api/dreaming/diaries/{filename}" },
  dreaming_is_running:             { method: "GET",    path: "/api/dreaming/status" },
  dreaming_last_report:            { method: "GET",    path: "/api/dreaming/last-report" },
  dreaming_idle_status:            { method: "GET",    path: "/api/dreaming/idle-status" },
  dreaming_list_runs:              { method: "GET",    path: "/api/dreaming/runs" },
  dreaming_get_run:                { method: "GET",    path: "/api/dreaming/runs/{id}" },
  dreaming_evidence_quote:         { method: "GET",    path: "/api/dreaming/evidence/quote" },
  validate_cron_expression:        { method: "POST",   path: "/api/cron/validate" },
  scan_openclaw_agents:            { method: "GET",    path: "/api/agents/openclaw/scan" },
  import_openclaw_agents:          { method: "POST",   path: "/api/agents/openclaw/import" },
  scan_openclaw_full:              { method: "GET",    path: "/api/agents/openclaw/scan-full" },
  import_openclaw_full:            { method: "POST",   path: "/api/agents/openclaw/import-full" },

  // -- User config --
  get_user_config:                 { method: "GET",    path: "/api/config/user" },
  save_user_config:                { method: "PUT",    path: "/api/config/user" },
  get_default_agent_id:            { method: "GET",    path: "/api/config/default-agent" },
  set_default_agent_id:            { method: "PUT",    path: "/api/config/default-agent" },

  // -- Memory --
  memory_search:                   { method: "POST",   path: "/api/memory/search" },
  memory_list:                     { method: "GET",    path: "/api/memory" },
  memory_count:                    { method: "GET",    path: "/api/memory/count" },
  memory_stats:                    { method: "GET",    path: "/api/memory/stats" },
  memory_add:                      { method: "POST",   path: "/api/memory" },
  memory_get:                      { method: "GET",    path: "/api/memory/{id}" },
  claim_list:                      { method: "GET",    path: "/api/claims" },
  claim_get:                       { method: "GET",    path: "/api/claims/{id}" },
  claim_update:                    { method: "PATCH",  path: "/api/claims/{id}" },
  claim_forget:                    { method: "POST",   path: "/api/claims/{id}/forget" },
  memory_backfill_plan:            { method: "GET",    path: "/api/memory/backfill/plan" },
  memory_backfill_apply:           { method: "POST",   path: "/api/memory/backfill/apply" },
  memory_update:                   { method: "PUT",    path: "/api/memory/{id}" },
  memory_delete:                   { method: "DELETE", path: "/api/memory/{id}" },
  memory_toggle_pin:               { method: "POST",   path: "/api/memory/{id}/pin" },
  memory_delete_batch:             { method: "POST",   path: "/api/memory/delete-batch" },
  memory_reembed:                  { method: "POST",   path: "/api/memory/reembed" },
  memory_export:                   { method: "POST",   path: "/api/memory/export" },
  memory_import:                   { method: "POST",   path: "/api/memory/import" },
  memory_find_similar:             { method: "POST",   path: "/api/memory/find-similar" },
  memory_get_import_from_ai_prompt:{ method: "GET",    path: "/api/memory/import-from-ai-prompt" },
  get_global_memory_md:            { method: "GET",    path: "/api/memory/global-md" },
  save_global_memory_md:           { method: "PUT",    path: "/api/memory/global-md" },

  // -- Memory config --
  get_embedding_config:            { method: "GET",    path: "/api/config/embedding" },
  save_embedding_config:           { method: "PUT",    path: "/api/config/embedding" },
  get_embedding_presets:           { method: "GET",    path: "/api/config/embedding/presets" },
  embedding_model_config_list:     { method: "GET",    path: "/api/config/embedding-models" },
  embedding_model_config_templates:{ method: "GET",    path: "/api/config/embedding-models/templates" },
  embedding_model_config_save:     { method: "PUT",    path: "/api/config/embedding-models" },
  embedding_model_config_delete:   { method: "POST",   path: "/api/config/embedding-models/delete" },
  embedding_model_config_test:     { method: "POST",   path: "/api/config/embedding-models/test" },
  memory_embedding_get:            { method: "GET",    path: "/api/config/memory-embedding" },
  memory_embedding_set_default:    { method: "POST",   path: "/api/config/memory-embedding/default" },
  memory_embedding_disable:        { method: "POST",   path: "/api/config/memory-embedding/disable" },
  memory_reembed_start:            { method: "POST",   path: "/api/memory/reembed-start" },
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
  get_hooks_config:                { method: "GET",    path: "/api/config/hooks" },
  save_hooks_config:               { method: "PUT",    path: "/api/config/hooks" },
  get_session_title_config:        { method: "GET",    path: "/api/config/session-title" },
  save_session_title_config:       { method: "PUT",    path: "/api/config/session-title" },

  // -- Behavior awareness --
  get_awareness_config:        { method: "GET",    path: "/api/config/awareness" },
  save_awareness_config:       { method: "PUT",    path: "/api/config/awareness" },
  get_session_awareness_override: { method: "GET", path: "/api/sessions/{sessionId}/awareness-config" },
  set_session_awareness_override: { method: "PATCH", path: "/api/sessions/{sessionId}/awareness-config" },

  // -- Plan mode --
  get_plan_mode:                   { method: "GET",    path: "/api/plan/{sessionId}/mode" },
  set_plan_mode:                   { method: "POST",   path: "/api/plan/{sessionId}/mode" },
  get_plan_content:                { method: "GET",    path: "/api/plan/{sessionId}/content" },
  save_plan_content:               { method: "PUT",    path: "/api/plan/{sessionId}/content" },
  get_plan_file_path:              { method: "GET",    path: "/api/plan/{sessionId}/file-path" },
  get_plan_checkpoint:             { method: "GET",    path: "/api/plan/{sessionId}/checkpoint" },
  get_plan_versions:               { method: "GET",    path: "/api/plan/{sessionId}/versions" },
  load_plan_version_content:       { method: "POST",   path: "/api/plan/version/load" },
  restore_plan_version:            { method: "POST",   path: "/api/plan/{sessionId}/version/restore" },
  plan_rollback:                   { method: "POST",   path: "/api/plan/{sessionId}/rollback" },
  cancel_plan_subagent:            { method: "POST",   path: "/api/plan/{sessionId}/cancel" },
  list_plans:                      { method: "POST",   path: "/api/plan/list" },
  resolve_plan_mention:            { method: "POST",   path: "/api/plan/resolve-mention" },
  respond_ask_user_question:       { method: "POST",   path: "/api/ask_user/respond" },
  get_pending_ask_user_group:      { method: "GET",    path: "/api/plan/{sessionId}/pending-ask-user" },
  set_plan_subagent:               { method: "POST",   path: "/api/config/plan-subagent" },
  get_plan_subagent:               { method: "GET",    path: "/api/config/plan-subagent" },
  set_ask_user_question_timeout:   { method: "POST",   path: "/api/config/ask-user-question-timeout" },
  get_ask_user_question_timeout:   { method: "GET",    path: "/api/config/ask-user-question-timeout" },
  set_ask_user_question_timeout_enabled: { method: "POST", path: "/api/config/ask-user-question-timeout-enabled" },
  get_ask_user_question_timeout_enabled: { method: "GET",  path: "/api/config/ask-user-question-timeout-enabled" },

  // -- Cron --
  cron_list_jobs:                  { method: "GET",    path: "/api/cron/jobs" },
  cron_get_job:                    { method: "GET",    path: "/api/cron/jobs/{id}" },
  cron_create_job:                 { method: "POST",   path: "/api/cron/jobs" },
  cron_update_job:                 { method: "PUT",    path: "/api/cron/jobs/{id}" },
  cron_toggle_job:                 { method: "POST",   path: "/api/cron/jobs/{id}/toggle" },
  cron_delete_job:                 { method: "DELETE", path: "/api/cron/jobs/{id}" },
  cron_run_now:                    { method: "POST",   path: "/api/cron/jobs/{id}/run" },
  cron_jobs_referencing_account:   { method: "GET",    path: "/api/cron/jobs-referencing-account/{accountId}" },
  cron_get_run_logs:               { method: "GET",    path: "/api/cron/jobs/{jobId}/logs" },
  cron_get_calendar_events:        { method: "GET",    path: "/api/cron/calendar" },
  cron_run_timeline:               { method: "GET",    path: "/api/cron/timeline" },
  cron_unread_total:               { method: "GET",    path: "/api/cron/unread" },
  cron_mark_all_read:              { method: "POST",   path: "/api/cron/read-all" },

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
  dashboard_overview_delta:        { method: "POST",   path: "/api/dashboard/overview-delta" },
  dashboard_insights:              { method: "POST",   path: "/api/dashboard/insights" },

  // -- Async / Deferred tools + Memory selection --
  get_async_tools_config:          { method: "GET",    path: "/api/config/async-tools" },
  save_async_tools_config:         { method: "PUT",    path: "/api/config/async-tools" },
  get_cron_config:                 { method: "GET",    path: "/api/config/cron" },
  save_cron_config:                { method: "PUT",    path: "/api/config/cron" },
  get_deferred_tools_config:       { method: "GET",    path: "/api/config/deferred-tools" },
  save_deferred_tools_config:      { method: "PUT",    path: "/api/config/deferred-tools" },
  get_memory_selection_config:     { method: "GET",    path: "/api/config/memory-selection" },
  save_memory_selection_config:    { method: "PUT",    path: "/api/config/memory-selection" },
  get_memory_budget_config:        { method: "GET",    path: "/api/config/memory-budget" },
  save_memory_budget_config:       { method: "PUT",    path: "/api/config/memory-budget" },

  // -- Recap --
  get_recap_config:                { method: "GET",    path: "/api/config/recap" },
  save_recap_config:               { method: "PUT",    path: "/api/config/recap" },
  get_recall_summary_config:       { method: "GET",    path: "/api/config/recall-summary" },
  save_recall_summary_config:      { method: "PUT",    path: "/api/config/recall-summary" },
  get_dreaming_config:             { method: "GET",    path: "/api/config/dreaming" },
  save_dreaming_config:            { method: "PUT",    path: "/api/config/dreaming" },
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
  get_auto_update_config:          { method: "GET",    path: "/api/config/auto-update" },
  set_auto_update_config:          { method: "PUT",    path: "/api/config/auto-update" },
  get_startup_notification_config: { method: "GET",    path: "/api/config/startup-notification" },
  save_startup_notification_config:{ method: "PUT",    path: "/api/config/startup-notification" },

  // -- Server --
  get_server_config:               { method: "GET",    path: "/api/config/server" },
  save_server_config:              { method: "PUT",    path: "/api/config/server" },
  get_server_runtime_status:       { method: "GET",    path: "/api/server/status" },

  // -- Proxy --
  get_proxy_config:                { method: "GET",    path: "/api/config/proxy" },
  save_proxy_config:               { method: "PUT",    path: "/api/config/proxy" },

  // -- Shortcuts --
  get_shortcut_config:             { method: "GET",    path: "/api/config/shortcuts" },
  save_shortcut_config:            { method: "PUT",    path: "/api/config/shortcuts" },
  set_shortcuts_paused:            { method: "POST",   path: "/api/config/shortcuts/pause" },
  get_quick_prompt_config:         { method: "GET",    path: "/api/config/quick-prompts" },
  add_quick_prompt:                { method: "POST",   path: "/api/config/quick-prompts" },

  // -- Sandbox --
  get_sandbox_config:              { method: "GET",    path: "/api/config/sandbox" },
  set_sandbox_config:              { method: "PUT",    path: "/api/config/sandbox" },
  check_sandbox_available:         { method: "GET",    path: "/api/config/sandbox/status" },

  // -- Canvas --
  get_canvas_config:               { method: "GET",    path: "/api/config/canvas" },
  save_canvas_config:              { method: "PUT",    path: "/api/config/canvas" },
  canvas_submit_snapshot:          { method: "POST",   path: "/api/canvas/snapshot/{requestId}" },
  canvas_submit_eval_result:       { method: "POST",   path: "/api/canvas/eval/{requestId}" },
  show_canvas_panel:               { method: "POST",   path: "/api/canvas/show" },
  list_canvas_projects_by_session: { method: "GET",    path: "/api/canvas/by-session/{sessionId}" },

  // -- Image generation --
  get_image_generate_config:       { method: "GET",    path: "/api/config/image-generate" },
  save_image_generate_config:      { method: "PUT",    path: "/api/config/image-generate" },
  get_audio_generate_config:       { method: "GET",    path: "/api/config/audio-generate" },
  save_audio_generate_config:      { method: "PUT",    path: "/api/config/audio-generate" },
  get_audio_model_catalog_cmd:     { method: "GET",    path: "/api/config/audio-model-catalog" },
  list_elevenlabs_voices_cmd:      { method: "GET",    path: "/api/config/elevenlabs-voices" },

  // -- Web search --
  get_web_search_config:           { method: "GET",    path: "/api/config/web-search" },
  save_web_search_config:          { method: "PUT",    path: "/api/config/web-search" },
  get_issue_reporting_config:      { method: "GET",    path: "/api/config/issue-reporting" },
  save_issue_reporting_config:     { method: "PUT",    path: "/api/config/issue-reporting" },
  save_issue_reporting_token:      { method: "PUT",    path: "/api/config/issue-reporting/token" },
  test_issue_reporting_connection: { method: "POST",   path: "/api/config/issue-reporting/test" },

  // -- Web fetch --
  get_web_fetch_config:            { method: "GET",    path: "/api/config/web-fetch" },
  save_web_fetch_config:           { method: "PUT",    path: "/api/config/web-fetch" },

  // -- SSRF policy --
  get_ssrf_config:                 { method: "GET",    path: "/api/config/ssrf" },
  save_ssrf_config:                { method: "PUT",    path: "/api/config/ssrf" },
  get_filesystem_config:           { method: "GET",    path: "/api/config/filesystem" },
  save_filesystem_config:          { method: "PUT",    path: "/api/config/filesystem" },

  // -- SearXNG Docker --
  searxng_docker_status:           { method: "GET",    path: "/api/searxng/status" },
  searxng_docker_deploy:           { method: "POST",   path: "/api/searxng/deploy" },
  searxng_docker_start:            { method: "POST",   path: "/api/searxng/start" },
  searxng_docker_stop:             { method: "POST",   path: "/api/searxng/stop" },
  searxng_docker_remove:           { method: "DELETE", path: "/api/searxng" },

  // -- Local LLM assistant --
  local_llm_detect_hardware:       { method: "GET",    path: "/api/local-llm/hardware" },
  local_llm_recommend_model:       { method: "GET",    path: "/api/local-llm/recommendation" },
  local_llm_detect_ollama:         { method: "GET",    path: "/api/local-llm/ollama-status" },
  local_llm_detect_ollama_version: { method: "GET",    path: "/api/local-llm/ollama-version" },
  local_llm_known_backends:        { method: "GET",    path: "/api/local-llm/known-backends" },
  local_llm_chat_catalog:          { method: "GET",    path: "/api/local-llm/chat-catalog" },
  local_llm_start_ollama:          { method: "POST",   path: "/api/local-llm/start" },
  local_llm_list_models:           { method: "GET",    path: "/api/local-llm/models" },
  local_llm_search_library:        { method: "GET",    path: "/api/local-llm/library/search" },
  local_llm_get_library_model:     { method: "POST",   path: "/api/local-llm/library/model" },
  local_llm_preload_model:         { method: "POST",   path: "/api/local-llm/preload" },
  local_llm_stop_model:            { method: "POST",   path: "/api/local-llm/stop-model" },
  local_llm_delete_model:          { method: "POST",   path: "/api/local-llm/delete-model" },
  local_llm_add_provider_model:    { method: "POST",   path: "/api/local-llm/provider-model" },
  local_llm_set_default_model:     { method: "POST",   path: "/api/local-llm/default-model" },
  local_llm_add_embedding_config:  { method: "POST",   path: "/api/local-llm/embedding-config" },
  local_embedding_list_models:     { method: "GET",    path: "/api/local-embedding/models" },
  local_model_job_start_chat_model:{ method: "POST",   path: "/api/local-model-jobs/chat-model" },
  local_model_job_start_embedding: { method: "POST",   path: "/api/local-model-jobs/embedding" },
  local_model_job_start_ollama_install:{ method: "POST", path: "/api/local-model-jobs/ollama-install" },
  local_model_job_start_ollama_pull:{ method: "POST",  path: "/api/local-model-jobs/ollama-pull" },
  local_model_job_start_ollama_preload:{ method: "POST", path: "/api/local-model-jobs/ollama-preload" },
  local_model_job_list:            { method: "GET",    path: "/api/local-model-jobs" },
  local_model_job_get:             { method: "GET",    path: "/api/local-model-jobs/{jobId}" },
  local_model_job_logs:            { method: "GET",    path: "/api/local-model-jobs/{jobId}/logs" },
  local_model_job_cancel:          { method: "POST",   path: "/api/local-model-jobs/{jobId}/cancel" },
  local_model_job_pause:           { method: "POST",   path: "/api/local-model-jobs/{jobId}/pause" },
  local_model_job_retry:           { method: "POST",   path: "/api/local-model-jobs/{jobId}/retry" },
  local_model_job_clear:           { method: "DELETE", path: "/api/local-model-jobs/{jobId}" },
  local_model_alert_dismiss_temporary:    { method: "POST", path: "/api/local-model/alert/dismiss-temporary" },
  local_model_alert_silence_session:      { method: "POST", path: "/api/local-model/alert/silence-session" },
  get_local_llm_auto_maintenance_enabled: { method: "GET",  path: "/api/local-model/auto-maintenance" },
  set_local_llm_auto_maintenance_enabled: { method: "PUT",  path: "/api/local-model/auto-maintenance" },
  local_model_auto_maintenance_disable:   { method: "POST", path: "/api/local-model/auto-maintenance/disable" },
  local_model_auto_maintenance_trigger:   { method: "POST", path: "/api/local-model/auto-maintenance/trigger" },

  // -- STT (Speech-to-Text) --
  get_stt_providers:                 { method: "GET",    path: "/api/stt/providers" },
  add_stt_provider:                  { method: "POST",   path: "/api/stt/providers" },
  update_stt_provider:               { method: "PUT",    path: "/api/stt/providers/{providerId}" },
  delete_stt_provider:               { method: "DELETE", path: "/api/stt/providers/{providerId}" },
  reorder_stt_providers:             { method: "POST",   path: "/api/stt/providers/reorder" },
  get_active_stt_model:              { method: "GET",    path: "/api/stt/active-model" },
  set_active_stt_model:              { method: "PUT",    path: "/api/stt/active-model" },
  clear_active_stt_model:            { method: "DELETE", path: "/api/stt/active-model" },
  get_stt_fallback_models:           { method: "GET",    path: "/api/stt/fallback-models" },
  set_stt_fallback_models:           { method: "PUT",    path: "/api/stt/fallback-models" },
  get_im_fallback_stt_model:         { method: "GET",    path: "/api/stt/im-fallback-model" },
  set_im_fallback_stt_model:         { method: "PUT",    path: "/api/stt/im-fallback-model" },
  list_known_local_stt_backends:     { method: "GET",    path: "/api/stt/local-backends" },
  probe_local_stt_backend:           { method: "GET",    path: "/api/stt/local-backends/{key}/probe" },
  upsert_known_local_stt_provider_cmd:{method: "POST",  path: "/api/stt/local-backends/{backendKey}/upsert" },
  stt_transcribe_blob:               { method: "POST",   path: "/api/stt/transcribe" },
  stt_start_session:                 { method: "POST",   path: "/api/stt/sessions" },
  stt_push_chunk:                    { method: "POST",   path: "/api/stt/sessions/{sessionId}/chunk" },
  stt_finalize_session:              { method: "POST",   path: "/api/stt/sessions/{sessionId}/finalize" },
  stt_cancel_session:                { method: "DELETE", path: "/api/stt/sessions/{sessionId}" },

  // -- Skills --
  get_skills:                      { method: "GET",    path: "/api/skills" },
  list_mentionable_skills:         { method: "GET",    path: "/api/skills/mentionable" },
  get_skill_detail:                { method: "GET",    path: "/api/skills/{name}" },
  toggle_skill:                    { method: "POST",   path: "/api/skills/{name}/toggle" },
  get_extra_skills_dirs:           { method: "GET",    path: "/api/skills/extra-dirs" },
  add_extra_skills_dir:            { method: "POST",   path: "/api/skills/extra-dirs" },
  remove_extra_skills_dir:         { method: "DELETE", path: "/api/skills/extra-dirs" },
  discover_preset_skill_sources:   { method: "GET",    path: "/api/skills/preset-sources" },
  get_skill_env:                   { method: "GET",    path: "/api/skills/{name}/env" },
  set_skill_env_var:               { method: "POST",   path: "/api/skills/{skill}/env" },
  remove_skill_env_var:            { method: "DELETE", path: "/api/skills/{skill}/env" },
  get_skills_env_status:           { method: "GET",    path: "/api/skills/env-status" },
  get_skills_status:               { method: "GET",    path: "/api/skills/status" },
  get_skill_env_check:             { method: "GET",    path: "/api/skills/env-check" },
  set_skill_env_check:             { method: "PUT",    path: "/api/skills/env-check" },
  install_skill_dependency:        { method: "POST",   path: "/api/skills/{skillName}/install" },
  list_draft_skills:               { method: "GET",    path: "/api/skills/drafts" },
  activate_draft_skill:            { method: "POST",   path: "/api/skills/{name}/activate" },
  discard_draft_skill:             { method: "DELETE", path: "/api/skills/{name}/draft" },
  trigger_skill_review_now:        { method: "POST",   path: "/api/skills/review/run" },
  get_skills_auto_review_promotion:{ method: "GET",    path: "/api/skills/auto-review/promotion" },
  set_skills_auto_review_promotion:{ method: "PUT",    path: "/api/skills/auto-review/promotion" },
  get_skills_auto_review_enabled:  { method: "GET",    path: "/api/skills/auto-review/enabled" },
  set_skills_auto_review_enabled:  { method: "PUT",    path: "/api/skills/auto-review/enabled" },
  get_skills_auto_review_config:   { method: "GET",    path: "/api/skills/auto-review/config" },
  set_skills_auto_review_config:   { method: "PATCH",  path: "/api/skills/auto-review/config" },
  reset_skills_auto_review_config: { method: "POST",   path: "/api/skills/auto-review/config/reset" },
  get_skills_auto_review_recent_rejects:{ method: "GET", path: "/api/skills/auto-review/recent-rejects" },
  run_skills_curator_now:          { method: "POST",   path: "/api/skills/curator/run" },
  apply_skills_curator_merge:      { method: "POST",   path: "/api/skills/curator/apply" },
  dashboard_learning_overview:     { method: "POST",   path: "/api/dashboard/learning/overview" },
  dashboard_learning_timeline:     { method: "POST",   path: "/api/dashboard/learning/timeline" },
  dashboard_top_skills:            { method: "POST",   path: "/api/dashboard/learning/top-skills" },
  dashboard_recall_stats:          { method: "POST",   path: "/api/dashboard/learning/recall-stats" },
  dashboard_plan_stats:            { method: "POST",   path: "/api/dashboard/plan-stats" },
  dashboard_local_model_usage:     { method: "POST",   path: "/api/dashboard/local-model-usage" },

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
  channel_set_auto_transcribe_voice:{ method: "PUT",  path: "/api/channel/accounts/{accountId}/auto-transcribe" },
  channel_start_account:           { method: "POST",   path: "/api/channel/accounts/{accountId}/start" },
  channel_stop_account:            { method: "POST",   path: "/api/channel/accounts/{accountId}/stop" },
  channel_sync_commands:           { method: "POST",   path: "/api/channel/sync-commands" },
  channel_health:                  { method: "GET",    path: "/api/channel/accounts/{accountId}/health" },
  channel_health_all:              { method: "GET",    path: "/api/channel/health" },
  channel_validate_credentials:    { method: "POST",   path: "/api/channel/validate" },
  channel_send_test_message:       { method: "POST",   path: "/api/channel/accounts/{accountId}/test-message" },
  channel_list_sessions:           { method: "GET",    path: "/api/channel/sessions" },
  channel_wechat_start_login:      { method: "POST",   path: "/api/channel/wechat/login/start" },
  channel_wechat_wait_login:       { method: "POST",   path: "/api/channel/wechat/login/wait" },
  channel_handover_session:        { method: "POST",   path: "/api/channel/handover" },

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
  get_team_messages_before:        { method: "GET",    path: "/api/teams/{teamId}/messages/before" },
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
  fetch_url_favicon:               { method: "POST",   path: "/api/url-preview/favicon" },
  fetch_url_previews:              { method: "POST",   path: "/api/url-preview/batch" },

  // -- Embedded browser --
  browser_get_status:              { method: "GET",    path: "/api/browser/status" },
  browser_extension_status:         { method: "GET",    path: "/api/browser/extension/status" },
  browser_install_native_host_manifest:{ method: "POST", path: "/api/browser/extension/install-native-host" },
  browser_extension_stop_control:   { method: "POST",   path: "/api/browser/extension/stop-control" },
  browser_list_profiles:           { method: "GET",    path: "/api/browser/profiles" },
  browser_create_profile:          { method: "POST",   path: "/api/browser/profiles" },
  browser_delete_profile:          { method: "DELETE", path: "/api/browser/profiles/{name}" },
  browser_launch:                  { method: "POST",   path: "/api/browser/launch" },
  browser_connect:                 { method: "POST",   path: "/api/browser/connect" },
  browser_disconnect:              { method: "POST",   path: "/api/browser/disconnect" },
  browser_capture_frame:           { method: "POST",   path: "/api/browser/capture-frame" },
  browser_spawn_user_chrome:       { method: "POST",   path: "/api/browser/spawn-user-chrome" },
  browser_doctor:                  { method: "GET",    path: "/api/browser/doctor" },
  browser_get_config:              { method: "GET",    path: "/api/browser/config" },
  browser_set_config:              { method: "POST",   path: "/api/browser/config" },
  browser_install_chromium_runtime:{ method: "POST",   path: "/api/browser/install-chromium-runtime" },

  // -- Theme / Language / UI --
  get_theme:                       { method: "GET",    path: "/api/config/theme" },
  set_theme:                       { method: "POST",   path: "/api/config/theme" },
  set_window_theme:                { method: "POST",   path: "/api/config/window-theme" },
  get_language:                    { method: "GET",    path: "/api/config/language" },
  set_language:                    { method: "POST",   path: "/api/config/language" },
  get_ui_effects_enabled:          { method: "GET",    path: "/api/config/ui-effects" },
  set_ui_effects_enabled:          { method: "POST",   path: "/api/config/ui-effects" },
  get_prevent_sleep_enabled:       { method: "GET",    path: "/api/config/prevent-sleep" },
  set_prevent_sleep_enabled:       { method: "POST",   path: "/api/config/prevent-sleep" },
  get_sidebar_display_mode:        { method: "GET",    path: "/api/config/sidebar-display-mode" },
  set_sidebar_display_mode:        { method: "POST",   path: "/api/config/sidebar-display-mode" },
  get_tool_call_narration_enabled: { method: "GET",    path: "/api/config/tool-call-narration" },
  set_tool_call_narration_enabled: { method: "POST",   path: "/api/config/tool-call-narration" },
  get_autostart_enabled:           { method: "GET",    path: "/api/config/autostart" },
  set_autostart_enabled:           { method: "POST",   path: "/api/config/autostart" },

  // -- Tools --
  get_tool_timeout:                { method: "GET",    path: "/api/config/tool-timeout" },
  set_tool_timeout:                { method: "POST",   path: "/api/config/tool-timeout" },
  get_timeout_policy_config:       { method: "GET",    path: "/api/config/timeout-policy" },
  save_timeout_policy_config:      { method: "PUT",    path: "/api/config/timeout-policy" },
  get_approval_timeout:            { method: "GET",    path: "/api/config/approval-timeout" },
  set_approval_timeout:            { method: "POST",   path: "/api/config/approval-timeout" },
  get_approval_timeout_enabled:    { method: "GET",    path: "/api/config/approval-timeout-enabled" },
  set_approval_timeout_enabled:    { method: "POST",   path: "/api/config/approval-timeout-enabled" },
  get_approval_timeout_action:     { method: "GET",    path: "/api/config/approval-timeout-action" },
  set_approval_timeout_action:     { method: "POST",   path: "/api/config/approval-timeout-action" },
  get_unattended_approval_action:  { method: "GET",    path: "/api/config/unattended-approval-action" },
  set_unattended_approval_action:  { method: "POST",   path: "/api/config/unattended-approval-action" },

  // -- Permission system v2 --
  get_protected_paths:             { method: "GET",    path: "/api/permission/protected-paths" },
  set_protected_paths:             { method: "POST",   path: "/api/permission/protected-paths" },
  reset_protected_paths:           { method: "POST",   path: "/api/permission/protected-paths/reset" },
  get_dangerous_commands:          { method: "GET",    path: "/api/permission/dangerous-commands" },
  set_dangerous_commands:          { method: "POST",   path: "/api/permission/dangerous-commands" },
  reset_dangerous_commands:        { method: "POST",   path: "/api/permission/dangerous-commands/reset" },
  get_edit_commands:               { method: "GET",    path: "/api/permission/edit-commands" },
  set_edit_commands:               { method: "POST",   path: "/api/permission/edit-commands" },
  reset_edit_commands:             { method: "POST",   path: "/api/permission/edit-commands/reset" },
  get_smart_mode_config:           { method: "GET",    path: "/api/permission/smart" },
  set_smart_mode_config:           { method: "POST",   path: "/api/permission/smart" },
  get_global_yolo_status:          { method: "GET",    path: "/api/permission/global-yolo" },
  mac_control_status:              { method: "GET",    path: "/api/mac-control/status" },
  mac_control_permissions:         { method: "GET",    path: "/api/mac-control/permissions" },
  mac_control_snapshot:            { method: "POST",   path: "/api/mac-control/snapshot" },
  mac_control_elements:            { method: "POST",   path: "/api/mac-control/elements" },
  mac_control_capture_frame:       { method: "POST",   path: "/api/mac-control/capture-frame" },
  get_tool_result_disk_threshold:  { method: "GET",    path: "/api/config/tool-result-threshold" },
  set_tool_result_disk_threshold:  { method: "POST",   path: "/api/config/tool-result-threshold" },
  get_tool_limits:                 { method: "GET",    path: "/api/config/tool-limits" },
  set_tool_limits:                 { method: "POST",   path: "/api/config/tool-limits" },

  // -- Crash / Recovery --
  get_crash_recovery_info:         { method: "GET",    path: "/api/crash/recovery-info" },
  get_config_health:               { method: "GET",    path: "/api/settings/config-health" },
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
  get_codex_models:                { method: "GET",    path: "/api/auth/codex/models" },
  set_codex_model:                 { method: "POST",   path: "/api/auth/codex/models" },

  // -- Desktop-only (no-op in web mode) --
  open_url:                        { method: "POST",   path: "/api/desktop/open-url" },
  open_directory:                  { method: "POST",   path: "/api/desktop/open-directory" },
  reveal_in_folder:                { method: "POST",   path: "/api/desktop/reveal-in-folder" },
  get_system_prompt:               { method: "POST",   path: "/api/system-prompt" },

  // -- First-run onboarding wizard --
  get_onboarding_state:            { method: "GET",    path: "/api/onboarding/state" },
  save_onboarding_draft:           { method: "POST",   path: "/api/onboarding/draft" },
  mark_onboarding_completed:       { method: "POST",   path: "/api/onboarding/complete" },
  mark_onboarding_skipped:         { method: "POST",   path: "/api/onboarding/skip" },
  reset_onboarding:                { method: "POST",   path: "/api/onboarding/reset" },
  apply_onboarding_language:       { method: "POST",   path: "/api/onboarding/language" },
  apply_onboarding_profile:        { method: "POST",   path: "/api/onboarding/profile" },
  apply_personality_preset_cmd:    { method: "POST",   path: "/api/onboarding/personality-preset" },
  apply_onboarding_safety:         { method: "POST",   path: "/api/onboarding/safety" },
  apply_onboarding_skills:         { method: "POST",   path: "/api/onboarding/skills" },
  apply_onboarding_server:         { method: "POST",   path: "/api/onboarding/server" },
  generate_api_key:                { method: "POST",   path: "/api/server/generate-api-key" },
  list_local_ips:                  { method: "GET",    path: "/api/server/local-ips" },
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

/**
 * Best-effort parse of an RFC 6266 / RFC 5987 `Content-Disposition` header.
 * Prefers `filename*=UTF-8''<percent-encoded>` (which the server emits for
 * non-ASCII titles) and falls back to the ASCII `filename="..."`.
 */
function parseDispositionFilename(disposition: string): string | null {
  if (!disposition) return null;
  const star = disposition.match(/filename\*\s*=\s*([^;]+)/i);
  if (star) {
    const value = star[1].trim();
    const m = value.match(/^([^']*)'([^']*)'(.+)$/);
    if (m) {
      try {
        return decodeURIComponent(m[3]);
      } catch {
        // fall through to ASCII fallback
      }
    }
  }
  const ascii = disposition.match(/filename\s*=\s*"([^"]+)"/i);
  if (ascii) return ascii[1];
  const bare = disposition.match(/filename\s*=\s*([^;]+)/i);
  if (bare) return bare[1].trim();
  return null;
}

function normalizeCommandResponse(command: string, value: unknown): unknown {
  if (
    (command === "list_sessions_cmd" || command === "list_project_sessions_cmd") &&
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    "sessions" in value &&
    "total" in value
  ) {
    // HTTP returns `{ sessions, total }` (PaginatedSessions); the Tauri command
    // returns the `[sessions, total]` tuple. Normalize both to the tuple so the
    // frontend stays transport-agnostic.
    const paginated = value as { sessions: unknown; total: unknown };
    return [paginated.sessions, paginated.total];
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    switch (command) {
      case "get_plan_mode":
        return record.state;
      case "get_plan_content":
      case "load_plan_version_content":
        return record.content;
      case "get_plan_file_path":
        return record.filePath;
      case "get_plan_checkpoint":
        return record.checkpoint;
      case "plan_rollback":
        return record.message;
      case "searxng_docker_deploy":
        return record.url;
      case "get_active_model":
        // axum 路由用 `Json(json!({"active_model": ...}))` 包了一层；Tauri 命令
        // 直接返回 `Option<ActiveModelRef>`，前端跨 transport 期望统一类型。
        return record.active_model ?? null;
      case "get_local_llm_auto_maintenance_enabled":
        // axum 路由返回 `{ enabled: bool }`；Tauri 命令直接返回 bool。
        return record.enabled ?? false;
    }
  }
  return value;
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

  /**
   * Centralized 401 handler. Every fetch site in this class funnels its
   * non-ok response status through here before throwing — without this
   * (e.g. the multipart upload / list-dir / export / search-files
   * paths), a token rejected mid-session would surface a generic error
   * but never re-trigger the AuthRequired dialog. Clears local state
   * and dispatches the event; the caller still throws so the UI's
   * own error path runs normally.
   */
  private handleAuthFailure(status: number): void {
    if (status !== 401) return;
    setStoredApiKey(null);
    this.apiKey = null;
    dispatchAuthRequired();
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
      const resp = await this.uploadMultipart<{ path: string }>(
        "/api/chat/attachment",
        args,
      );
      return resp.path as unknown as T;
    }
    if (command === "upload_project_file_cmd" && args) {
      const projectId = args.projectId as string;
      const rest = { ...args };
      delete rest.projectId;
      return this.uploadMultipart<T>(`/api/projects/${encodeURIComponent(projectId)}/files`, rest);
    }
    // Avatar upload — mirrors the Tauri `save_avatar(imageData, fileName)`
    // contract but ships raw bytes over multipart instead of base64 JSON.
    // The server returns `{ path }`; unwrap to match Tauri's `-> String`.
    if (command === "save_avatar" && args) {
      const resp = await this.uploadMultipart<{ path: string }>(
        "/api/avatars",
        args,
      );
      return resp.path as unknown as T;
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
      this.handleAuthFailure(response.status);
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

    return normalizeCommandResponse(command, await response.json()) as T;
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
      this.handleAuthFailure(response.status);
      throw new Error(`[HttpTransport] POST ${url} returned ${response.status}: ${text}`);
    }

    return (await response.json()) as T;
  }

  // ----- startChat -----

  async startChat(
    args: ChatStartArgs,
    onEvent: (event: string) => void,
  ): Promise<string> {
    // Stream deltas and turn lifecycle events arrive via /ws/events →
    // useChatStreamReattach. We only bridge `session_created` so the in-hook
    // __pending__ cache key gets renamed in place. Do not synthesize
    // `turn_started` here: POST /api/chat resolves after the engine finishes,
    // so a late local start event can incorrectly overwrite a terminal state.
    const resp = await this.call<{
      sessionId: string
      response: string
      turnId?: string
      blockedReason?: string
    }>("chat", args)
    if (!args.sessionId) {
      onEvent(
        JSON.stringify({
          type: "session_created",
          session_id: resp.sessionId,
        }),
      );
    }
    // `blockedReason` is set when a `UserPromptSubmit` hook short-circuited
    // the turn before a stream started — i.e. there will be no
    // `stream_delta` / `stream_end` events to deliver the notice through.
    // Synthesize a text delta so the UI surfaces the block reason the same
    // way the desktop path does (`src-tauri/src/commands/chat.rs` emits an
    // identical `{type:"text"}` event on the on_event channel).
    if (resp.blockedReason) {
      onEvent(
        JSON.stringify({
          type: "text",
          text: resp.blockedReason,
        }),
      )
    }
    return resp.response;
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

  private appendToken(url: string): string {
    if (!this.apiKey) return url;
    return `${url}${url.includes("?") ? "&" : "?"}token=${encodeURIComponent(this.apiKey)}`;
  }

  private addDownloadParam(href: string): string {
    if (!href.startsWith(`${this.baseUrl}/api/`)) return href;
    const url = new URL(href);
    url.searchParams.set("download", "1");
    return url.toString();
  }

  private clickHref(href: string, filename?: string): void {
    const a = document.createElement("a");
    a.href = href;
    if (filename) a.download = filename;
    a.rel = "noopener";
    a.target = "_blank";
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  async projectFsRawUrl(
    args: ProjectFsScope & { path: string; download?: boolean },
  ): Promise<string | null> {
    const url = new URL(`${this.baseUrl}/api/fs/raw`);
    url.searchParams.set("scope", args.scope);
    url.searchParams.set("scopeId", args.scopeId);
    url.searchParams.set("path", args.path);
    if (args.download) url.searchParams.set("download", "1");
    if (this.apiKey) url.searchParams.set("token", this.apiKey);
    return url.toString();
  }

  async previewReadText(
    path: string,
    opts?: { sessionId?: string | null },
  ): Promise<FileTextContent> {
    if (!opts?.sessionId) throw new Error("preview requires a session id in HTTP mode");
    return this.call<FileTextContent>("preview_read_text", {
      sessionId: opts.sessionId,
      path,
    });
  }

  async previewExtractDoc(
    path: string,
    opts?: { sessionId?: string | null },
  ): Promise<ExtractedContent> {
    if (!opts?.sessionId) throw new Error("preview requires a session id in HTTP mode");
    return this.call<ExtractedContent>("preview_extract", {
      sessionId: opts.sessionId,
      path,
    });
  }

  async previewRawUrl(
    path: string,
    opts?: { sessionId?: string | null },
    download?: boolean,
  ): Promise<string | null> {
    // The session-authorized by-path route serves inline (preview) or as an
    // attachment (download) based on `?download=1`; reuse it as the raw src.
    return this.sessionFileUrl(path, opts?.sessionId, download ?? false);
  }

  async loadSessionArtifacts(sessionId: string): Promise<SessionArtifacts> {
    return this.call<SessionArtifacts>("load_session_artifacts_cmd", { sessionId });
  }

  async loadSessionEnvironment(sessionId: string): Promise<WorkspaceEnvironmentSnapshot> {
    return this.call<WorkspaceEnvironmentSnapshot>("load_session_environment_cmd", { sessionId });
  }

  async loadSessionGitDiff(sessionId: string): Promise<FileChangesMetadata> {
    return this.call<FileChangesMetadata>("load_session_git_diff_cmd", { sessionId });
  }

  async projectFsUpload(
    args: ProjectFsScope & {
      dirPath: string;
      data: Blob;
      fileName: string;
      mimeType?: string;
      overwrite?: boolean;
    },
  ): Promise<UploadResult> {
    const qs = new URLSearchParams();
    qs.set("scope", args.scope);
    qs.set("scopeId", args.scopeId);
    qs.set("dirPath", args.dirPath);
    if (args.overwrite) qs.set("overwrite", "true");
    return this.uploadMultipart<UploadResult>(`/api/fs/upload?${qs.toString()}`, {
      data: args.data,
      fileName: args.fileName,
      mimeType: args.mimeType,
    });
  }

  private sessionFileUrl(
    path: string,
    sessionId: string | null | undefined,
    forceDownload: boolean,
  ): string | null {
    if (!sessionId) return null;
    const url = new URL(
      `${this.baseUrl}/api/sessions/${encodeURIComponent(sessionId)}/files/by-path`,
    );
    url.searchParams.set("path", path);
    if (forceDownload) url.searchParams.set("download", "1");
    if (this.apiKey) url.searchParams.set("token", this.apiKey);
    return url.toString();
  }

  resolveAssetUrl(path: string | null | undefined): string | null {
    if (!path) return null;
    if (
      path.startsWith("data:") ||
      path.startsWith("http://") ||
      path.startsWith("https://")
    ) {
      return path;
    }
    // Recognize known asset categories by their parent-directory segment
    // in the stored absolute path. Each category needs a matching
    // server-side route. Anything unrecognized returns `null` so callers
    // fall back gracefully (emoji / default icon / broken state).
    const stamped = (url: string) => this.appendToken(url);

    // Avatars: `~/.hope-agent/avatars/{file}` → `/api/avatars/{file}`
    const avatarMatch = path.match(/[\\/]avatars[\\/]([^\\/]+)$/);
    if (avatarMatch) {
      return stamped(
        `${this.baseUrl}/api/avatars/${encodeURIComponent(avatarMatch[1])}`,
      );
    }

    // Session attachments: `~/.hope-agent/attachments/{sessionId}/{file}` →
    // `/api/attachments/{sessionId}/{file}`. Used by image-tool preview
    // markers and by persisted tool attachments.
    const attachmentMatch = path.match(/[\\/]attachments[\\/]([^\\/]+)[\\/]([^\\/]+)$/);
    if (attachmentMatch) {
      return stamped(
        `${this.baseUrl}/api/attachments/${encodeURIComponent(attachmentMatch[1])}/${encodeURIComponent(attachmentMatch[2])}`,
      );
    }

    // Retained knowledge source assets:
    // `~/.hope-agent/knowledge/{kbId}/sources/assets/{sourceId}/{original|thumbnail}.{ext}`
    const sourceAssetMatch = path.match(
      /[\\/]knowledge[\\/]([^\\/]+)[\\/]sources[\\/]assets[\\/]([^\\/]+)[\\/](original|thumbnail)\.[^\\/]+$/,
    );
    if (sourceAssetMatch) {
      return stamped(
        `${this.baseUrl}/api/knowledge/${encodeURIComponent(sourceAssetMatch[1])}/sources/${encodeURIComponent(sourceAssetMatch[2])}/assets/${encodeURIComponent(sourceAssetMatch[3])}`,
      );
    }

    // Generated images: `~/.hope-agent/image_generate/{file}` → `/api/generated-images/{file}`
    // (Only the last path segment matters — historic `mediaUrls` may encode
    // different working-directory prefixes.)
    const imgMatch = path.match(/[\\/]image_generate[\\/]([^\\/]+)$/);
    if (imgMatch) {
      return stamped(
        `${this.baseUrl}/api/generated-images/${encodeURIComponent(imgMatch[1])}`,
      );
    }

    // Canvas projects: `~/.hope-agent/canvas/projects/{id}/{...rest}` →
    // `/api/canvas/projects/{id}/{...rest}`. Preserves sub-paths so the
    // iframe can load index.html plus its relative CSS / JS / images.
    const canvasMatch = path.match(/[\\/]canvas[\\/]projects[\\/]([^\\/]+)[\\/](.+)$/);
    if (canvasMatch) {
      const pid = encodeURIComponent(canvasMatch[1]);
      const rest = canvasMatch[2]
        .split("/")
        .map((seg) => encodeURIComponent(seg))
        .join("/");
      return stamped(`${this.baseUrl}/api/canvas/projects/${pid}/${rest}`);
    }

    // Design artifacts:
    // `~/.hope-agent/design/projects/{pid}/artifacts/{aid}/{...rest}` →
    // `/api/design/projects/{pid}/artifacts/{aid}/{...rest}`. Preserves
    // sub-paths so the preview iframe loads index.html plus relative assets.
    const designMatch = path.match(
      /[\\/]design[\\/]projects[\\/]([^\\/]+)[\\/]artifacts[\\/]([^\\/]+)[\\/](.+)$/,
    );
    if (designMatch) {
      const pid = encodeURIComponent(designMatch[1]);
      const aid = encodeURIComponent(designMatch[2]);
      const rest = designMatch[3]
        .split("/")
        .map((seg) => encodeURIComponent(seg))
        .join("/");
      return stamped(
        `${this.baseUrl}/api/design/projects/${pid}/artifacts/${aid}/${rest}`,
      );
    }

    return null;
  }

  async openMedia(item: MediaItem): Promise<void> {
    const href = this.resolveMediaUrl(item);
    if (!href) return;
    // Transient anchor click so the browser honors the server's
    // Content-Disposition (inline preview vs download prompt).
    this.clickHref(href);
  }

  async downloadMedia(item: MediaItem): Promise<void> {
    const href = this.resolveMediaUrl(item);
    if (!href) return;
    this.clickHref(this.addDownloadParam(href), item.name || undefined);
  }

  async openFilePath(path: string, opts?: { sessionId?: string | null }): Promise<void> {
    const href = this.sessionFileUrl(path, opts?.sessionId, false);
    if (!href) return;
    this.clickHref(href);
  }

  async downloadFilePath(
    path: string,
    opts?: { sessionId?: string | null; filename?: string },
  ): Promise<void> {
    const href = this.sessionFileUrl(path, opts?.sessionId, true);
    if (!href) return;
    this.clickHref(href, opts?.filename);
  }

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async revealMedia(_item: MediaItem): Promise<void> {
    // No-op in HTTP mode — there's no OS file manager on the client side.
  }

  supportsLocalFileOps(): boolean {
    return false;
  }

  async saveFileAs(blob: Blob, filename: string): Promise<SaveResult> {
    // Prefer the File System Access API (Chromium + secure context) so the user
    // can pick a directory + filename. Falls back to a plain browser download on
    // Firefox/Safari or plain-http, where the API is unavailable.
    // NOTE: deliberately client-side only — a remote client must NEVER write to
    // the server's disk (that would be a remote-write / exfil surface).
    const picker = (
      window as unknown as {
        showSaveFilePicker?: (opts: {
          suggestedName?: string;
        }) => Promise<{
          createWritable: () => Promise<{
            write: (data: Blob) => Promise<void>;
            close: () => Promise<void>;
          }>;
        }>;
      }
    ).showSaveFilePicker;
    if (typeof picker === "function" && window.isSecureContext) {
      try {
        const handle = await picker({ suggestedName: filename });
        const writable = await handle.createWritable();
        await writable.write(blob);
        await writable.close();
        return { status: "saved", path: null };
      } catch (e) {
        // AbortError = the user dismissed the picker → treat as cancel, don't
        // silently dump a download they didn't ask for.
        if (e instanceof DOMException && e.name === "AbortError") {
          return { status: "canceled" };
        }
        // Any other failure (NotAllowedError, etc.) → fall through to download.
      }
    }
    downloadBlob(blob, filename);
    return { status: "downloaded" };
  }

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  async revealFile(_path: string): Promise<void> {
    // No-op in HTTP mode — a sandboxed web client cannot reveal a local file.
  }

  async pickLocalImage(): Promise<PickedImage | null> {
    return new Promise<PickedImage | null>((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = "image/*";
      input.style.position = "fixed";
      input.style.left = "-9999px";
      input.style.top = "-9999px";
      input.style.opacity = "0";
      input.style.pointerEvents = "none";

      // Modern browsers fire `cancel` when the picker is dismissed without
      // a selection. Older Safari/Firefox don't, so we also piggy-back on
      // the next `focus` event after `click` — any frame in which `files`
      // hasn't populated is treated as a cancel.
      let settled = false;
      const settle = (value: PickedImage | null) => {
        if (settled) return;
        settled = true;
        cleanup();
        resolve(value);
      };
      const cleanup = () => {
        input.removeEventListener("change", onChange);
        input.removeEventListener("cancel", onCancel);
        window.removeEventListener("focus", onFocus);
        if (input.parentNode) input.parentNode.removeChild(input);
      };

      const onChange = () => {
        const file = input.files?.[0] ?? null;
        if (!file) {
          settle(null);
          return;
        }
        const url = URL.createObjectURL(file);
        settle({
          src: url,
          file,
          revoke: () => URL.revokeObjectURL(url),
        });
      };
      const onCancel = () => settle(null);
      const onFocus = () => {
        // Give the browser a tick to populate `input.files`. If nothing
        // came in, treat it as cancel.
        setTimeout(() => {
          if (!settled && !input.files?.length) settle(null);
        }, 300);
      };

      input.addEventListener("change", onChange);
      input.addEventListener("cancel", onCancel);
      window.addEventListener("focus", onFocus, { once: true });

      document.body.appendChild(input);
      input.click();
    });
  }

  async pickLocalDirectory(): Promise<string | null> {
    // In HTTP mode the browser has no access to the server's filesystem —
    // the directory picker is a React modal that calls listServerDirectory.
    // WorkingDirectoryButton branches on isTauriMode() before calling here.
    throw new Error(
      "pickLocalDirectory is not available in HTTP mode; render ServerDirectoryBrowser instead",
    );
  }

  async listServerDirectory(path?: string): Promise<DirListing> {
    const url = new URL(`${this.baseUrl}/api/filesystem/list-dir`);
    if (path) url.searchParams.set("path", path);
    const headers: Record<string, string> = {};
    if (this.apiKey) headers["Authorization"] = `Bearer ${this.apiKey}`;
    const res = await fetch(url.toString(), { method: "GET", headers });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      this.handleAuthFailure(res.status);
      let message = text || `list-dir failed: ${res.status}`;
      try {
        const parsed = JSON.parse(text) as { error?: string };
        if (parsed?.error) message = parsed.error;
      } catch {
        /* text was not JSON */
      }
      throw new Error(message);
    }
    // Response already has camelCase keys and matches `DirListing` exactly;
    // assert the shape and return without a per-entry remap.
    return (await res.json()) as DirListing;
  }

  async createDirectory(path: string): Promise<DirListing> {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (this.apiKey) headers["Authorization"] = `Bearer ${this.apiKey}`;
    const res = await fetch(`${this.baseUrl}/api/filesystem/create-dir`, {
      method: "POST",
      headers,
      body: JSON.stringify({ path }),
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      this.handleAuthFailure(res.status);
      let message = text || `create-dir failed: ${res.status}`;
      try {
        const parsed = JSON.parse(text) as { error?: string };
        if (parsed?.error) message = parsed.error;
      } catch {
        /* text was not JSON */
      }
      throw new Error(message);
    }
    return (await res.json()) as DirListing;
  }

  async exportSession(args: ExportSessionArgs): Promise<ExportSessionResult | null> {
    const url = new URL(
      `${this.baseUrl}/api/sessions/${encodeURIComponent(args.sessionId)}/export`,
    );
    url.searchParams.set("format", args.format);
    url.searchParams.set("includeThinking", String(args.includeThinking));
    url.searchParams.set("includeTools", String(args.includeTools));
    const headers: Record<string, string> = {};
    if (this.apiKey) headers["Authorization"] = `Bearer ${this.apiKey}`;
    const res = await fetch(url.toString(), { method: "GET", headers });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      this.handleAuthFailure(res.status);
      throw new Error(text || `export failed: ${res.status}`);
    }
    const disposition = res.headers.get("content-disposition") ?? "";
    const filename =
      parseDispositionFilename(disposition) ??
      args.defaultFilename ??
      `session.${args.format}`;
    const blob = await res.blob();
    return { filename, blob };
  }

  async searchFiles(root: string, q: string, limit?: number): Promise<FileSearchResponse> {
    const url = new URL(`${this.baseUrl}/api/filesystem/search-files`);
    url.searchParams.set("root", root);
    url.searchParams.set("q", q);
    if (limit !== undefined) url.searchParams.set("limit", String(limit));
    const headers: Record<string, string> = {};
    if (this.apiKey) headers["Authorization"] = `Bearer ${this.apiKey}`;
    const res = await fetch(url.toString(), { method: "GET", headers });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      this.handleAuthFailure(res.status);
      let message = text || `search-files failed: ${res.status}`;
      try {
        const parsed = JSON.parse(text) as { error?: string };
        if (parsed?.error) message = parsed.error;
      } catch {
        /* text was not JSON */
      }
      throw new Error(message);
    }
    return (await res.json()) as FileSearchResponse;
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
