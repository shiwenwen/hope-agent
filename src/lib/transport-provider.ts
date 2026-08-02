/**
 * Transport singleton with automatic environment detection.
 *
 * Usage:
 * ```ts
 * import { getTransport } from "@/lib/transport-provider";
 *
 * const transport = getTransport();
 * const sessions = await transport.call<SessionMeta[]>("list_sessions_cmd", { limit: 50 });
 * ```
 *
 * In Tauri mode, the singleton is a `TauriTransport` backed by native IPC.
 * In web mode, it is an `HttpTransport` pointing at the configured server URL.
 */

import { useSyncExternalStore } from "react";
import { isTauriMode } from "@/lib/transport";
import type { Transport } from "@/lib/transport";
import { TauriTransport } from "@/lib/transport-tauri";
import { HttpTransport, RemoteAuthenticationError } from "@/lib/transport-http";
import { getStoredApiKey } from "@/lib/api-key-storage";
import { normalizeHttpBaseUrl } from "@/lib/httpUrl";
import { confirmDiscardDirtyFileEditors } from "@/components/chat/files/fileDirtyRegistry";

/**
 * Default server URL for standalone web mode.
 *
 * Prefers `window.location.origin` when the page itself is served by the
 * Hope Agent server (the common Docker / reverse-proxy case) — that way
 * the browser hits the same hostname / port / scheme it loaded from
 * instead of `localhost`, which would resolve to the visitor's own
 * machine. The hard-coded `http://localhost:8420` only kicks in for
 * non-browser callers (SSR, tests, build-time tooling).
 */
function defaultHttpBase(): string {
  if (typeof window !== "undefined" && window.location?.origin) {
    return window.location.origin;
  }
  return "http://localhost:8420";
}

/** HTTP server selected for standalone web mode, normalized for path joins. */
export function configuredHttpBase(): string {
  return (import.meta.env?.VITE_SERVER_URL || defaultHttpBase()).replace(/\/+$/, "");
}

/** Whether the configured server can use the browser's same-origin session cookie. */
export function isConfiguredHttpBaseSameOrigin(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return new URL(configuredHttpBase()).origin === window.location.origin;
  } catch {
    return false;
  }
}

let instance: Transport | null = null;
const listeners = new Set<() => void>();
let transportRevision = 0;
let dirtyTransportConfirmText =
  "You have unsaved file changes. Switch connection and discard them?";

/** App-level i18n bridge kept outside the transport singleton's dependency graph. */
export function setDirtyTransportConfirmText(message: string): void {
  if (message.trim()) dirtyTransportConfirmText = message;
}

function emitTransportChanged(): void {
  transportRevision += 1;
  for (const listener of listeners) listener();
}

function subscribeTransport(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Monotonic identity for transport switches and scoped-ticket refreshes. */
export function getTransportRevision(): number {
  return transportRevision;
}

/** Re-render a consumer whenever transport-scoped credentials change. */
export function useTransportRevision(): number {
  return useSyncExternalStore(
    subscribeTransport,
    getTransportRevision,
    getTransportRevision,
  );
}

export function confirmTransportChange(): boolean {
  return confirmDiscardDirtyFileEditors(dirtyTransportConfirmText);
}

/**
 * Return the application-wide Transport singleton.
 *
 * The first call detects the environment and creates the appropriate
 * implementation. Subsequent calls return the cached instance.
 */
export function getTransport(): Transport {
  if (instance) return instance;

  if (isTauriMode()) {
    instance = new TauriTransport();
  } else {
    // In standalone web mode, read the server URL from a Vite env variable
    // or fall back to the page's origin. A legacy localStorage token is
    // visible here for one release only; AuthGate exchanges and clears it.
    const baseUrl = configuredHttpBase();
    const apiKey = getStoredApiKey();
    const http = new HttpTransport(baseUrl, apiKey, emitTransportChanged);
    instance = http;
    if (apiKey) {
      void http.initializeRemoteAccess().then(emitTransportChanged).catch(() => undefined);
    }
  }

  return instance;
}

/**
 * Authenticate the standalone web client without persisting the Owner Token.
 * Same-origin deployments exchange it for an HttpOnly cookie; split-origin
 * deployments retain it only in the in-memory HTTP transport and mint scoped
 * browser tickets for resources and WebSockets.
 */
export async function authenticateWebOwnerToken(token: string): Promise<boolean> {
  const baseUrl = configuredHttpBase();
  if (isConfiguredHttpBaseSameOrigin()) {
    const response = await fetch(`${baseUrl}/api/auth/session`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token, remember: true }),
    });
    return response.ok;
  }

  const next = new HttpTransport(baseUrl, token, emitTransportChanged);
  try {
    await next.initializeRemoteAccess(false);
  } catch (error) {
    next.dispose();
    if (error instanceof RemoteAuthenticationError) return false;
    throw error;
  }
  if (instance instanceof HttpTransport) instance.dispose();
  instance = next;
  emitTransportChanged();
  return true;
}

