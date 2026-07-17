import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Globe } from "lucide-react"
import { cn } from "@/lib/utils"
import { useBrowserFrame } from "@/hooks/useBrowserFrame"
import { usePanelActionHistory, type PanelActionEntry } from "@/hooks/usePanelActionHistory"
import { ControlPanelHeader } from "./right-panel/ControlPanelHeader"
import { FramePreview } from "./right-panel/FramePreview"
import { PanelActionTimeline } from "./right-panel/PanelActionTimeline"
import { BrowserQuickBar } from "./right-panel/PanelQuickBar"
import { PanelSessionStats } from "./right-panel/PanelSessionStats"

export interface BrowserPanelContentProps {
  variant: "docked" | "floating"
  sessionId?: string | null
  /** False while the docked shell is collapsed — pauses active polling. */
  active?: boolean
  onClose: () => void
  onFloat?: () => void
}

/**
 * Single source of truth for the browser panel UI, rendered by both the
 * docked RightPanelShell container and the floating window. The floating
 * variant is the compact mirror (header text lives in the window title bar);
 * the docked variant adds the quick bar / stats / execution timeline.
 */
export function BrowserPanelContent({
  variant,
  sessionId,
  active = true,
  onClose,
  onFloat,
}: BrowserPanelContentProps) {
  const { t } = useTranslation()
  const [paused, setPaused] = useState(false)
  const { frame, error, refresh } = useBrowserFrame({
    sessionId,
    pollKey: variant,
    pollActive: active && !paused,
  })
  const { entries, stats } = usePanelActionHistory("browser", sessionId)
  const [replayEntry, setReplayEntry] = useState<PanelActionEntry | null>(null)

  const replay =
    replayEntry?.thumbJpegBase64 != null
      ? {
          thumbJpegBase64: replayEntry.thumbJpegBase64,
          index: entries.findIndex((e) => e.actionId === replayEntry.actionId) + 1,
          total: entries.length,
          onExit: () => setReplayEntry(null),
        }
      : null

  const preview = (
    <FramePreview
      jpegBase64={frame?.jpegBase64}
      alt={frame?.title || t("chat.browserPanel.frameAlt")}
      emptyText={t("chat.browserPanel.idleHint")}
      errorText={error ? t("chat.browserPanel.captureFailed") : null}
      metaText={frame?.capturedAt ? new Date(frame.capturedAt).toLocaleTimeString() : null}
      variant={variant}
      replay={replay}
    />
  )

  if (variant === "floating") {
    return preview
  }

  return (
    <>
      <ControlPanelHeader
        icon={<Globe className="h-4 w-4 text-muted-foreground" />}
        title={frame?.title || t("chat.browserPanel.idleTitle")}
        badge={
          frame?.backend ? (
            <span
              className={cn(
                "rounded px-1.5 py-0.5 text-[10px] font-medium uppercase",
                frame.backend === "mcp"
                  ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                  : "bg-blue-500/15 text-blue-700 dark:text-blue-300",
              )}
            >
              {frame.backend}
            </span>
          ) : undefined
        }
        onFloat={onFloat}
        onRefresh={() => void refresh()}
        onClose={onClose}
      />
      {frame?.url && (
        <div className="truncate bg-muted/40 px-3 py-1 text-xs text-muted-foreground">
          {frame.url}
        </div>
      )}
      {preview}
      <BrowserQuickBar
        sessionId={sessionId}
        currentUrl={frame?.url}
        paused={paused}
        onTogglePaused={() => setPaused((p) => !p)}
      />
      <PanelSessionStats {...stats} />
      <PanelActionTimeline
        entries={entries}
        replayActionId={replayEntry?.actionId}
        onSelect={(entry) =>
          setReplayEntry((prev) =>
            prev?.actionId === entry.actionId ? null : entry.thumbJpegBase64 ? entry : prev,
          )
        }
      />
    </>
  )
}
