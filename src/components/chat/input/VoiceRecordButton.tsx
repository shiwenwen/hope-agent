import { Loader2, Mic, MicOff, Square } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { VoiceInputState } from "./useVoiceInput"

const isMac = typeof navigator !== "undefined" && navigator.platform.toUpperCase().includes("MAC")
// Press-to-talk hint surfaced in the idle tooltip; actual keydown / keyup
// wiring lives in ChatInput so the listener has access to start/stop.
const VOICE_PTT_SHORTCUT_LABEL = isMac ? "⌃⇧H" : "Ctrl+Shift+H"

interface VoiceRecordButtonProps {
  state: VoiceInputState
  durationMs: number
  audioLevel: number
  disabled?: boolean
  onStart: () => void
  onStop: () => void
  onCancel: () => void
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`
}

export function VoiceRecordButton({
  state,
  durationMs,
  audioLevel,
  disabled,
  onStart,
  onStop,
  onCancel,
}: VoiceRecordButtonProps) {
  const { t } = useTranslation()

  if (state === "recording") {
    // Recording: stop / cancel pair + live timer + audio-level pulse.
    const pulseScale = 1 + Math.min(0.4, audioLevel * 0.6)
    return (
      <div className="flex items-center gap-1 animate-in fade-in-0 zoom-in-90 duration-150">
        <span
          aria-hidden
          className="inline-block h-2 w-2 rounded-full bg-red-500 transition-transform"
          style={{ transform: `scale(${pulseScale})` }}
        />
        <span className="text-xs font-mono text-muted-foreground tabular-nums min-w-[42px]">
          {formatDuration(durationMs)}
        </span>
        <IconTip label={t("voice.cancel")}>
          <Button
            size="icon"
            variant="ghost"
            className="h-8 w-8 rounded-full text-muted-foreground hover:text-destructive"
            onClick={onCancel}
            aria-label={t("voice.cancel")}
          >
            <MicOff className="h-4 w-4" />
          </Button>
        </IconTip>
        <IconTip label={t("voice.stop")}>
          <Button
            size="icon"
            variant="destructive"
            className="h-8 w-8 rounded-full"
            onClick={onStop}
            aria-label={t("voice.stop")}
          >
            <Square className="h-3.5 w-3.5 fill-white stroke-white" />
          </Button>
        </IconTip>
      </div>
    )
  }

  if (state === "transcribing") {
    return (
      <Button
        size="icon"
        variant="ghost"
        className="h-8 w-8 rounded-full text-muted-foreground"
        disabled
        aria-label={t("voice.processing")}
      >
        <Loader2 className="h-4 w-4 animate-spin" />
      </Button>
    )
  }

  if (state === "requesting-permission") {
    return (
      <Button
        size="icon"
        variant="ghost"
        className="h-8 w-8 rounded-full text-muted-foreground"
        disabled
        aria-label={t("voice.record")}
      >
        <Loader2 className="h-4 w-4 animate-spin" />
      </Button>
    )
  }

  // idle / stopped / ready / error → ready-to-record state.
  return (
    <IconTip label={t("voice.recordTip", { shortcut: VOICE_PTT_SHORTCUT_LABEL })}>
      <Button
        size="icon"
        variant="ghost"
        className={cn(
          "h-8 w-8 rounded-full text-muted-foreground hover:text-foreground",
          state === "error" && "text-destructive hover:text-destructive",
        )}
        disabled={disabled}
        onClick={onStart}
        aria-label={t("voice.record")}
      >
        <Mic className="h-4 w-4" />
      </Button>
    </IconTip>
  )
}
