import { useState, useCallback } from "react"
import { Pause, Play, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import { getTransport } from "@/lib/transport"
import type { Team } from "./teamTypes"

interface TeamToolbarProps {
  team: Team
  onPause: () => void
  onResume: () => void
  onDissolve: () => void
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
}: TeamToolbarProps) {
  const { t } = useTranslation()
  const [confirmDissolve, setConfirmDissolve] = useState(false)

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

  return (
    <div className="flex items-center gap-2 px-3 py-2 border-b border-border">
      {/* Team name */}
      <span className="text-sm font-semibold text-foreground truncate">
        {team.name}
      </span>

      {/* Status badge */}
      <span
        className={cn(
          "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium",
          STATUS_STYLES[team.status] ?? STATUS_STYLES.dissolved,
        )}
      >
        {team.status}
      </span>

      <div className="flex-1" />

      {/* Pause / Resume */}
      {(isActive || isPaused) && (
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={isPaused ? onResume : onPause}
        >
          {isPaused ? (
            <>
              <Play className="mr-1 h-3 w-3" />
              {t("team.resume", "Resume")}
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
          {confirmDissolve
            ? t("team.confirmDissolve", "Confirm?")
            : t("team.dissolve", "Dissolve")}
        </Button>
      )}
    </div>
  )
}
