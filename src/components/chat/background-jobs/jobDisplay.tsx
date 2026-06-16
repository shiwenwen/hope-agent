import type { TFunction } from "i18next"
import { useTranslation } from "react-i18next"
import { Bot, Layers, Loader2, Terminal, type LucideIcon } from "lucide-react"

import { cn } from "@/lib/utils"
import type {
  BackgroundJobSnapshot,
  BackgroundJobStatus,
} from "@/types/background-jobs"

// Shared display helpers for the R4 background-jobs surfaces (the dedicated
// panel + the simplified workspace section) so labels / chips / icons stay
// identical between them.

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

/** Human display label: exec command head / tool name; localized for group/subagent. */
export function backgroundJobLabel(job: BackgroundJobSnapshot, t: TFunction): string {
  if (job.label) return job.label
  if (job.kind === "group") return t("backgroundJobs.kindGroup", "任务组")
  if (job.kind === "subagent") return t("backgroundJobs.kindSubagent", "子智能体")
  return job.tool
}

export function backgroundJobKindIcon(kind: BackgroundJobSnapshot["kind"]): LucideIcon {
  switch (kind) {
    case "group":
      return Layers
    case "subagent":
      return Bot
    default:
      return Terminal
  }
}

export function BackgroundJobStatusChip({ status }: { status: BackgroundJobStatus }) {
  const { t } = useTranslation()
  const tone = STATUS_TONE[status] ?? "muted"
  const active = status === "running" || status === "cancelling"
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium",
        TONE_CLASS[tone],
      )}
    >
      {active && <Loader2 className="h-2.5 w-2.5 animate-spin" />}
      {t(`backgroundJobs.status.${status}`, status)}
    </span>
  )
}
