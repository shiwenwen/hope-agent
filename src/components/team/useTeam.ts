import { useState, useEffect, useCallback, useRef } from "react"
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
  const [stateTeamId, setStateTeamId] = useState(teamId)
  if (stateTeamId !== teamId) {
    setStateTeamId(teamId)
    setTeam(null)
    setMembers([])
    setMessages([])
    setTasks([])
    setHasMore(false)
    setLoadingMore(false)
    setLoading(Boolean(teamId))
  }
  const teamIdRef = useRef(teamId)
  teamIdRef.current = teamId

  // ── Fetch data ────────────────────────────────────────────

  const reload = useCallback(async () => {
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
      if (teamIdRef.current === teamId) {
        setLoading(false)
      }
    }
  }, [teamId])

  // ── Pagination: load older messages ───────────────────────

  const loadMoreMessages = useCallback(async () => {
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
  }, [hasMore, loadingMore, messages])

  // ── Initial load ──────────────────────────────────────────

  useEffect(() => {
    if (teamId) {
      reload()
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
    [teamId],
  )

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

function preferredControllableTeamId(teams: readonly Team[]): string | null {
  // An active team is the primary workspace. If none exists, retain the most
  // recent paused team so a restart does not hide the only UI resume control.
  return (
    teams.find((team) => team.status === "active")?.teamId ??
    teams.find((team) => team.status === "paused")?.teamId ??
    null
  )
}

/**
 * Hook to discover the current active or resumable team for a session.
 *
 * The historical name is retained for call-site compatibility; a paused team
 * is intentionally visible because it is still a live control-plane object.
 */
export function useActiveTeam(sessionId: string | null) {
  const [activeTeamState, setActiveTeamState] = useState<{
    sessionId: string
    teamId: string | null
  } | null>(null)
  const activeTeamId = activeTeamState?.sessionId === sessionId ? activeTeamState.teamId : null
  const sessionIdRef = useRef(sessionId)

  useEffect(() => {
    sessionIdRef.current = sessionId
  }, [sessionId])

  const refresh = useCallback(async (requestedSessionId: string) => {
    try {
      const teams = await getTransport().call<Team[]>("list_teams", {
        sessionId: requestedSessionId,
      })
      if (sessionIdRef.current === requestedSessionId) {
        setActiveTeamState({
          sessionId: requestedSessionId,
          teamId: preferredControllableTeamId(teams),
        })
      }
    } catch {
      if (sessionIdRef.current === requestedSessionId) {
        setActiveTeamState({ sessionId: requestedSessionId, teamId: null })
      }
    }
  }, [])

  useEffect(() => {
    if (!sessionId) return
    let cancelled = false
    void getTransport()
      .call<Team[]>("list_teams", { sessionId })
      .then((teams) => {
        if (!cancelled) {
          setActiveTeamState({ sessionId, teamId: preferredControllableTeamId(teams) })
        }
      })
      .catch(() => {
        if (!cancelled) setActiveTeamState({ sessionId, teamId: null })
      })
    return () => {
      cancelled = true
    }
  }, [sessionId])

  useEffect(() => {
    const unlisten = getTransport().listen("team_event", (raw) => {
      const event = raw as TeamEvent
      if (event.type === "created") {
        const team = event.payload as Team
        const currentSessionId = sessionIdRef.current
        if (currentSessionId && team.leadSessionId === currentSessionId) {
          setActiveTeamState({ sessionId: currentSessionId, teamId: team.teamId })
        }
      } else if (event.type === "dissolved") {
        const payload = event.payload as { teamId: string }
        const currentSessionId = sessionIdRef.current
        if (!currentSessionId) return
        // Clear immediately, then discover another active/paused team if one
        // exists for this session.
        setActiveTeamState((prev) =>
          prev?.sessionId === currentSessionId && prev.teamId === payload.teamId
            ? { sessionId: currentSessionId, teamId: null }
            : prev,
        )
        void refresh(currentSessionId)
      } else if (event.type === "paused" || event.type === "resumed") {
        const currentSessionId = sessionIdRef.current
        if (currentSessionId) void refresh(currentSessionId)
      }
    })
    return unlisten
  }, [refresh])

  return activeTeamId
}
