import { useTranslation } from "react-i18next"
import { Bot, Clock, Layers, Loader2, Terminal } from "lucide-react"

import { cn } from "@/lib/utils"
import type {
  BackgroundJobSnapshot,
  BackgroundJobStatus,
} from "@/types/background-jobs"

// Shared presentational components for the R4 background-jobs surfaces (the
// dedicated panel + the simplified workspace section) so the status chip / kind
// icon stay identical between them. Pure-function helpers (label derivation)
// live in `@/types/background-jobs` to keep this a components-only module
// (react-refresh requirement).

const STATUS_TONE: Record<
  BackgroundJobStatus,
  "muted" | "good" | "warn" | "danger" | "info"
> = {
  queued: "muted",
  running: "info",
  cancelling: "warn",
  awaiting_approval: "warn",
  completed: "good",
  failed: "danger",
  timed_out: "danger",
  interrupted: "muted",
  cancelled: "muted",
}

const TONE_CLASS: Record<string, string> = {
  muted: "border-border bg-muted/50 text-muted-foreground",
  good: "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warn: "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  danger: "border-destructive/35 bg-destructive/10 text-destructive",
  info: "border-blue-500/35 bg-blue-500/10 text-blue-700 dark:text-blue-300",
}

export function BackgroundJobKindIcon({
  kind,
  className,
}: {
  kind: BackgroundJobSnapshot["kind"]
  className?: string
}) {
  if (kind === "group") return <Layers className={className} />
  if (kind === "subagent") return <Bot className={className} />
  return <Terminal className={className} />
}

export function BackgroundJobStatusChip({ status }: { status: BackgroundJobStatus }) {
  const { t } = useTranslation()
  const tone = STATUS_TONE[status] ?? "muted"
  const spinning = status === "running" || status === "cancelling"
  // R8: a parked job is waiting on a human decision, not running — a static
  // clock (not the spinner) tells "needs your approval" apart from "in flight".
  const awaiting = status === "awaiting_approval"
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium",
        TONE_CLASS[tone],
      )}
    >
      {spinning && <Loader2 className="h-2.5 w-2.5 animate-spin" />}
      {awaiting && <Clock className="h-2.5 w-2.5" />}
      {t(`backgroundJobs.status.${status}`, status)}
    </span>
  )
}
