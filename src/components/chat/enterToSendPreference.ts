import { useEffect, useRef, useState } from "react"

import { logger } from "@/lib/logger"
import { getTransport, useTransportRevision } from "@/lib/transport-provider"

export const ENTER_TO_SEND_EVENT = "hope:enterToSend"

export function normalizeEnterToSendPreference(value: unknown): boolean {
  return value !== false
}

function shouldReloadForConfigEvent(payload: unknown): boolean {
  if (!payload || typeof payload !== "object") return true
  const category = (payload as { category?: unknown }).category
  return typeof category !== "string" || category === "user"
}

export function emitEnterToSendPreference(enabled: boolean): void {
  if (typeof window === "undefined") return
  window.dispatchEvent(
    new CustomEvent(ENTER_TO_SEND_EVENT, {
      detail: { enabled },
    }),
  )
}

/**
 * Keep every ChatInput surface on the same global send shortcut. The local
 * event makes Settings changes immediate in the current window; the backend
 * config event covers ha-settings updates and other connected surfaces.
 */
export function useEnterToSendPreference(): { enabled: boolean; ready: boolean } {
  const transportRevision = useTransportRevision()
  const [preference, setPreference] = useState(() => ({
    enabled: true,
    loadedTransportRevision: null as number | null,
  }))
  const requestRevisionRef = useRef(0)

  useEffect(() => {
    let cancelled = false
    const transport = getTransport()

    const load = async () => {
      const revision = ++requestRevisionRef.current
      try {
        const config = await transport.call<{ enterToSend?: unknown }>("get_user_config")
        if (cancelled || revision !== requestRevisionRef.current) return
        const enabled = normalizeEnterToSendPreference(config.enterToSend)
        setPreference({ enabled, loadedTransportRevision: transportRevision })
      } catch (error) {
        logger.warn(
          "settings",
          "useEnterToSendPreference",
          "Failed to load chat send shortcut preference",
          error,
        )
      }
    }

    void load()
    const unlistenConfig = transport.listen("config:changed", (payload) => {
      if (!shouldReloadForConfigEvent(payload)) return
      setPreference((current) => ({ ...current, loadedTransportRevision: null }))
      void load()
    })
    const handlePreferenceChange = (event: Event) => {
      requestRevisionRef.current += 1
      setPreference({
        enabled: normalizeEnterToSendPreference((event as CustomEvent).detail?.enabled),
        loadedTransportRevision: transportRevision,
      })
    }
    window.addEventListener(ENTER_TO_SEND_EVENT, handlePreferenceChange)

    return () => {
      cancelled = true
      unlistenConfig()
      window.removeEventListener(ENTER_TO_SEND_EVENT, handlePreferenceChange)
    }
  }, [transportRevision])

  return {
    enabled: preference.enabled,
    ready: preference.loadedTransportRevision === transportRevision,
  }
}
