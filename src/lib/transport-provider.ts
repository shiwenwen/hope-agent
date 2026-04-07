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

import { isTauriMode } from "@/lib/transport";
import type { Transport } from "@/lib/transport";
import { TauriTransport } from "@/lib/transport-tauri";
import { HttpTransport } from "@/lib/transport-http";

/** Default server URL for standalone web mode. */
const DEFAULT_HTTP_BASE = "http://localhost:8420";

let instance: Transport | null = null;

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
    // or fall back to the default.
    const baseUrl = import.meta.env?.VITE_SERVER_URL || DEFAULT_HTTP_BASE;
    instance = new HttpTransport(baseUrl);
  }

  return instance;
}

/**
 * Replace the current transport singleton (useful for testing).
 */
export function setTransport(transport: Transport): void {
  instance = transport;
}

/**
 * Switch to a remote HTTP transport with the given base URL and optional API key.
 * Replaces the current singleton so all subsequent calls go to the remote server.
 */
export function switchToRemote(baseUrl: string, apiKey?: string | null): void {
  instance = new HttpTransport(baseUrl, apiKey);
}

/**
 * Switch back to the default transport (Tauri IPC if in Tauri, else localhost).
 * Resets the singleton so it will be recreated on next `getTransport()` call.
 */
export function switchToEmbedded(): void {
  if (isTauriMode()) {
    instance = new TauriTransport();
  } else {
    instance = new HttpTransport(DEFAULT_HTTP_BASE);
  }
}
