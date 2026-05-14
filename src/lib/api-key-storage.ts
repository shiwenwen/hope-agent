/**
 * Browser-side API key storage for the standalone Web GUI mode.
 *
 * The Hope Agent server enforces a Bearer token whenever `HA_API_KEY` is
 * set. The Tauri desktop app stores its token in native config; the Web
 * GUI has no such backing store, so we lean on `localStorage` plus a
 * one-shot URL capture: visit `https://host/?token=XXX` once, the page
 * pulls the value into storage, rewrites the URL to drop the query
 * string, and subsequent loads reuse it transparently. The 401 retry
 * path (see `transport-http.ts`) clears storage and asks the user to
 * paste a fresh token when the server rejects what we have.
 */

const STORAGE_KEY = "ha.apiKey";
const URL_PARAM = "token";

/** Read the stored Bearer token, or `null` if storage is empty / disabled. */
export function getStoredApiKey(): string | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value && value.length > 0 ? value : null;
  } catch {
    return null;
  }
}

/** Persist (or clear, when passed `null`) the Bearer token. */
export function setStoredApiKey(token: string | null): void {
  try {
    if (token) {
      localStorage.setItem(STORAGE_KEY, token);
      // User supplied a fresh token — assume it's valid; let the next
      // 401 (if any) re-arm the sticky flag.
      authRequiredSticky = false;
    } else {
      localStorage.removeItem(STORAGE_KEY);
    }
  } catch {
    // Storage might be disabled (Safari private mode, server-side render);
    // we silently fall back to in-memory-only behavior.
  }
}

/**
 * One-shot URL capture: read `?token=…` from the current address,
 * persist it, and rewrite the URL via `history.replaceState` so the
 * token never lands in browser history, the `Referer` header, or
 * bookmarks. Safe to call multiple times — a no-op when the URL has
 * no `token` param.
 *
 * MUST run before the transport singleton is constructed so the
 * `HttpTransport` picks the new value up on first use.
 */
export function captureTokenFromUrl(): void {
  if (typeof window === "undefined") return;
  try {
    const url = new URL(window.location.href);
    const token = url.searchParams.get(URL_PARAM);
    if (!token) return;
    setStoredApiKey(token);
    url.searchParams.delete(URL_PARAM);
    const cleaned = url.pathname + (url.search ? `?${url.searchParams.toString()}` : "") + url.hash;
    window.history.replaceState({}, "", cleaned);
  } catch {
    // Malformed URL or storage error — leave things as-is.
  }
}

/**
 * Event name dispatched on `window` when the server returns 401 with a
 * Bearer-token-required error. Listeners can open a token-entry dialog;
 * the transport itself stays silent so each consumer can decide whether
 * to retry or surface an error.
 */
export const AUTH_REQUIRED_EVENT = "ha:auth-required";

/**
 * Sticky flag: stays true after `dispatchAuthRequired()` until the user
 * actually provides a token via `setStoredApiKey`. The 401 that
 * triggers the dialog frequently fires during the very first boot
 * `useEffect`, while `AuthRequiredDialog` has not yet mounted — without
 * a sticky flag the event arrives at an empty listener list and the
 * dialog never opens. The dialog reads (and clears) this flag on mount
 * to recover the event after the fact.
 */
let authRequiredSticky = false;

/** Notify listeners that the stored token (if any) was rejected. */
export function dispatchAuthRequired(): void {
  authRequiredSticky = true;
  if (typeof window === "undefined") return;
  try {
    window.dispatchEvent(new CustomEvent(AUTH_REQUIRED_EVENT));
  } catch {
    // Older browsers without CustomEvent — drop. The page just won't
    // surface the dialog.
  }
}

/**
 * Read and clear the sticky auth-required flag. `AuthRequiredDialog`
 * calls this when it mounts so it can catch up to a 401 that fired
 * before its listener was attached.
 */
export function consumeAuthRequiredSticky(): boolean {
  const v = authRequiredSticky;
  authRequiredSticky = false;
  return v;
}
