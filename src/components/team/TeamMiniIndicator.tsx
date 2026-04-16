import { useState, useEffect } from "react"
import { Users, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport"
import type { Team, TeamMember } from "./teamTypes"

interface TeamMiniIndicatorProps {
  teamId: string
  onClick: () => void
}

export default function TeamMiniIndicator({ teamId, onClick }: TeamMiniIndicatorProps) {
  const [teamName, setTeamName] = useState("")
  const [memberCount, setMemberCount] = useState(0)
  const [hasActive, setHasActive] = useState(false)

  useEffect(() => {
    Promise.all([
      getTransport().call<Team | null>("get_team", { teamId }),
      getTransport().call<TeamMember[]>("get_team_members", { teamId }),
    ])
      .then(([team, members]) => {
        if (team) setTeamName(team.name)
        setMemberCount(members.length)
        setHasActive(members.some((m) => m.status === "working"))
      })
      .catch(() => {})

    const unlisten = getTransport().listen("team_event", () => {
      getTransport()
        .call<TeamMember[]>("get_team_members", { teamId })
        .then((members) => {
          setMemberCount(members.length)
          setHasActive(members.some((m) => m.status === "working"))
        })
        .catch(() => {})
    })
    return unlisten
  }, [teamId])

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border border-border",
        "bg-muted/60 px-2.5 py-1 text-xs font-medium text-foreground",
        "transition-colors hover:bg-accent",
      )}
    >
      {hasActive ? (
        <Loader2 className="h-3 w-3 animate-spin text-blue-500" />
      ) : (
        <Users className="h-3 w-3 text-muted-foreground" />
      )}
      <span className="max-w-[100px] truncate">{teamName || "Team"}</span>
      <span className="tabular-nums text-muted-foreground">{memberCount}</span>
    </button>
  )
}