/**
 * Replace the current transport singleton (useful for testing).
 */
export function setTransport(transport: Transport): void {
  if (instance instanceof HttpTransport && instance !== transport) instance.dispose();
  instance = transport;
  emitTransportChanged();
}

/** Fail-closed identity check for decisions that may reuse the active client. */
export function isCurrentHttpTransportFor(baseUrl: string): boolean {
  return instance instanceof HttpTransport && instance.matchesBaseUrl(baseUrl);
}

export interface PreparedRemoteTransport {
  /** Publish the already-validated client as the application singleton. */
  activate(): void;
  /** Erase provisional credentials when persistence or another setup step fails. */
  dispose(): void;
}

/**
 * Validate a provisional remote client without changing the active singleton.
 * Callers can safely persist connection preferences through the current
 * transport, then publish this exact authenticated client with `activate()`.
 */
export async function prepareRemoteTransport(
  baseUrl: string,
  apiKey?: string | null,
): Promise<PreparedRemoteTransport> {
  let published = false;
  const next = new HttpTransport(baseUrl, apiKey, () => {
    if (published) emitTransportChanged();
  });
  try {
    if (apiKey) {
      await next.initializeRemoteAccess(false);
    } else {
      const normalizedBase = normalizeHttpBaseUrl(baseUrl);
      const response = await fetch(`${normalizedBase}/api/auth/status`, {
        signal: AbortSignal.timeout(10_000),
      });
      if (!response.ok) throw new Error(`Remote connection failed (${response.status})`);
      const status = (await response.json()) as {
        authRequired: boolean;
        authenticated: boolean;
      };
      if (status.authRequired && !status.authenticated) {
        throw new RemoteAuthenticationError(401);
      }
    }
  } catch (error) {
    next.dispose();
    throw error;
  }

  let available = true;
  return {
    activate() {
      if (!available) throw new Error("Prepared remote transport is no longer available");
      available = false;
      published = true;
      if (instance instanceof HttpTransport) instance.dispose();
      instance = next;
      emitTransportChanged();
    },
    dispose() {
      if (!available) return;
      available = false;
      next.dispose();
    },
  };
}

/**
 * Switch to a remote HTTP transport with the given base URL and optional API key.
 * Replaces the current singleton so all subsequent calls go to the remote server.
 */
export async function switchToRemote(
  baseUrl: string,
  apiKey?: string | null,
  options?: { dirtyConfirmed?: boolean },
): Promise<boolean> {
  if (!options?.dirtyConfirmed && !confirmTransportChange()) return false;
  const prepared = await prepareRemoteTransport(baseUrl, apiKey);
  prepared.activate();
  return true;
}

/** Keep the active HTTP client authenticated after the server changes its root token. */
export async function activateCurrentHttpOwnerToken(token: string): Promise<boolean> {
  if (instance instanceof HttpTransport) {
    await instance.activateOwnerToken(token);
    emitTransportChanged();
    return true;
  }
  return false;
}

/**
 * Switch back to the default transport (Tauri IPC if in Tauri, else localhost).
 * Resets the singleton so it will be recreated on next `getTransport()` call.
 */
export function switchToEmbedded(options?: { dirtyConfirmed?: boolean }): boolean {
  if (!options?.dirtyConfirmed && !confirmTransportChange()) return false;
  if (instance instanceof HttpTransport) instance.dispose();
  if (isTauriMode()) {
    instance = new TauriTransport();
  } else {
    // AuthGate normally cleared this legacy value before App mounted.
    instance = new HttpTransport(defaultHttpBase(), getStoredApiKey());
  }
  emitTransportChanged();
  return true;
}

/** React hook that re-renders consumers as soon as the active transport changes. */
export function useTransport(): Transport {
  useTransportRevision();
  return getTransport();
}
