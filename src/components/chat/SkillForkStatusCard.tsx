import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { ArrowUpRight, CheckCircle2, Loader2, XCircle } from "lucide-react"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { SubagentEvent, SubagentRun } from "@/types/chat"

interface SkillForkStatusCardProps {
  runId?: string | null
  skillName?: string | null
  onViewChildSession?: (sessionId: string) => void
}

type SkillForkStatus = SubagentRun["status"]

export default function SkillForkStatusCard({
  runId,
  skillName,
  onViewChildSession,
}: SkillForkStatusCardProps) {
  const { t } = useTranslation()
  const safeRunId = runId?.trim() ?? ""
  const safeSkillName = skillName?.trim() || t("skills.defaultName", { defaultValue: "skill" })
  const [status, setStatus] = useState<SkillForkStatus>("spawning")
  const [error, setError] = useState<string | null>(null)
  const [childSessionId, setChildSessionId] = useState<string | null>(null)

  useEffect(() => {
    if (!safeRunId) return
    let cancelled = false
    getTransport()
      .call<SubagentRun | null>("get_subagent_run", { runId: safeRunId })
      .then((run) => {
        if (cancelled || !run) return
        setStatus(run.status)
        setError(run.error ?? null)
        setChildSessionId(run.childSessionId ?? null)
      })
      .catch(() => {})

    const unlisten = getTransport().listen("subagent_event", (raw) => {
      const event = raw as SubagentEvent
      if (event.runId !== safeRunId) return
      setStatus(event.status)
      setError(event.error ?? null)
      setChildSessionId(event.childSessionId ?? null)
    })

    return () => {
      cancelled = true
      unlisten()
    }
  }, [safeRunId])

  const isRunning = status === "queued" || status === "spawning" || status === "running"
  const isFailed = status === "error" || status === "timeout" || status === "killed"
  const StatusIcon = isRunning ? Loader2 : isFailed ? XCircle : CheckCircle2

  if (!safeRunId) {
    return (
      <div className="w-full max-w-[520px] rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive">
        {t("skills.chatFork.missingRunId", { defaultValue: "Skill run id is missing." })}
      </div>
    )
  }

  return (
    <div className="w-full max-w-[520px] rounded-lg border border-border-soft bg-surface-panel p-3 text-sm shadow-sm">
      <div className="flex items-start gap-2.5">
        <div
          className={cn(
            "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md",
            isRunning
              ? "bg-primary/10 text-primary"
              : isFailed
                ? "bg-destructive/10 text-destructive"
                : "bg-emerald-500/10 text-emerald-600 dark:text-emerald-300",
          )}
        >
          <StatusIcon className={cn("h-4 w-4", isRunning && "animate-spin")} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium text-foreground">
            {t("skills.chatFork.title", {
              skill: safeSkillName,
              defaultValue: `Skill ${safeSkillName}`,
            })}
          </div>
          <div className="mt-1 text-xs text-muted-foreground">{statusText(status, t)}</div>
          {error && (
            <div className="mt-2 rounded-md border border-destructive/20 bg-destructive/5 px-2 py-1.5 text-xs text-destructive">
              {error}
            </div>
          )}
          <div className="mt-3 flex items-center justify-between gap-2">
            <code className="truncate text-[11px] text-muted-foreground">
              {safeRunId.slice(0, 8)}
            </code>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              disabled={!childSessionId || !onViewChildSession}
              onClick={() => {
                if (childSessionId) onViewChildSession?.(childSessionId)
              }}
            >
              <ArrowUpRight className="mr-1.5 h-3.5 w-3.5" />
              {t("subagent.openSession", { defaultValue: "Open Session" })}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}

function statusText(status: SkillForkStatus, t: ReturnType<typeof useTranslation>["t"]): string {
  switch (status) {
    case "queued":
    case "spawning":
      return t("skills.chatFork.spawning", {
        defaultValue: "Starting skill sub-agent...",
      })
    case "running":
      return t("skills.chatFork.running", {
        defaultValue: "Skill sub-agent is running. The result will be injected here.",
      })
    case "completed":
      return t("skills.chatFork.completed", {
        defaultValue: "Completed. The result has been injected into this chat.",
      })
    case "error":
      return t("skills.chatFork.error", { defaultValue: "Failed." })
    case "timeout":
      return t("skills.chatFork.timeout", { defaultValue: "Timed out." })
    case "killed":
      return t("skills.chatFork.killed", { defaultValue: "Cancelled." })
  }
}
