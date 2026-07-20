/**
 * HelpWindow — root component for the built-in user manual.
 * Rendered when `?window=help` is in the URL (see main.tsx): a dedicated
 * Tauri window on desktop, a same-origin browser tab in Web GUI mode.
 * Initial chapter/anchor arrive via URL params; re-targeting an already-open
 * desktop window arrives via the `help:navigate` window event.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { BookOpenText, X } from "lucide-react"

import { initLanguageFromConfig } from "@/i18n/i18n"
import { initThemeFromConfig, listenThemeConfigChange } from "@/hooks/useTheme"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import HelpView, { type HelpTarget } from "@/components/help/HelpView"
import { isTauriMode } from "@/lib/transport"

function initialTargetFromUrl(): HelpTarget {
  const params = new URLSearchParams(window.location.search)
  const chapterRaw = params.get("chapter")
  const chapter = chapterRaw === null ? undefined : Number(chapterRaw)
  return {
    chapter: chapter !== undefined && Number.isInteger(chapter) ? chapter : undefined,
    anchor: params.get("anchor") ?? undefined,
  }
}

export default function HelpWindow() {
  const { t } = useTranslation()
  const desktop = isTauriMode()
  const [initialTarget] = useState<HelpTarget>(initialTargetFromUrl)
  const [navigateSignal, setNavigateSignal] = useState<{
    nonce: number
    target: HelpTarget
  } | null>(null)

  useEffect(() => {
    void initLanguageFromConfig()
    // Unlike theme-init.js (localStorage/system only), this applies the
    // user's persisted theme preference — reading-heavy window, no dark flash.
    void initThemeFromConfig()
    return listenThemeConfigChange()
  }, [])

  // Desktop: `openHelpWindow` re-targets an already-open window via event.
  useEffect(() => {
    if (!desktop) return
    let unlisten: (() => void) | null = null
    let cancelled = false
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<HelpTarget>("help:navigate", (event) => {
        setNavigateSignal((prev) => ({
          nonce: (prev?.nonce ?? 0) + 1,
          target: event.payload ?? {},
        }))
      }).then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      }),
    )
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [desktop])

  const handleClose = () => {
    if (!desktop) {
      window.close()
      return
    }
    void import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
      getCurrentWindow().close(),
    )
  }

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col bg-background text-foreground">
        {/* Title bar — draggable on desktop */}
        <div
          className="flex shrink-0 items-center gap-2 border-b border-border bg-secondary/30 px-3 py-2"
          {...(desktop ? { "data-tauri-drag-region": true } : {})}
        >
          <BookOpenText className="h-4 w-4 text-muted-foreground" />
          <span className="flex-1 truncate text-sm font-medium">{t("help.title")}</span>
          <IconTip label={t("common.close", "Close")}>
            <button
              type="button"
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
              onClick={handleClose}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
        <div className="min-h-0 flex-1">
          <HelpView initialTarget={initialTarget} navigateSignal={navigateSignal} />
        </div>
        <Toaster />
      </div>
    </TooltipProvider>
  )
}
