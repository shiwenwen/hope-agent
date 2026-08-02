/** One-release migration support for the legacy browser Bearer-token flow. */

const STORAGE_KEY = "ha.apiKey"

/** Read the legacy root token so AuthGate can exchange and immediately erase it. */
export function getStoredApiKey(): string | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY)
    return value && value.length > 0 ? value : null
  } catch {
    return null
  }
}

/** Erase the legacy root token after migration or an authentication failure. */
export function clearStoredApiKey(): void {
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    // Storage might be disabled; there is nothing else to clean up.
  }
}

/**
 * Remove credentials from legacy `?token=` bookmarks without accepting or
 * storing them. This runs before the application mounts so the secret does
 * not remain in browser history or leak through later navigations.
 */
export function discardTokenFromUrl(): void {
  if (typeof window === "undefined") return
  try {
    const url = new URL(window.location.href)
    if (!url.searchParams.has("token")) return
    url.searchParams.delete("token")
    window.history.replaceState({}, "", `${url.pathname}${url.search}${url.hash}`)
  } catch {
    // A malformed URL should not prevent the Auth Gate from loading.
  }
}

export const AUTH_REQUIRED_EVENT = "ha:auth-required"

/** Ask the top-level Auth Gate to replace the application after a 401. */
export function dispatchAuthRequired(): void {
  if (typeof window === "undefined") return
  try {
    window.dispatchEvent(new CustomEvent(AUTH_REQUIRED_EVENT))
  } catch {
    // Older browsers without CustomEvent cannot surface the Auth Gate.
  }
}
