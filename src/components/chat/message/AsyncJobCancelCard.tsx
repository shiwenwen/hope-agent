import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { XCircle } from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import { IconTip } from "@/components/ui/tooltip"

const ASYNC_JOB_TERMINAL = new Set(["completed", "failed", "timed_out", "interrupted", "cancelled"])

function parseAsyncJobStarted(
  result: string | undefined,
): { jobId: string; tool: string; origin?: string } | null {
  if (!result) return null
  try {
    const parsed = JSON.parse(result) as {
      job_id?: string
      status?: string
      tool?: string
      origin?: string
    }
    if (parsed.status !== "started" || !parsed.job_id || !parsed.tool) return null
    return { jobId: parsed.job_id, tool: parsed.tool, origin: parsed.origin }
  } catch {
    return null
  }
}

export default function AsyncJobCancelCard({
  result,
  className,
}: {
  result: string | undefined
  className?: string
}) {
  const { t } = useTranslation()
  const asyncJob = parseAsyncJobStarted(result)
  const asyncJobId = asyncJob?.jobId
  const [asyncJobState, setAsyncJobState] = useState<{ jobId: string; status: string } | null>(null)
  const asyncJobStatus =
    asyncJobState && asyncJobState.jobId === asyncJobId
      ? asyncJobState.status
      : asyncJobId
        ? "running"
        : null

  useEffect(() => {
    if (!asyncJobId) return
    const unlistenCompleted = getTransport().listen("async_tool_job:completed", (raw) => {
      const payload = raw as { job_id?: string; status?: string }
      if (payload.job_id === asyncJobId && payload.status) {
        setAsyncJobState({ jobId: asyncJobId, status: payload.status })
      }
    })
    const unlistenUpdated = getTransport().listen("async_tool_job:updated", (raw) => {
      const payload = raw as { job_id?: string; status?: string }
      if (payload.job_id === asyncJobId && payload.status) {
        setAsyncJobState({ jobId: asyncJobId, status: payload.status })
      }
    })
    return () => {
      unlistenCompleted()
      unlistenUpdated()
    }
  }, [asyncJobId])

  const handleCancelAsyncJob = async () => {
    if (!asyncJobId || ASYNC_JOB_TERMINAL.has(asyncJobStatus || "")) return
    setAsyncJobState({ jobId: asyncJobId, status: "cancelling" })
    try {
      const cancelResult = await getTransport().call<{ status?: string }>("cancel_runtime_task", {
        kind: "async_job",
        id: asyncJobId,
      })
      if (cancelResult.status) {
        setAsyncJobState({ jobId: asyncJobId, status: cancelResult.status })
      }
    } catch {
      setAsyncJobState({ jobId: asyncJobId, status: "running" })
    }
  }

  if (!asyncJob) return null

  return (
    <div
      className={cn(
        "mb-1 flex items-center gap-2 rounded-md border border-border/40 bg-secondary/30 px-2 py-1 text-[11px] text-muted-foreground",
        className,
      )}
    >
      <span className="font-mono truncate">{asyncJob.jobId}</span>
      <span className="shrink-0">{asyncJobStatus || "running"}</span>
      {!ASYNC_JOB_TERMINAL.has(asyncJobStatus || "") && (
        <IconTip label={t("common.cancel")}>
          <button
            type="button"
            className="ml-auto rounded p-0.5 text-muted-foreground/60 transition-colors hover:bg-secondary hover:text-red-500"
            onClick={handleCancelAsyncJob}
            aria-label={t("common.cancel")}
          >
            <XCircle className="h-3 w-3" />
          </button>
        </IconTip>
      )}
    </div>
  )
}
