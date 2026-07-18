import { useTranslation } from "react-i18next"
import { Monitor } from "lucide-react"
import { FRAME_CAPTURE_FAILED } from "@/lib/frame-store"
import { useMacControlFrame } from "@/hooks/useMacControlFrame"
import { usePanelActionHistory } from "@/hooks/usePanelActionHistory"
import { useReplaySelection } from "@/hooks/useReplaySelection"
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
  const { replay, replayActionId, onSelect } = useReplaySelection(entries)

  const title = frame?.frontmostApp?.name || t("settings.macControl.title")

  const preview = (
    <FramePreview
      jpegBase64={frame?.jpegBase64}
      alt={title}
      widthPx={frame?.widthPx}
      heightPx={frame?.heightPx}
      emptyText={t("settings.macControl.messages.blocked")}
      errorText={error === FRAME_CAPTURE_FAILED ? t("chat.browserPanel.captureFailed") : error}
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
      <PanelActionTimeline entries={entries} replayActionId={replayActionId} onSelect={onSelect} />
    </>
  )
}
