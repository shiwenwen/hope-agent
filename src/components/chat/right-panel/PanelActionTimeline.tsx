import { memo, useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import {
  AppWindow,
  Camera,
  CircleDot,
  Command,
  Globe,
  Keyboard,
  MousePointer,
  MoveVertical,
} from "lucide-react"
import { cn } from "@/lib/utils"
import { formatDuration } from "@/components/chat/chatUtils"
import type { PanelActionEntry } from "@/hooks/usePanelActionHistory"
import { PANEL_SCROLL_FADE } from "./panelFade"

const ENTRY_ICON_CLASS = "h-3.5 w-3.5 shrink-0 text-muted-foreground"

function entryIcon(entry: PanelActionEntry) {
  const op = entry.op ?? entry.action
  if (op.includes("click") || op === "hover" || op === "drag")
    return <MousePointer className={ENTRY_ICON_CLASS} />
  if (op === "press" || op === "hotkey" || op === "key")
    return <Command className={ENTRY_ICON_CLASS} />
  if (op === "fill" || op === "type" || op === "paste" || op === "set_value")
    return <Keyboard className={ENTRY_ICON_CLASS} />
  if (entry.action === "navigate" || entry.action === "tabs" || op === "go")
    return <Globe className={ENTRY_ICON_CLASS} />
  if (op === "scroll") return <MoveVertical className={ENTRY_ICON_CLASS} />
  if (op === "screenshot" || op === "image" || op === "pdf")
    return <Camera className={ENTRY_ICON_CLASS} />
  if (entry.action === "windows" || entry.action === "apps" || entry.action === "dock")
    return <AppWindow className={ENTRY_ICON_CLASS} />
  return <CircleDot className={ENTRY_ICON_CLASS} />
}

interface TimelineRowProps {
  entry: PanelActionEntry
  index: number
  selected: boolean
  onSelect: (entry: PanelActionEntry) => void
}

const TimelineRow = memo(function TimelineRow({
  entry,
  index,
  selected,
  onSelect,
}: TimelineRowProps) {
  const { t } = useTranslation()
  const op = entry.op ?? entry.action
  const label = t(`chat.controlPanel.actions.${op}`, op)
  const description = entry.target || entry.detail || entry.url || entry.app || ""
  return (
    <button
      type="button"
      onClick={() => onSelect(entry)}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left",
        selected ? "bg-secondary/70" : "hover:bg-secondary/40",
      )}
    >
      <span className="w-6 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground">
        {index + 1}
      </span>
      {entryIcon(entry)}
      <span className="min-w-0 flex-1 truncate text-xs">
        <span className="font-medium">{label}</span>
        {description && <span className="text-muted-foreground"> · {description}</span>}
        {!entry.ok && entry.error && (
          <span className="text-destructive"> · {entry.error}</span>
        )}
      </span>
      <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
        {formatDuration(entry.durationMs)}
      </span>
      <span
        className={cn(
          "h-1.5 w-1.5 shrink-0 rounded-full",
          entry.ok ? "bg-emerald-500" : "bg-destructive",
        )}
      />
      {entry.thumbJpegBase64 && (
        <img
          src={`data:image/jpeg;base64,${entry.thumbJpegBase64}`}
          alt=""
          className="h-7 w-10 shrink-0 rounded-sm border border-border/60 object-cover"
          draggable={false}
        />
      )}
    </button>
  )
})

interface PanelActionTimelineProps {
  entries: PanelActionEntry[]
  replayActionId?: string | null
  onSelect: (entry: PanelActionEntry) => void
}

/** Execution-path timeline. Sticks to the newest entry unless the user
 *  scrolled up or is replaying a step. */
export function PanelActionTimeline({
  entries,
  replayActionId,
  onSelect,
}: PanelActionTimelineProps) {
  const { t } = useTranslation()
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const stickToBottom = useRef(true)

  useEffect(() => {
    const node = scrollRef.current
    if (node && stickToBottom.current && !replayActionId) {
      node.scrollTop = node.scrollHeight
    }
  }, [entries.length, replayActionId])

  return (
    <div className="flex min-h-[120px] flex-1 flex-col overflow-hidden border-t border-border/60">
      <div className="px-3 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {t("chat.controlPanel.history.title")}
      </div>
      <div
        ref={scrollRef}
        onScroll={(e) => {
          const node = e.currentTarget
          stickToBottom.current = node.scrollHeight - node.scrollTop - node.clientHeight < 24
        }}
        className={cn("min-h-0 flex-1 overflow-y-auto px-1 pb-1", PANEL_SCROLL_FADE)}
      >
        {entries.length === 0 ? (
          <div className="flex h-full items-center justify-center px-6 py-4 text-center text-xs text-muted-foreground">
            {t("chat.controlPanel.history.empty")}
          </div>
        ) : (
          entries.map((entry, index) => (
            <TimelineRow
              key={entry.actionId}
              entry={entry}
              index={index}
              selected={entry.actionId === replayActionId}
              onSelect={onSelect}
            />
          ))
        )}
      </div>
    </div>
  )
}
