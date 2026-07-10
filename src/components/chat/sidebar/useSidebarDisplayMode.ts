import { useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  DEFAULT_SIDEBAR_DISPLAY_MODE,
  normalizeSidebarDisplayMode,
  type SidebarDisplayMode,
} from "./types"

/**
 * Current sidebar display mode (`"compact"` = 简约模式 / `"detailed"`), kept in
 * sync across surfaces: loads the persisted value on mount and listens for the
 * `sidebar-display-mode-changed` window event that the sidebar / settings
 * toggle dispatches. Extracted from `ChatTitleBar`'s inline copy so the design
 * and knowledge chat panels' `AgentSwitcher` can follow 简约模式 the same way
 * (their headers previously never passed `compactLabel`, so their agent picker
 * stayed full-size regardless of the toggle).
 */
export function useSidebarDisplayMode(): SidebarDisplayMode {
  const [mode, setMode] = useState<SidebarDisplayMode>(DEFAULT_SIDEBAR_DISPLAY_MODE)

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<string>("get_sidebar_display_mode")
      .then((m) => {
        if (!cancelled) setMode(normalizeSidebarDisplayMode(m))
      })
      .catch((err) => {
        logger.error(
          "ui",
          "useSidebarDisplayMode::load",
          "Failed to load sidebar display mode",
          err,
        )
      })

    const onChange = (event: Event) => {
      const detail = (event as CustomEvent<{ mode?: unknown }>).detail
      setMode(normalizeSidebarDisplayMode(detail?.mode))
    }
    window.addEventListener("sidebar-display-mode-changed", onChange)
    return () => {
      cancelled = true
      window.removeEventListener("sidebar-display-mode-changed", onChange)
    }
  }, [])

  return mode
}
