import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { Cpu, Layers, SquareTerminal, X, type LucideIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { type BackgroundJobSnapshot } from "@/types/background-jobs"
import {
  type LocalModelJobSnapshot,
  localModelJobPercent,
  phaseTranslationKey,
} from "@/types/local-model-jobs"
import { useLocalModelJobsMirror } from "./useLocalModelJobsMirror"
import { SessionBackgroundJobsList } from "./SessionBackgroundJobsList"

function SectionHeader({
  icon: Icon,
  title,
  count,
}: {
  icon: LucideIcon
  title: string
  count: number
}) {
  return (
    <div className="flex items-center gap-2 px-1 pb-1.5 pt-0.5">
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <span className="text-xs font-medium text-foreground/80">{title}</span>
      <span className="text-xs font-normal tabular-nums text-muted-foreground">{count}</span>
    </div>
  )
}

function EmptyHint({ children }: { children: ReactNode }) {
  return <div className="px-2 py-4 text-center text-xs text-muted-foreground/70">{children}</div>
}

function LocalModelJobRow({ job }: { job: LocalModelJobSnapshot }) {
  const { t } = useTranslation()
  const pct = localModelJobPercent(job)
  const phaseKey = phaseTranslationKey(job.phase)
  const phaseLabel = phaseKey ? t(phaseKey, job.phase) : job.phase
  return (
    <div className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/20 px-2.5 py-1.5">
      <div className="flex items-center gap-2">
        <Cpu className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <IconTip label={job.displayName || job.modelId}>
          <span className="min-w-0 flex-1 truncate text-[11px] text-foreground/85">
            {job.displayName || job.modelId}
          </span>
        </IconTip>
        <span className="shrink-0 text-[10px] text-muted-foreground/80">{phaseLabel}</span>
      </div>
      {pct != null && (
        <div className="flex items-center gap-2">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-secondary">
            <div
              className="h-full rounded-full bg-sky-500 transition-all duration-300"
              style={{ width: `${Math.round(pct)}%` }}
            />
          </div>
          <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {Math.round(pct)}%
          </span>
        </div>
      )}
    </div>
  )
}

/**
 * R4 background-jobs panel: a session's background jobs (cancellable) plus a
 * read-only mirror of the global local-model jobs (downloads / installs /
 * reembeds), so "什么在后台跑" lives in one place. Mounted as a sibling exclusive
 * right panel in {@link ChatScreen}.
 */
export default function BackgroundJobsPanel({
  jobs,
  jobExpansionOverrides,
  onJobExpandedChange,
  onClose,
  onViewSubagentSession,
}: {
  jobs: BackgroundJobSnapshot[]
  jobExpansionOverrides?: Record<string, boolean>
  onJobExpandedChange?: (jobId: string, expanded: boolean) => void
  onClose: () => void
  onViewSubagentSession?: (sessionId: string) => void
}) {
  const { t } = useTranslation()
  // The global model-job mirror only subscribes while this panel is mounted.
  const localModelJobs = useLocalModelJobsMirror()

  return (
    <div className="flex h-full min-h-0 w-full flex-col overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2">
        <Layers className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate text-sm font-medium">
          {t("backgroundJobs.panelTitle", "后台任务")}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="ml-auto h-7 w-7 shrink-0"
          onClick={onClose}
          aria-label={t("common.close", "关闭")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex-1 space-y-3 overflow-auto p-2">
        <div>
          <SectionHeader
            icon={SquareTerminal}
            title={t("backgroundJobs.sectionSession", "本会话")}
            count={jobs.length}
          />
          {jobs.length > 0 ? (
            <SessionBackgroundJobsList
              jobs={jobs}
              jobExpansionOverrides={jobExpansionOverrides}
              onJobExpandedChange={onJobExpandedChange}
              onViewSubagentSession={onViewSubagentSession}
            />
          ) : (
            <EmptyHint>{t("backgroundJobs.empty", "暂无后台任务")}</EmptyHint>
          )}
        </div>

        {localModelJobs.length > 0 && (
          <div>
            <SectionHeader
              icon={Cpu}
              title={t("backgroundJobs.sectionLocalModel", "本地模型")}
              count={localModelJobs.length}
            />
            <div className="space-y-1">
              {localModelJobs.map((job) => (
                <LocalModelJobRow key={job.jobId} job={job} />
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
