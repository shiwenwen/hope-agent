import { createContext, useCallback, useEffect, useMemo, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import type { SubagentEvent, SubagentRun } from "@/types/chat"
import { TERMINAL_STATUSES } from "../subagentShared"
import { indexLatestRunByChildSession, type SubagentOpenTarget } from "./subagentRunModel"

const REFETCH_DEBOUNCE_MS = 200

export interface SubagentRunsSnapshot {
  sessionId: string | null
  /** started_at DESC, as returned by the backend. */
  runs: SubagentRun[]
  byId: ReadonlyMap<string, SubagentRun>
  byChildSessionId: ReadonlyMap<string, SubagentRun>
  /** Runs whose status is not yet terminal. */
  runningCount: number
  /** The initial `list_subagent_runs` for this session has returned — lets
   *  callers distinguish "not fetched yet" from "genuinely no runs". */
  loaded: boolean
  refetch: () => void
}

/**
 * Session-scoped live snapshot of sub-agent runs. Mirrors `useBackgroundJobs`:
 * seed from `list_subagent_runs`, then debounced full refetch on any
 * `subagent_event` for THIS session (runs emit ~2-3 events total, never
 * per-token, so a refetch is simpler and always-correct vs. incremental merge).
 * State is tagged with its session so a switch reads as empty immediately
 * (derived) and a late response for the previous session is ignored.
 */
export function useSubagentRuns(sessionId: string | null | undefined): SubagentRunsSnapshot {
  const sid = sessionId ?? null
  const [state, setState] = useState<{
    sid: string | null
    runs: SubagentRun[]
    loaded: boolean
  }>({ sid: null, runs: [], loaded: false })
  const [refetchTick, setRefetchTick] = useState(0)

  const refetch = useCallback(() => setRefetchTick((t) => t + 1), [])

  useEffect(() => {
    if (!sid) return

    let alive = true
    let debounce: ReturnType<typeof setTimeout> | null = null

    const fetchNow = () => {
      getTransport()
        .call<SubagentRun[]>("list_subagent_runs", { sessionId: sid })
        .then((rows) => {
          if (alive) setState({ sid, runs: rows ?? [], loaded: true })
        })
        .catch(() => {
          /* transient read failure — keep the last good snapshot */
        })
    }
    const scheduleRefetch = () => {
      if (debounce) clearTimeout(debounce)
      debounce = setTimeout(fetchNow, REFETCH_DEBOUNCE_MS)
    }

    fetchNow()

    const off = getTransport().listen("subagent_event", (raw) => {
      if ((raw as { parentSessionId?: string })?.parentSessionId === sid) scheduleRefetch()
    })

    return () => {
      alive = false
      if (debounce) clearTimeout(debounce)
      off()
    }
  }, [sid, refetchTick])

  // A stale snapshot from the previous session reads as empty / not-loaded
  // until the new fetch lands.
  const runs = useMemo(() => (state.sid === sid ? state.runs : []), [state, sid])
  const loaded = state.sid === sid && state.loaded
  const byId = useMemo(() => new Map(runs.map((r) => [r.runId, r] as const)), [runs])
  const byChildSessionId = useMemo(() => indexLatestRunByChildSession(runs), [runs])
  const runningCount = useMemo(
    () => [...byChildSessionId.values()].filter((r) => !TERMINAL_STATUSES.has(r.status)).length,
    [byChildSessionId],
  )

  return { sessionId: sid, runs, byId, byChildSessionId, runningCount, loaded, refetch }
}

export type SubagentRunsView = SubagentRunsSnapshot & {
  /** Absent when the host wired no handler — chips then render inert rather
   *  than as buttons that silently do nothing. */
  openRun?: (target: SubagentOpenTarget) => void
}

export const SubagentRunsContext = createContext<SubagentRunsView | null>(null)

/** Detail for a single run in the panel. When `primary` is supplied (the run is
 *  in the always-live parent-session snapshot) it is authoritative. Otherwise
 *  (a nested-level run not in the parent snapshot) fetch once and re-fetch on
 *  each matching `subagent_event`. Bump `reloadToken` to force a manual re-fetch
 *  (the panel's Refresh button). */
export function useSubagentRunDetail(
  runId: string | null,
  primary?: SubagentRun,
  reloadToken?: number,
): SubagentRun | null {
  const hasPrimary = !!primary
  const [fetched, setFetched] = useState<{ runId: string; run: SubagentRun | null } | null>(null)

  useEffect(() => {
    if (!runId || hasPrimary) return
    let alive = true
    const fetchNow = () => {
      getTransport()
        .call<SubagentRun | null>("get_subagent_run", { runId })
        .then((run) => {
          if (alive) setFetched({ runId, run: run ?? null })
        })
        .catch(() => {})
    }
    fetchNow()
    const off = getTransport().listen("subagent_event", (raw) => {
      if ((raw as SubagentEvent).runId === runId) fetchNow()
    })
    return () => {
      alive = false
      off()
    }
  }, [runId, hasPrimary, reloadToken])

  if (primary) return primary
  // Ignore a stale fetch left over from a previous runId.
  return fetched && fetched.runId === runId ? fetched.run : null
}
