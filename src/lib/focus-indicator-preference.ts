import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

const LOAD_TIMEOUT_MS = 2_000

async function withTimeout<T>(operation: Promise<T>, timeoutMs: number): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error("Focus preference load timed out")), timeoutMs)
  })
  try {
    return await Promise.race([operation, timeout])
  } finally {
    if (timeoutId !== undefined) clearTimeout(timeoutId)
  }
}

export function applyEnhancedFocusIndicators(enabled: boolean): void {
  document.documentElement.dataset.focusIndicators = enabled ? "enhanced" : "auto"
}

export async function loadEnhancedFocusIndicators(): Promise<boolean> {
  try {
    const enabled = await withTimeout(
      getTransport().call<boolean>("get_enhanced_focus_indicators"),
      LOAD_TIMEOUT_MS,
    )
    applyEnhancedFocusIndicators(enabled)
    return enabled
  } catch {
    applyEnhancedFocusIndicators(false)
    return false
  }
}

export function saveEnhancedFocusIndicators(enabled: boolean): Promise<unknown> {
  applyEnhancedFocusIndicators(enabled)
  return getTransport().call("set_enhanced_focus_indicators", { enabled })
}

export function listenEnhancedFocusIndicators(onChange?: (enabled: boolean) => void): () => void {
  return getTransport().listen("config:changed", (raw) => {
    const payload = parsePayload<{ category?: string }>(raw)
    if (
      payload?.category !== "focus_indicator" &&
      payload?.category !== "app" &&
      payload?.category !== "settings_reset.general" &&
      payload?.category !== "settings_reset.general.appearance"
    ) return
    void loadEnhancedFocusIndicators().then(onChange)
  })
}
