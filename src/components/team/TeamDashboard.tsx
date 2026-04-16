import { useMemo } from "react"
import { Users, Zap, Clock } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { Team, TeamMember, TeamTask } from "./teamTypes"
import { TeamMemberCard } from "./TeamMemberCard"

interface TeamDashboardProps {
  members: TeamMember[]
  tasks: TeamTask[]
  team: Team
  onViewSession?: (sessionId: string) => void
}

export function TeamDashboard({ members, tasks, team, onViewSession }: TeamDashboardProps) {
  const { t } = useTranslation()

  const stats = useMemo(() => {
    const activeMembers = members.filter((m) => m.status === "working").length
    const totalInput = members.reduce((s, m) => s + (m.inputTokens ?? 0), 0)
    const totalOutput = members.reduce((s, m) => s + (m.outputTokens ?? 0), 0)
    const completedTasks = tasks.filter((tk) => tk.columnName === "done").length
    const totalTasks = tasks.length

    const createdMs = new Date(team.createdAt).getTime()
    const elapsedMs = Date.now() - createdMs
    const elapsedMin = Math.floor(elapsedMs / 60000)
    const elapsedSec = Math.floor((elapsedMs % 60000) / 1000)
    const elapsed =
      elapsedMin > 0 ? `${elapsedMin}m ${elapsedSec}s` : `${elapsedSec}s`

    return {
      total: members.length,
      active: activeMembers,
      totalInput,
      totalOutput,
      completedTasks,
      totalTasks,
      elapsed,
    }
  }, [members, tasks, team.createdAt])

  const progressPct =
    stats.totalTasks > 0
      ? Math.round((stats.completedTasks / stats.totalTasks) * 100)
      : 0

  const fmtTokens = (n: number) =>
    n >= 1_000_000
      ? `${(n / 1_000_000).toFixed(1)}M`
      : n >= 1000
        ? `${(n / 1000).toFixed(1)}k`
        : String(n)

  return (
    <div className="flex flex-col gap-4">
      {/* Member grid */}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {members.map((m) => (
          <TeamMemberCard
            key={m.memberId}
            member={m}
            onViewSession={onViewSession}
          />
        ))}
      </div>

      {/* Progress bar */}
      {stats.totalTasks > 0 && (
        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>
              {t("team.progress", "Progress")}
            </span>
            <span className="tabular-nums">
              {stats.completedTasks}/{stats.totalTasks} ({progressPct}%)
            </span>
          </div>
          <div className="h-2 w-full overflow-hidden rounded-full bg-secondary">
            <div
              className={cn(
                "h-full rounded-full bg-primary transition-all duration-500",
                progressPct === 100 && "bg-green-500",
              )}
              style={{ width: `${progressPct}%` }}
            />
          </div>
        </div>
      )}

      {/* Stats row */}
      <div className="flex flex-wrap items-center gap-4 rounded-lg border border-border bg-muted/50 px-4 py-2.5 text-xs text-muted-foreground">
        <div className="flex items-center gap-1.5">
          <Users className="h-3.5 w-3.5" />
          <span>
            {stats.total} {t("team.members", "members")}
          </span>
          {stats.active > 0 && (
            <span className="text-blue-500">
              ({stats.active} {t("team.active", "active")})
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <Zap className="h-3.5 w-3.5" />
          <span className="tabular-nums">
            {fmtTokens(stats.totalInput)} / {fmtTokens(stats.totalOutput)}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <Clock className="h-3.5 w-3.5" />
          <span className="tabular-nums">{stats.elapsed}</span>
        </div>
      </div>
    </div>
  )
}
