import { useMemo } from "react"
import { Eye } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import type { TeamMember } from "./teamTypes"
import { MEMBER_STATUS_CONFIG } from "./teamTypes"

interface TeamMemberCardProps {
  member: TeamMember
  onViewSession?: (sessionId: string) => void
}

export function TeamMemberCard({ member, onViewSession }: TeamMemberCardProps) {
  const { t } = useTranslation()

  const statusCfg = MEMBER_STATUS_CONFIG[member.status]
  const inputTokens = member.inputTokens ?? 0
  const outputTokens = member.outputTokens ?? 0

  const formattedTokens = useMemo(() => {
    const fmt = (n: number) =>
      n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n)
    return { input: fmt(inputTokens), output: fmt(outputTokens) }
  }, [inputTokens, outputTokens])

  const roleLabel = member.role === "lead"
    ? t("team.role.lead", "Lead")
    : member.role === "reviewer"
      ? t("team.role.reviewer", "Reviewer")
      : t("team.role.worker", "Worker")

  return (
    <div
      className="relative flex items-start gap-3 rounded-lg border border-border bg-background p-3 transition-colors hover:bg-accent/50"
      style={{ borderLeftColor: member.color, borderLeftWidth: 3 }}
    >
      {/* Name + Role */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{member.name}</span>
          <span className="shrink-0 rounded-full bg-secondary px-2 py-0.5 text-[10px] font-medium text-secondary-foreground">
            {roleLabel}
          </span>
        </div>

        {/* Status */}
        <div className="mt-1 flex items-center gap-1.5">
          {member.status === "working" && (
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-blue-400 opacity-75" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-blue-500" />
            </span>
          )}
          <span
            className={cn(
              "rounded-full px-1.5 py-0.5 text-[10px] font-medium",
              statusCfg.bgColor,
              statusCfg.color,
            )}
          >
            {statusCfg.label}
          </span>
        </div>

        {/* Tokens */}
        <div className="mt-1.5 flex items-center gap-2 text-[11px] tabular-nums text-muted-foreground">
          <span>{formattedTokens.input}</span>
          <span className="text-muted-foreground/50">/</span>
          <span>{formattedTokens.output}</span>
        </div>
      </div>

      {/* View button */}
      {member.sessionId && onViewSession && (
        <Button
          variant="ghost"
          size="sm"
          className="shrink-0 h-7 px-2 text-xs"
          onClick={() => onViewSession(member.sessionId!)}
        >
          <Eye className="mr-1 h-3 w-3" />
          {t("team.view", "View")}
        </Button>
      )}
    </div>
  )
}
