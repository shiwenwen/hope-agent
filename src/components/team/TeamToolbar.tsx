import { useState, useCallback } from "react"
import { Loader2, Pause, Play, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport-provider"
import type { ResumeTeamResult, Team } from "./teamTypes"

interface TeamToolbarProps {
  team: Team
  onPause: () => void
  onResume: () => void | Promise<void>
  onDissolve: () => void
  resumeResult?: ResumeTeamResult | null
  resumeNotNeeded?: boolean
}

const STATUS_STYLES: Record<string, string> = {
  active: "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400",
  paused: "bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400",
  dissolved: "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400",
}

export function TeamToolbar({
  team,
  onPause,
  onResume,
  onDissolve,
  resumeResult,
  resumeNotNeeded = false,
}: TeamToolbarProps) {
  const { t } = useTranslation()
  const [confirmDissolve, setConfirmDissolve] = useState(false)
  const [resuming, setResuming] = useState(false)

  const handleResume = useCallback(async () => {
    setResuming(true)
    try {
      await onResume()
    } finally {
      setResuming(false)
    }
  }, [onResume])

  const handleDissolve = useCallback(async () => {
    if (!confirmDissolve) {
      setConfirmDissolve(true)
      // Auto-reset after 3s
      setTimeout(() => setConfirmDissolve(false), 3000)
      return
    }
    try {
      await getTransport().call("dissolve_team", { teamId: team.teamId })
      onDissolve()
    } catch {
      // Error handled by transport
    }
    setConfirmDissolve(false)
  }, [confirmDissolve, team.teamId, onDissolve])

  const isPaused = team.status === "paused"
  const isActive = team.status === "active"
  const feedbackDisposition = resumeNotNeeded ? "no_op" : resumeResult?.disposition
  const resumeFeedback = feedbackDisposition
    ? feedbackDisposition === "no_op"
      ? t("team.resumeFeedback.noOpTitle", "No resume needed")
      : t("team.resumeFeedback.counts", {
          resumed: resumeResult?.resumedMemberCount ?? 0,
          failed: resumeResult?.failedMemberCount ?? 0,
          defaultValue: "{{resumed}} resumed · {{failed}} failed",
        })
    : null
  const resumeFeedbackTitle = feedbackDisposition
    ? feedbackDisposition === "resumed"
      ? t("team.resumeFeedback.resumedTitle", "Team resumed")
      : feedbackDisposition === "partial"
        ? t("team.resumeFeedback.partialTitle", "Team partially resumed")
        : feedbackDisposition === "refused"
          ? t("team.resumeFeedback.refusedTitle", "Team resume refused")
          : t(
              "team.resumeFeedback.noOpDescription",
              "All team members have already completed; no new attempts were started.",
            )
    : null

  return (
    <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
      {/* Team name */}
      <span className="text-sm font-semibold text-foreground truncate">{team.name}</span>

      {/* Status badge */}
      <span
        className={cn(
          "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium",
          STATUS_STYLES[team.status] ?? STATUS_STYLES.dissolved,
        )}
      >
        {t(`team.${team.status}`, { defaultValue: team.status.replaceAll("_", " ") })}
      </span>

      {feedbackDisposition && resumeFeedback && (
        <span
          role="status"
          aria-live="polite"
          aria-label={`${resumeFeedbackTitle} · ${resumeFeedback}`}
          className={cn(
            "max-w-44 shrink truncate rounded-full px-2 py-0.5 text-[10px] font-medium",
            feedbackDisposition === "resumed" &&
              "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400",
            feedbackDisposition === "partial" &&
              "bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400",
            feedbackDisposition === "refused" &&
              "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400",
            feedbackDisposition === "no_op" && "bg-secondary text-muted-foreground",
          )}
        >
          {resumeFeedback}
        </span>
      )}

      <div className="flex-1" />

      {/* Pause / Resume */}
      {(isActive || (isPaused && !resumeNotNeeded)) && (
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={isPaused ? handleResume : onPause}
          disabled={isPaused && resuming}
        >
          {isPaused ? (
            <>
              {resuming ? (
                <Loader2 className="mr-1 h-3 w-3 animate-spin" />
              ) : (
                <Play className="mr-1 h-3 w-3" />
              )}
              {resuming
                ? t("team.resumeFeedback.resuming", "Resuming…")
                : t("team.resume", "Resume")}
            </>
          ) : (
            <>
              <Pause className="mr-1 h-3 w-3" />
              {t("team.pause", "Pause")}
            </>
          )}
        </Button>
      )}

      {/* Dissolve */}
      {team.status !== "dissolved" && (
        <Button
          variant={confirmDissolve ? "destructive" : "ghost"}
          size="sm"
          className={cn(
            "h-7 px-2 text-xs",
            !confirmDissolve && "text-muted-foreground hover:text-red-500",
          )}
          onClick={handleDissolve}
        >
          <Trash2 className="mr-1 h-3 w-3" />
          {confirmDissolve ? t("team.confirmDissolve", "Confirm?") : t("team.dissolve", "Dissolve")}
        </Button>
      )}
    </div>
  )
}
