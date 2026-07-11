import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import { ArrowDown, ArrowUpRight, ChevronRight, XCircle } from "lucide-react"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import {
  type BackgroundJobSnapshot,
  type BackgroundJobStatus,
  backgroundJobLabel,
  isBackgroundJobActive,
  isBackgroundJobCancellable,
} from "@/types/background-jobs"
import { BackgroundJobKindIcon, BackgroundJobStatusChip } from "./jobDisplay"
import { resolveBackgroundSubagentSessionId } from "./subagentSession"
import { isScrolledNearBottom, normalizeTerminalText, parseAnsiSegments } from "./terminalOutput"

const noopJobExpandedChange = () => {}

function mergeJobSnapshot(
  job: BackgroundJobSnapshot,
  detail?: BackgroundJobSnapshot,
): BackgroundJobSnapshot {
  if (!detail) return job

  const jobActive = isBackgroundJobActive(job)
  const detailActive = isBackgroundJobActive(detail)

  if (!jobActive) {
    return job
  }
  if (!detailActive) {
    return {
      ...job,
      ...detail,
      error: detail.error ?? job.error,
      resultPreview: detail.resultPreview ?? job.resultPreview,
      resultPath: detail.resultPath ?? job.resultPath,
      outputTail: null,
    }
  }

  return {
    ...job,
    ...detail,
    status:
      job.status === "cancelling" && isBackgroundJobCancellable(detail)
        ? job.status
        : detail.status,
    error: detail.error ?? job.error,
    resultPreview: detail.resultPreview ?? job.resultPreview,
    resultPath: detail.resultPath ?? job.resultPath,
    outputTail: detail.outputTail ?? job.outputTail,
  }
}

function defaultJobExpanded(job: BackgroundJobSnapshot): boolean {
  return job.status !== "completed"
}

