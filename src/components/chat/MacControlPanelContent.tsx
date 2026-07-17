import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Monitor } from "lucide-react"
import { useMacControlFrame } from "@/hooks/useMacControlFrame"
import { usePanelActionHistory, type PanelActionEntry } from "@/hooks/usePanelActionHistory"
import { ControlPanelHeader } from "./right-panel/ControlPanelHeader"
import { FramePreview } from "./right-panel/FramePreview"
import { PanelActionTimeline } from "./right-panel/PanelActionTimeline"
import { MacQuickBar } from "./right-panel/PanelQuickBar"
import { PanelSessionStats } from "./right-panel/PanelSessionStats"

export interface MacControlPanelContentProps {
  variant: "docked" | "floating"
  sessionId?: string | null
  active?: boolean
  onClose: () => void
  onFloat?: () => void
}

/** Single source of truth for the mac control panel UI (docked + floating). */
export function MacControlPanelContent({
  variant,
  sessionId,
  active = true,
  onClose,
  onFloat,
}: MacControlPanelContentProps) {
  const { t } = useTranslation()
  const { frame, error, refresh, setDisplayId, displayId } = useMacControlFrame({
    pollKey: variant,
    pollActive: active,
  })
  const { entries, stats } = usePanelActionHistory("mac-control", sessionId)
  const [replayEntry, setReplayEntry] = useState<PanelActionEntry | null>(null)

  const title = frame?.frontmostApp?.name || t("settings.macControl.title")
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
      alt={title}
      widthPx={frame?.widthPx}
      heightPx={frame?.heightPx}
      emptyText={t("settings.macControl.messages.blocked")}
      errorText={error}
      metaText={
        frame
          ? `${frame.widthPx}x${frame.heightPx} · ${new Date(frame.capturedAt).toLocaleTimeString()}`
          : null
      }
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
        icon={<Monitor className="h-4 w-4 text-muted-foreground" />}
        title={title}
        onFloat={onFloat}
        onRefresh={() => void refresh()}
        onClose={onClose}
      />
      {preview}
      <MacQuickBar
        displayId={displayId}
        onDisplayChange={setDisplayId}
        onCaptureNow={() => void refresh()}
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
