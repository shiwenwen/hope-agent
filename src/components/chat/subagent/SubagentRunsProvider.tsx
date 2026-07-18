import { type ReactNode } from "react"

import {
  SubagentRunsContext,
  useSubagentRuns,
  type SubagentRunsSnapshot,
  type SubagentRunsView,
} from "./useSubagentRuns"
import type { SubagentOpenTarget } from "./subagentRunModel"

/** Wraps a subtree (a `MessageList`) so descendant sub-agent chips share one
 *  live snapshot + one `openRun` action, without threading props through the
 *  memoized MessageBubble → MessageContent chain.
 *
 *  Pass `snapshot` to reuse a snapshot the host already subscribes to (the main
 *  chat screen keeps one for its panel + title badge) — this avoids a second
 *  `list_subagent_runs` fetch and event listener for the same session, and keeps
 *  the chips and the panel reading the SAME data so a chip-click always resolves.
 *  Omit it and the provider self-subscribes for `sessionId`. */
export function SubagentRunsProvider({
  sessionId,
  snapshot,
  onOpenRun,
  children,
}: {
  sessionId: string | null
  snapshot?: SubagentRunsSnapshot
  onOpenRun?: (target: SubagentOpenTarget) => void
  children: ReactNode
}) {
  // When a host snapshot is supplied, subscribe to nothing (null session) so we
  // don't duplicate its fetch/listener; otherwise self-subscribe.
  const own = useSubagentRuns(snapshot ? null : sessionId)
  // Pass the handler through as-is: wrapping it in an always-defined callback
  // would make chips look actionable on hosts that wired nothing (e.g. the cron
  // session viewer), where a click would silently do nothing.
  const value: SubagentRunsView = { ...(snapshot ?? own), openRun: onOpenRun }
  return <SubagentRunsContext.Provider value={value}>{children}</SubagentRunsContext.Provider>
}
