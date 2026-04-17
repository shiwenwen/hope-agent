import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"

export const WINDOW_OPACITY_MIN = 0.3
export const WINDOW_OPACITY_MAX = 1.0

function clamp(v: number): number {
  if (!Number.isFinite(v)) return 1.0
  return Math.min(WINDOW_OPACITY_MAX, Math.max(WINDOW_OPACITY_MIN, v))
}

/** Apply opacity to the document root CSS variable used by body + .bg-app-window. */
export function applyWindowOpacityVisual(opacity: number) {
  const v = clamp(opacity)
  document.documentElement.style.setProperty("--window-opacity", String(v))
}

/** Load opacity from backend config and apply immediately. Safe to call from anywhere. */
export async function initWindowOpacity(): Promise<number> {
  try {
    const v = await getTransport().call<number>("get_window_opacity")
    const clamped = clamp(v)
    applyWindowOpacityVisual(clamped)
    return clamped
  } catch {
    applyWindowOpacityVisual(1.0)
    return 1.0
  }
}

export function useWindowOpacity() {
  const [opacity, setOpacityState] = useState<number>(1.0)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    let cancelled = false
    initWindowOpacity().then((v) => {
      if (cancelled) return
      setOpacityState(v)
      setLoaded(true)
    })
    return () => { cancelled = true }
  }, [])

  const setOpacity = useCallback(async (next: number) => {
    const target = clamp(next)
    setOpacityState(target)
    applyWindowOpacityVisual(target)
    try {
      const persisted = await getTransport().call<number>("set_window_opacity", { opacity: target })
      if (typeof persisted === "number" && Number.isFinite(persisted)) {
        const c = clamp(persisted)
        setOpacityState(c)
        applyWindowOpacityVisual(c)
      }
    } catch {
      /* failure reverts are handled by the caller if needed */
    }
  }, [])

  useEffect(() => {
    return getTransport().listen("config:changed", (raw) => {
      try {
        const payload = typeof raw === "string" ? JSON.parse(raw) : raw
        if (payload?.category === "window_opacity") {
          getTransport().call<number>("get_window_opacity").then((v) => {
            const c = clamp(v)
            setOpacityState(c)
            applyWindowOpacityVisual(c)
          }).catch(() => {})
        }
      } catch { /* ignore */ }
    })
  }, [])

  return { opacity, setOpacity, loaded }
}