function BackgroundJobRow({
  job,
  detail,
  expanded,
  onExpandedChange,
  onCancel,
  onViewSubagentSession,
  viewing,
}: {
  job: BackgroundJobSnapshot
  detail?: BackgroundJobSnapshot
  expanded: boolean
  onExpandedChange: (jobId: string, expanded: boolean) => void
  onCancel: (jobId: string) => void
  onViewSubagentSession?: (job: BackgroundJobSnapshot) => void
  viewing?: boolean
}) {
  const { t } = useTranslation()
  const merged = mergeJobSnapshot(job, detail)
  const label = backgroundJobLabel(merged, t)
  const showGroupProgress =
    merged.kind === "group" && merged.childCount != null && merged.childCount > 0
  const groupDone = merged.childrenTerminal ?? 0
  const groupTotal = merged.childCount ?? 0
  const groupPct = groupTotal > 0 ? Math.round((groupDone / groupTotal) * 100) : 0
  const cancellable = isBackgroundJobCancellable(merged)
  const canViewSubagentSession =
    merged.kind === "subagent" && !!merged.subagentRunId && !!onViewSubagentSession
  const liveOutput = isBackgroundJobActive(merged) ? merged.outputTail : null
  const outputText = liveOutput || merged.resultPreview || merged.error
  const outputLabel = liveOutput
    ? t("backgroundJobs.liveOutput", "实时输出")
    : merged.error
      ? t("backgroundJobs.errorOutput", "错误")
      : t("backgroundJobs.resultPreview", "结果预览")
  const toggleLabel = expanded
    ? t("backgroundJobs.collapseJob", "收起任务")
    : t("backgroundJobs.expandJob", "展开任务")

  return (
    <div className="flex flex-col gap-1 rounded-md border border-border/50 bg-secondary/30 px-2.5 py-1.5">
      <div className="flex items-center gap-2">
        <IconTip label={toggleLabel}>
          <button
            type="button"
            onClick={() => onExpandedChange(merged.jobId, !expanded)}
            className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-foreground"
            aria-label={toggleLabel}
            aria-expanded={expanded}
          >
            <ChevronRight
              className={cn(
                "h-3.5 w-3.5 transition-transform duration-200",
                expanded && "rotate-90",
              )}
            />
          </button>
        </IconTip>
        <BackgroundJobKindIcon
          kind={merged.kind}
          className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
        />
        <IconTip label={label}>
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/90">
            {label}
          </span>
        </IconTip>
        <BackgroundJobStatusChip status={merged.status} />
        {canViewSubagentSession && (
          <IconTip label={t("subagent.viewChildSession", "查看子会话")}>
            <button
              type="button"
              onClick={() => onViewSubagentSession(merged)}
              disabled={viewing}
              className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-foreground disabled:cursor-wait disabled:opacity-50"
              aria-label={t("subagent.viewChildSession", "查看子会话")}
            >
              <ArrowUpRight className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
        {cancellable && (
          <IconTip label={t("common.cancel", "取消")}>
            <button
              type="button"
              onClick={() => onCancel(merged.jobId)}
              className="rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-red-500"
              aria-label={t("common.cancel", "取消")}
            >
              <XCircle className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
      </div>
      <AnimatedCollapse open={expanded}>
        <div className="space-y-1">
          {showGroupProgress && (
            <div className="flex items-center gap-2">
              <div className="h-1 flex-1 overflow-hidden rounded-full bg-secondary">
                <div
                  className="h-full rounded-full bg-blue-500 transition-all duration-300"
                  style={{ width: `${groupPct}%` }}
                />
              </div>
              <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
                {groupDone}/{groupTotal}
              </span>
            </div>
          )}
          {outputText && <JobOutputBlock label={outputLabel} text={outputText} />}
          {merged.resultPath && (
            <div
              className="truncate text-[10px] text-muted-foreground/75"
              data-ha-title-tip={merged.resultPath}
            >
              {t("backgroundJobs.outputFile", "完整结果")}: {merged.resultPath}
            </div>
          )}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function JobOutputBlock({ label, text }: { label: string; text: string }) {
  const { t } = useTranslation()
  const outputRef = useRef<HTMLPreElement | null>(null)
  const stickToBottomRef = useRef(true)
  const [showJumpToBottom, setShowJumpToBottom] = useState(false)
  const normalizedText = useMemo(() => normalizeTerminalText(text), [text])
  const segments = useMemo(() => parseAnsiSegments(normalizedText), [normalizedText])

  const scrollToBottom = useCallback(() => {
    const node = outputRef.current
    if (!node) return
    node.scrollTop = node.scrollHeight
    stickToBottomRef.current = true
    setShowJumpToBottom(false)
  }, [])

  const updateStickiness = useCallback(() => {
    const node = outputRef.current
    if (!node) return
    const nearBottom = isScrolledNearBottom(node)
    stickToBottomRef.current = nearBottom
    setShowJumpToBottom(!nearBottom)
  }, [])

  useLayoutEffect(() => {
    const node = outputRef.current
    if (node && stickToBottomRef.current) {
      node.scrollTop = node.scrollHeight
    }
  }, [normalizedText])

  return (
    <div className="mt-1 border-t border-border/40 pt-1">
      <div className="pb-1 text-[10px] text-muted-foreground">{label}</div>
      <div className="relative rounded-md border border-border/40 bg-background/80">
        <pre
          ref={outputRef}
          onScroll={updateStickiness}
          className="max-h-36 overflow-auto whitespace-pre-wrap break-words px-2 py-1.5 font-mono text-[10px] leading-relaxed text-foreground/85"
        >
          {segments.map((segment, index) => (
            <span key={index} className={segment.className}>
              {segment.text}
            </span>
          ))}
        </pre>
        {showJumpToBottom && (
          <IconTip label={t("backgroundJobs.jumpToBottom", "跳到底部")}>
            <button
              type="button"
              onClick={scrollToBottom}
              className="absolute bottom-1.5 right-1.5 inline-flex h-6 w-6 items-center justify-center rounded-md border border-border/60 bg-background/95 text-muted-foreground shadow-sm transition-colors hover:bg-secondary hover:text-foreground"
              aria-label={t("backgroundJobs.jumpToBottom", "跳到底部")}
            >
              <ArrowDown className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        )}
      </div>
    </div>
  )
}

export function SessionBackgroundJobsList({
  jobs,
  jobExpansionOverrides,
  onJobExpandedChange,
  onViewSubagentSession,
  limit,
}: {
  jobs: BackgroundJobSnapshot[]
  jobExpansionOverrides?: Record<string, boolean>
  onJobExpandedChange?: (jobId: string, expanded: boolean) => void
  onViewSubagentSession?: (sessionId: string) => void
  limit?: number
}) {
  const [pendingCancel, setPendingCancel] = useState<Set<string>>(new Set())
  const [pendingViewRunIds, setPendingViewRunIds] = useState<Set<string>>(new Set())
  const [details, setDetails] = useState<Record<string, BackgroundJobSnapshot>>({})

  const clearPending = useCallback((jobId: string) => {
    setPendingCancel((prev) => {
      if (!prev.has(jobId)) return prev
      const next = new Set(prev)
      next.delete(jobId)
      return next
    })
  }, [])

  const handleCancel = useCallback(
    (jobId: string) => {
      setPendingCancel((prev) => new Set(prev).add(jobId))
      getTransport()
        .call<{ accepted?: boolean }>("cancel_runtime_task", { kind: "async_job", id: jobId })
        .then((res) => {
          // Backend declined (job already gone / not found / DB unavailable): no
          // `job:*` event will arrive to reconcile, so revert the optimistic
          // override and re-expose the cancel button instead of getting stuck.
          if (res?.accepted === false) clearPending(jobId)
        })
        .catch(() => clearPending(jobId))
    },
    [clearPending],
  )

  const handleViewSubagentSession = useCallback(
    (job: BackgroundJobSnapshot) => {
      const runId = job.subagentRunId
      if (!runId || !onViewSubagentSession) return
      setPendingViewRunIds((prev) => new Set(prev).add(runId))
      resolveBackgroundSubagentSessionId(runId)
        .then((childSessionId) => {
          if (childSessionId) onViewSubagentSession(childSessionId)
        })
        .catch(() => {})
        .finally(() => {
          setPendingViewRunIds((prev) => {
            if (!prev.has(runId)) return prev
            const next = new Set(prev)
            next.delete(runId)
            return next
          })
        })
    },
    [onViewSubagentSession],
  )

  // Optimistically show "cancelling" for any cancellable job whose cancel is in
  // flight (covers running / queued / awaiting_approval — the full cancellable
  // set). The override is gated on `isBackgroundJobCancellable`, so once a refetch
  // flips the job terminal a lingering id is inert (no stale "cancelling" pin) —
  // and `accepted === false` already self-heals via `clearPending`.
  const visibleJobs = useMemo(
    () =>
      jobs.map((j) =>
        pendingCancel.has(j.jobId) && isBackgroundJobCancellable(j)
          ? { ...j, status: "cancelling" as BackgroundJobStatus }
          : j,
      ),
    [jobs, pendingCancel],
  )
  const listedJobs = useMemo(
    () => (typeof limit === "number" ? visibleJobs.slice(0, limit) : visibleJobs),
    [visibleJobs, limit],
  )
  const activeJobIds = useMemo(
    () => listedJobs.filter(isBackgroundJobActive).map((j) => j.jobId),
    [listedJobs],
  )
  const activeJobKey = activeJobIds.join("|")

  useEffect(() => {
    if (activeJobIds.length === 0) return

    let alive = true
    let timer: ReturnType<typeof setTimeout> | null = null

    const fetchDetails = () => {
      Promise.all(
        activeJobIds.map((jobId) =>
          getTransport()
            .call<BackgroundJobSnapshot | null>("get_background_job", { jobId })
            .catch(() => null),
        ),
      )
        .then((rows) => {
          if (!alive) return
          const byId = new Map(
            rows
              .filter((row): row is BackgroundJobSnapshot => !!row)
              .map((row) => [row.jobId, row]),
          )
          setDetails((prev) => {
            const next: Record<string, BackgroundJobSnapshot> = {}
            for (const job of listedJobs) {
              next[job.jobId] = byId.get(job.jobId) ?? prev[job.jobId] ?? job
            }
            return next
          })
        })
        .finally(() => {
          if (alive) timer = setTimeout(fetchDetails, 1000)
        })
    }

    fetchDetails()

    return () => {
      alive = false
      if (timer) clearTimeout(timer)
    }
  }, [activeJobKey, activeJobIds, listedJobs])

  if (listedJobs.length === 0) return null

  return (
    <div className="space-y-1">
      {listedJobs.map((job) => {
        const detail = details[job.jobId]
        const merged = mergeJobSnapshot(job, detail)

        return (
          <BackgroundJobRow
            key={job.jobId}
            job={job}
            detail={detail}
            expanded={jobExpansionOverrides?.[job.jobId] ?? defaultJobExpanded(merged)}
            onExpandedChange={onJobExpandedChange ?? noopJobExpandedChange}
            onCancel={handleCancel}
            onViewSubagentSession={handleViewSubagentSession}
            viewing={!!job.subagentRunId && pendingViewRunIds.has(job.subagentRunId)}
          />
        )
      })}
    </div>
  )
}
