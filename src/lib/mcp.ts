/**
 * MCP (Model Context Protocol) client helper.
 *
 * Mirrors the serialized shapes from `crates/ha-core/src/mcp/api.rs`
 * and `crates/ha-core/src/mcp/config.rs`. Every function here is a
 * thin wrapper over `transport.call(...)` so switching between Tauri
 * IPC and HTTP transport is invisible to callers.
 */

import { transport } from "@/lib/transport-provider";

// ── Enums & unions ───────────────────────────────────────────────

export type McpTransportKind = "stdio" | "streamableHttp" | "sse" | "websocket";

export type McpTransportSpec =
  | { kind: "stdio"; command: string; args?: string[]; cwd?: string | null }
  | { kind: "streamableHttp"; url: string }
  | { kind: "sse"; url: string }
  | { kind: "websocket"; url: string };

export type McpTrustLevel = "untrusted" | "trusted";

/** Matches `crates/ha-core/src/mcp/registry.rs::ServerState::label()`. */
export type McpServerState =
  | "disabled"
  | "idle"
  | "connecting"
  | "ready"
  | "needsAuth"
  | "failed";

// ── Persisted config shape ───────────────────────────────────────

export interface McpOAuthConfig {
  clientId?: string | null;
  clientSecret?: string | null;
  authorizationEndpoint?: string | null;
  tokenEndpoint?: string | null;
  scopes?: string[];
  extraParams?: Record<string, string>;
}

export interface McpServerConfig {
  id: string;
  name: string;
  enabled: boolean;
  transport: McpTransportSpec;
  env?: Record<string, string>;
  headers?: Record<string, string>;
  oauth?: McpOAuthConfig | null;
  allowedTools?: string[];
  deniedTools?: string[];
  connectTimeoutSecs: number;
  callTimeoutSecs: number;
  healthCheckIntervalSecs: number;
  maxConcurrentCalls: number;
  autoApprove: boolean;
  trustLevel: McpTrustLevel;
  eager: boolean;
  projectPaths?: string[];
  description?: string | null;
  icon?: string | null;
  createdAt: number;
  updatedAt: number;
  trustAcknowledgedAt?: string | null;
}

/** Shape sent to `mcp_add_server` / `mcp_update_server` — id optional. */
export type McpServerDraft = Omit<
  McpServerConfig,
  | "id"
  | "createdAt"
  | "updatedAt"
  | "connectTimeoutSecs"
  | "callTimeoutSecs"
  | "healthCheckIntervalSecs"
  | "maxConcurrentCalls"
> & {
  id?: string;
  connectTimeoutSecs?: number | null;
  callTimeoutSecs?: number | null;
  healthCheckIntervalSecs?: number | null;
  maxConcurrentCalls?: number | null;
};

export interface McpGlobalSettings {
  enabled: boolean;
  maxConcurrentCalls: number;
  backoffInitialSecs: number;
  backoffMaxSecs: number;
  consecutiveFailureCircuitBreaker: number;
  autoReconnectAfterCircuitSecs: number;
  alwaysLoadServers?: string[];
  deniedServers?: string[];
}

// ── Runtime snapshots ────────────────────────────────────────────

export interface McpServerStatusSnapshot {
  id: string;
  name: string;
  enabled: boolean;
  transportKind: McpTransportKind;
  state: McpServerState;
  reason?: string | null;
  toolCount: number;
  resourceCount: number;
  promptCount: number;
  consecutiveFailures: number;
  lastHealthCheckTs: number;
}

/** `McpServerSummary` from api.rs — config + status merged. */
export type McpServerSummary = McpServerConfig & Omit<McpServerStatusSnapshot, "transportKind">;

export interface McpToolSummary {
  name: string;
  namespacedName: string;
  description?: string;
}

export interface McpLogLine {
  ts: number;
  level: string;
  source: string;
  message: string;
}

export interface McpImportSummary {
  imported: string[];
  skipped: { name: string; reason: string }[];
}

// ── Transport helpers ────────────────────────────────────────────

export function listServers(): Promise<McpServerSummary[]> {
  return transport.call("mcp_list_servers");
}

export function getServerStatus(id: string): Promise<McpServerStatusSnapshot> {
  return transport.call("mcp_get_server_status", { id });
}

export function addServer(draft: McpServerDraft): Promise<McpServerSummary> {
  return transport.call("mcp_add_server", { draft });
}

export function updateServer(
  id: string,
  draft: McpServerDraft,
): Promise<McpServerSummary> {
  return transport.call("mcp_update_server", { id, draft });
}

export function removeServer(id: string): Promise<void> {
  return transport.call("mcp_remove_server", { id });
}

export function reorderServers(order: string[]): Promise<void> {
  return transport.call("mcp_reorder_servers", { order });
}

export function testConnection(id: string): Promise<McpServerStatusSnapshot> {
  return transport.call("mcp_test_connection", { id });
}

export function reconnectServer(id: string): Promise<McpServerStatusSnapshot> {
  return transport.call("mcp_reconnect_server", { id });
}

export function startOauth(id: string): Promise<void> {
  return transport.call("mcp_start_oauth", { id });
}

export function signOut(id: string): Promise<void> {
  return transport.call("mcp_sign_out", { id });
}

export function listServerTools(id: string): Promise<McpToolSummary[]> {
  return transport.call("mcp_list_tools", { id });
}

export function getRecentLogs(
  id: string,
  limit: number = 200,
): Promise<McpLogLine[]> {
  return transport.call("mcp_get_recent_logs", { id, limit });
}

export function importClaudeDesktopConfig(
  json: string,
): Promise<McpImportSummary> {
  return transport.call("mcp_import_claude_desktop_config", { json });
}

export function getGlobalSettings(): Promise<McpGlobalSettings> {
  return transport.call("mcp_get_global_settings");
}

export function updateGlobalSettings(
  settings: McpGlobalSettings,
): Promise<void> {
  return transport.call("mcp_update_global_settings", { settings });
}

// ── EventBus subscriptions ──────────────────────────────────────

export const MCP_EVENTS = {
  SERVER_STATUS_CHANGED: "mcp:server_status_changed",
  CATALOG_REFRESHED: "mcp:catalog_refreshed",
  AUTH_REQUIRED: "mcp:auth_required",
  AUTH_COMPLETED: "mcp:auth_completed",
  SERVERS_CHANGED: "mcp:servers_changed",
  SERVER_LOG: "mcp:server_log",
} as const;

// ── UI helpers ──────────────────────────────────────────────────

/** Decompose a namespaced tool name into its constituents, or null when
 * the name isn't MCP-owned. */
export function parseMcpToolName(
  name: string,
): { serverName: string; tool: string } | null {
  if (!name.startsWith("mcp__")) return null;
  const rest = name.slice(5);
  const sep = rest.indexOf("__");
  if (sep <= 0) return null;
  return {
    serverName: rest.slice(0, sep),
    tool: rest.slice(sep + 2),
  };
}
