import { useEffect, useRef, useState, useEffectEvent } from "react"
import { getTransport } from "@/lib/transport-provider"
import type { Team, TeamMember, TeamMessage, TeamTask, TeamEvent } from "./teamTypes"

const TEAM_MESSAGE_PAGE_SIZE = 50

/**
 * Hook to manage team state with real-time EventBus subscription.
 */
export function useTeam(teamId: string | null) {
  const [team, setTeam] = useState<Team | null>(null)
  const [members, setMembers] = useState<TeamMember[]>([])
  const [messages, setMessages] = useState<TeamMessage[]>([])
  const [tasks, setTasks] = useState<TeamTask[]>([])
  const [loading, setLoading] = useState(false)
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const teamIdRef = useRef(teamId)
  teamIdRef.current = teamId

  // ── Fetch data ────────────────────────────────────────────

  const reload = async () => {
    if (!teamId) return
    setLoading(true)
    try {
      const [t, m, msgPage, tks] = await Promise.all([
        getTransport().call<Team | null>("get_team", { teamId }),
        getTransport().call<TeamMember[]>("get_team_members", { teamId }),
        getTransport().call<[TeamMessage[], boolean]>("get_team_messages", {
          teamId,
          limit: TEAM_MESSAGE_PAGE_SIZE,
        }),
        getTransport().call<TeamTask[]>("get_team_tasks", { teamId }),
      ])
      if (teamIdRef.current === teamId) {
        setTeam(t)
        setMembers(m)
        setMessages(msgPage[0])
        setHasMore(msgPage[1])
        setLoadingMore(false)
        setTasks(tks)
      }
    } catch {
      // Ignore errors during reload
    } finally {
      setLoading(false)
    }
  }
  const reloadEffectEvent = useEffectEvent(reload)

  // ── Pagination: load older messages ───────────────────────

  const loadMoreMessages = async () => {
    const tid = teamIdRef.current
    if (!tid || !hasMore || loadingMore) return
    const oldest = messages[0]
    if (!oldest) return
    setLoadingMore(true)
    try {
      const [older, moreBefore] = await getTransport().call<[TeamMessage[], boolean]>(
        "get_team_messages_before",
        {
          teamId: tid,
          beforeTimestamp: oldest.timestamp,
          beforeMessageId: oldest.messageId,
          limit: TEAM_MESSAGE_PAGE_SIZE,
        },
      )
      if (teamIdRef.current !== tid) return
      if (older.length === 0) {
        setHasMore(false)
        return
      }
      setMessages((prev) => [...older, ...prev])
      setHasMore(moreBefore)
    } catch {
      // Ignore; user can retry by scrolling up again
    } finally {
      setLoadingMore(false)
    }
  }

  // ── Initial load ──────────────────────────────────────────

  useEffect(() => {
    if (teamId) {
      reloadEffectEvent()
    } else {
      setTeam(null)
      setMembers([])
      setMessages([])
      setTasks([])
      setHasMore(false)
      setLoadingMore(false)
    }
  }, [teamId])

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
            setMessages((prev) =>
              prev.some((m) => m.messageId === msg.messageId) ? prev : [...prev, msg],
            )
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
          reloadEffectEvent()
          break
      }
    })

    return unlisten
  }, [teamId])

  // ── Actions ───────────────────────────────────────────────

  const sendMessage = async (to: string | null, content: string) => {
    if (!teamId) return
    await getTransport().call("send_user_team_message", {
      teamId,
      to,
      content,
    })
  }

  return {
    team,
    members,
    messages,
    tasks,
    loading,
    hasMore,
    loadingMore,
    loadMoreMessages,
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
      // eslint-disable-next-line react-hooks/set-state-in-effect
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
  useEffect(() => {
    sessionIdRef.current = sessionId
  }, [sessionId])

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
