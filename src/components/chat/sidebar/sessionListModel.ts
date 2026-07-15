import type { SessionMeta, SessionSearchResult, SessionSearchType } from "@/types/chat"
import type { SessionFilterType } from "./types"

/**
 * Search is intentionally broader than the browsing tabs. Project sessions
 * have their own tree and subagents have their own tab, but both must remain
 * discoverable from the single sidebar search box.
 */
export const GLOBAL_SESSION_SEARCH_TYPES: SessionSearchType[] = ["regular", "subagent", "channel"]

export function sidebarSessionPageArgs(
  filter: SessionFilterType,
  selectedAgentId: string | null,
  offset: number,
  limit: number,
  activeSessionId?: string | null,
): Record<string, unknown> {
  return {
    unassigned: true,
    parentSession: filter === "subagent",
    agentId: selectedAgentId ?? undefined,
    limit,
    offset,
    activeSessionId: activeSessionId ?? undefined,
  }
}

/**
 * The flat list below the project tree owns only unassigned sessions. Project
 * sessions render exclusively inside their project group, avoiding duplicate
 * rows and keeping deep project navigation from reshaping the flat list.
 */
export function filterSessionsForSidebarTab(
  sessions: SessionMeta[],
  filter: SessionFilterType,
  selectedAgentId: string | null = null,
): SessionMeta[] {
  return sessions.filter((session) => {
    if (selectedAgentId !== null && session.agentId !== selectedAgentId) return false
    if (session.isCron || session.projectId) return false

    return filter === "subagent" ? !!session.parentSessionId : !session.parentSessionId
  })
}

/**
 * The backend already excludes cron results through GLOBAL_SESSION_SEARCH_TYPES.
 * Keep this defensive guard so mixed/legacy transports cannot leak cron runs
 * into the main sidebar search surface.
 */
export function filterGlobalSessionSearchResults(
  results: SessionSearchResult[] | null,
): SessionSearchResult[] {
  return results?.filter((result) => !result.isCron) ?? []
}
