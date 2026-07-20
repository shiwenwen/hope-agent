// Shared "open the Help window" entry used by the sidebar icon, the About
// panel, settings deep links and the native menu / tray (via the backend).
//
// Desktop: a dedicated `help-window` WebviewWindow (get-or-create + focus,
// same pattern as the quickchat window). Web GUI: a same-origin new tab —
// `?window=help` routes main.tsx to the Help root, and the stored API token
// in localStorage carries over automatically.

import { isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"

export interface OpenHelpTarget {
  /** 0 = README index. */
  chapter?: number
  /** Heading slug inside the chapter. */
  anchor?: string
}

export const HELP_WINDOW_LABEL = "help-window"

function helpUrl(target: OpenHelpTarget = {}): string {
  const params = new URLSearchParams({ window: "help" })
  if (target.chapter !== undefined) params.set("chapter", String(target.chapter))
  if (target.anchor) params.set("anchor", target.anchor)
  return `index.html?${params.toString()}`
}

export async function openHelpWindow(target: OpenHelpTarget = {}): Promise<void> {
  if (!isTauriMode()) {
    const url = `${window.location.pathname}?${new URLSearchParams({
      window: "help",
      ...(target.chapter !== undefined ? { chapter: String(target.chapter) } : {}),
      ...(target.anchor ? { anchor: target.anchor } : {}),
    }).toString()}`
    window.open(url, "_blank", "noopener")
    return
  }

  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow")
    const existing = await WebviewWindow.getByLabel(HELP_WINDOW_LABEL)
    if (existing) {
      // Re-target an already-open window via an event (recreating it would
      // flash). Only when the caller actually names a destination — a plain
      // "open help" click must focus the window, not reset the user's
      // reading position back to the index.
      if (target.chapter !== undefined || target.anchor) {
        await existing.emit("help:navigate", target)
      }
      await existing.show()
      await existing.unminimize()
      await existing.setFocus()
      return
    }
    const webview = new WebviewWindow(HELP_WINDOW_LABEL, {
      url: helpUrl(target),
      title: "Hope Agent",
      width: 1080,
      height: 760,
      minWidth: 640,
      minHeight: 480,
      center: true,
      acceptFirstMouse: true,
    })
    webview.once("tauri://error", (e) => {
      logger.error("help", "openHelpWindow", "Failed to create help window", { error: e })
    })
  } catch (e) {
    logger.error("help", "openHelpWindow", "Open failed", { error: e })
  }
}
