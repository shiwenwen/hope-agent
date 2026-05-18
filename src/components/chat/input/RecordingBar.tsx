import { Check, Loader2, X } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"

import { Waveform } from "./Waveform"

interface RecordingBarProps {
  /** True once stop has been clicked and we're waiting on STT. */
  transcribing: boolean
  durationMs: number
  levels: number[]
  /** Discard the recording without running STT. */
  onCancel: () => void
  /** Stop the recording and submit for transcription. */
  onStop: () => void
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${s.toString().padStart(2, "0")}`
}

/**
 * Bottom-toolbar replacement shown while voice recording is active.
 * Layout (matches the reference UX):
 *
 *     [✕ cancel] [▁▃▅▇▄▂▁ waveform fills available width] 0:11 [✓ stop]
 *
 * `transcribing` swaps the stop button for a spinner — the user has
 * already finished talking, we're waiting on the STT round-trip.
 */
export function RecordingBar({
  transcribing,
  durationMs,
  levels,
  onCancel,
  onStop,
}: RecordingBarProps) {
  const { t } = useTranslation()

  return (
    <div className="flex items-center gap-2 px-2 pb-2 animate-in fade-in-0 slide-in-from-bottom-1 duration-150">
      <IconTip label={t("voice.cancel")}>
        <Button
          size="icon"
          variant="ghost"
          className="h-8 w-8 rounded-full text-muted-foreground hover:text-destructive shrink-0"
          onClick={onCancel}
          disabled={transcribing}
          aria-label={t("voice.cancel")}
        >
          <X className="h-4 w-4" />
        </Button>
      </IconTip>

      <Waveform levels={levels} className="flex-1 min-w-0" />

      <span className="font-mono text-xs tabular-nums text-muted-foreground min-w-[40px] text-right">
        {formatDuration(durationMs)}
      </span>

      {transcribing ? (
        <Button
          size="icon"
          variant="default"
          className="h-8 w-8 rounded-full shrink-0"
          disabled
          aria-label={t("voice.processing")}
        >
          <Loader2 className="h-4 w-4 animate-spin" />
        </Button>
      ) : (
        <IconTip label={t("voice.stop")}>
          <Button
            size="icon"
            variant="default"
            className="h-8 w-8 rounded-full shrink-0"
            onClick={onStop}
            aria-label={t("voice.stop")}
          >
            <Check className="h-4 w-4" />
          </Button>
        </IconTip>
      )}
    </div>
  )
}
