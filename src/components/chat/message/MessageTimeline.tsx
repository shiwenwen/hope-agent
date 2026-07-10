import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export type MessageTimelineTone =
  | "assistant"
  | "failed"
  | "muted"
  | "running"
  | "thinking"
  | "tool"
  | "user"

export type MessageTimelineMarkerAlign = "control" | "text"

const DOT_TONE_CLASSES: Record<MessageTimelineTone, string> = {
  assistant: "bg-teal-500",
  failed: "bg-red-500",
  muted: "bg-muted-foreground/55",
  running: "bg-blue-500",
  thinking: "bg-violet-500",
  tool: "bg-teal-500",
  user: "bg-sky-500",
}

const DOT_RIPPLE_CLASSES: Record<MessageTimelineTone, string> = {
  assistant: "bg-teal-400",
  failed: "bg-red-400",
  muted: "bg-muted-foreground/50",
  running: "bg-blue-400",
  thinking: "bg-violet-400",
  tool: "bg-teal-400",
  user: "bg-sky-400",
}

interface MessageTimelineProps {
  children: ReactNode
  className?: string
}

export function MessageTimeline({ children, className }: MessageTimelineProps) {
  return (
    <div
      className={cn(
        "relative grid w-full max-w-4xl grid-cols-[1rem_minmax(0,1fr)] gap-x-3",
        className,
      )}
    >
      {children}
    </div>
  )
}

interface MessageTimelineItemProps {
  children: ReactNode
  className?: string
  contentClassName?: string
  active?: boolean
  dense?: boolean
  markerAlign?: MessageTimelineMarkerAlign
  tone?: MessageTimelineTone
}

export function MessageTimelineItem({
  children,
  className,
  contentClassName,
  active = false,
  dense = false,
  markerAlign = "text",
  tone = "assistant",
}: MessageTimelineItemProps) {
  const markerOffsetClass =
    dense && markerAlign === "control"
      ? "pt-[0.625rem]"
      : dense
        ? "pt-[0.375rem]"
        : "pt-[0.45rem]"
  const lineStartClass =
    dense && markerAlign === "control"
      ? "top-[1.225rem]"
      : dense
        ? "top-[0.975rem]"
        : "top-[1.1rem]"

  return (
    <div
      className={cn(
        "group relative col-span-2 grid min-w-0 grid-cols-[1rem_minmax(0,1fr)] gap-x-3",
        dense ? "py-0.5" : "py-1.5",
        className,
      )}
    >
      <div className={cn("relative flex justify-center", markerOffsetClass)}>
        <span
          aria-hidden
          className={cn(
            "pointer-events-none absolute bottom-0 left-1/2 w-px -translate-x-1/2 bg-sky-500/18 dark:bg-sky-300/12 group-last:hidden",
            lineStartClass,
          )}
        />
        <span className="relative flex h-2.5 w-2.5 items-center justify-center">
          {active && (
            <>
              <span
                className={cn(
                  "absolute h-4 w-4 rounded-full opacity-70 animate-ping [animation-duration:1.6s]",
                  DOT_RIPPLE_CLASSES[tone],
                )}
              />
              <span
                className={cn(
                  "absolute h-5 w-5 rounded-full opacity-40 animate-ping [animation-delay:450ms] [animation-duration:1.8s]",
                  DOT_RIPPLE_CLASSES[tone],
                )}
              />
            </>
          )}
          <span
            className={cn(
              "relative z-10 h-2.5 w-2.5 rounded-full ring-[3px] ring-background",
              active && "animate-pulse",
              DOT_TONE_CLASSES[tone],
            )}
          />
        </span>
      </div>
      <div
        className={cn(
          "message-markdown-content min-w-0 break-words text-sm leading-relaxed text-foreground select-text",
          dense ? "pb-0.5" : "pb-2",
          contentClassName,
        )}
      >
        {children}
      </div>
    </div>
  )
}
