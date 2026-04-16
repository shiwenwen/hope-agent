import { useState, useEffect, useCallback, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import type {
  Team,
  TeamMember,
  TeamMessage,
  TeamTask,
  TeamEvent,
} from "./teamTypes"

/**
 * Hook to manage team state with real-time EventBus subscription.
 */
export function useTeam(teamId: string | null) {
  const [team, setTeam] = useState<Team | null>(null)
  const [members, setMembers] = useState<TeamMember[]>([])
  const [messages, setMessages] = useState<TeamMessage[]>([])
  const [tasks, setTasks] = useState<TeamTask[]>([])
  const [loading, setLoading] = useState(false)
  const teamIdRef = useRef(teamId)
  teamIdRef.current = teamId

  // ── Fetch data ────────────────────────────────────────────

  const reload = useCallback(async () => {
    if (!teamId) return
    setLoading(true)
    try {
      const [t, m, msgs, tks] = await Promise.all([
        getTransport().call<Team | null>("get_team", { teamId }),
        getTransport().call<TeamMember[]>("get_team_members", { teamId }),
        getTransport().call<TeamMessage[]>("get_team_messages", { teamId, limit: 100 }),
        getTransport().call<TeamTask[]>("get_team_tasks", { teamId }),
      ])
      if (teamIdRef.current === teamId) {
        setTeam(t)
        setMembers(m)
        setMessages(msgs)
        setTasks(tks)
      }
    } catch {
      // Ignore errors during reload
    } finally {
      setLoading(false)
    }
  }, [teamId])

  // ── Initial load ──────────────────────────────────────────

  useEffect(() => {
    if (teamId) {
      reload()
    } else {
      setTeam(null)
      setMembers([])
      setMessages([])
      setTasks([])
    }
  }, [teamId, reload])

  // ── Real-time event subscription (debounced member reload) ─

  const memberReloadTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    const debouncedMemberReload = () => {
      if (memberReloadTimer.current) clearTimeout(memberReloadTimer.current)
      memberReloadTimer.current = setTimeout(() => {
        if (!teamIdRef.current) return
        getTransport()
          .call<TeamMember[]>("get_team_members", { teamId: teamIdRef.current })
          .then(setMembers)
          .catch(() => {})
      }, 300)
    }

    const unlisten = getTransport().listen("team_event", (raw) => {
      const event = raw as TeamEvent
      if (!teamIdRef.current) return

      switch (event.type) {
        case "member_joined":
        case "member_status":
        case "member_completed":
          debouncedMemberReload()
          break

        case "message": {
          const msg = event.payload as TeamMessage
          if (msg.teamId === teamIdRef.current) {
            setMessages((prev) => {
              const next = [...prev, msg]
              return next.length > 200 ? next.slice(-200) : next
            })
          }
          break
        }

        case "task_updated": {
          const task = event.payload as TeamTask
          if (task.teamId === teamIdRef.current) {
            setTasks((prev) => {
              const idx = prev.findIndex((t) => t.id === task.id)
              if (idx >= 0) {
                const next = [...prev]
                next[idx] = task
                return next
              }
              return [...prev, task]
            })
          }
          break
        }

        case "paused":
        case "resumed":
        case "dissolved":
          reload()
          break
      }
    })

    return unlisten
  }, [reload])

  // ── Actions ───────────────────────────────────────────────

  const sendMessage = useCallback(
    async (to: string | null, content: string) => {
      if (!teamId) return
      await getTransport().call("send_user_team_message", {
        teamId,
        to,
        content,
      })
    },
    [teamId]
  )

  return {
    team,
    members,
    messages,
    tasks,
    loading,
    reload,
    sendMessage,
  }
}

/**
 * Hook to discover any active team for the current session.
 */
export function useActiveTeam(sessionId: string | null) {
  const [activeTeamId, setActiveTeamId] = useState<string | null>(null)

  useEffect(() => {
    if (!sessionId) {
      setActiveTeamId(null)
      return
    }

    getTransport()
      .call<Team[]>("list_teams", { sessionId })
      .then((teams) => {
        const active = teams.find((t) => t.status === "active")
        setActiveTeamId(active?.teamId ?? null)
      })
      .catch(() => setActiveTeamId(null))
  }, [sessionId])

  // Listen for team create/dissolve events — scoped to current session
  const sessionIdRef = useRef(sessionId)
  sessionIdRef.current = sessionId

  useEffect(() => {
    const unlisten = getTransport().listen("team_event", (raw) => {
      const event = raw as TeamEvent
      if (event.type === "created") {
        const team = event.payload as Team
        if (team.leadSessionId === sessionIdRef.current) {
          setActiveTeamId(team.teamId)
        }
      } else if (event.type === "dissolved") {
        const payload = event.payload as { teamId: string }
        // Only clear if the dissolved team is our active team
        setActiveTeamId((prev) => (prev === payload.teamId ? null : prev))
      }
    })
    return unlisten
  }, [])

  return activeTeamId
}
