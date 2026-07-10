import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { BrowserActivityDto, FileArtifactSummary, SessionArtifacts } from "@/lib/transport"
import type { Message } from "@/types/chat"
import { useSessionFileChanges, type SessionFileEntry } from "./useSessionFileChanges"
import { useSessionUrlSources, type SessionUrlSource } from "./useSessionUrlSources"
import { useSessionBrowserActivity, type SessionBrowserActivity } from "./useSessionBrowserActivity"

export interface WorkspaceArtifacts {
  files: SessionFileEntry[]
  sources: SessionUrlSource[]
  browser: SessionBrowserActivity[]
  filesTruncated: boolean
  sourcesTruncated: boolean
  browserTruncated: boolean
}

/** Backend file summary → `SessionFileEntry` (no historical diff — window-外
 *  files preview current content instead). */
function backendFileToEntry(f: FileArtifactSummary): SessionFileEntry {
  return {
    path: f.path,
    kind: f.kind,
    diff: null,
    readLines: f.readLines,
    linesAdded: f.linesAdded,
    linesRemoved: f.linesRemoved,
  }
}

function backendBrowserToEntry(activity: BrowserActivityDto): SessionBrowserActivity {
  return {
    action: activity.action,
    op: activity.op,
    targetId: activity.targetId,
    url: activity.url,
    title: activity.title,
    backend: activity.backend,
    sessionId: activity.sessionId,
    callId: activity.callId,
    at: activity.at,
  }
}

/**
 * Merge the in-memory live tail with the complete backend list. Both are
 * most-recent-first; the live tail is the loaded window (always the most
 * recent), so it goes first — this keeps a file/url re-touched in the current
 * turn at the top even when an older copy exists in the backend snapshot.
 * Backend-only entries (older than the loaded window) follow. Overlaps take the
 * live entry (fresher diff / counts), unless `reconcile` adjusts the merged
 * value from both sides.
 */
export function mergeArtifacts<T>(
  backend: T[],
  live: T[],
  keyOf: (t: T) => string,
  reconcile?: (live: T, backend: T) => T,
): T[] {
  if (backend.length === 0) return live
  const backendByKey = new Map(backend.map((i) => [keyOf(i), i]))
  const liveKeys = new Set(live.map(keyOf))
  const mergedLive = reconcile
    ? live.map((l) => {
        const b = backendByKey.get(keyOf(l))
        return b ? reconcile(l, b) : l
      })
    : live
  const backendOnly = backend.filter((b) => !liveKeys.has(keyOf(b)))
  return backendOnly.length ? [...mergedLive, ...backendOnly] : mergedLive
}

/** Preserve a `web_search` badge: if either side saw the URL via search, the
 *  merged source keeps that origin (the live tail may have only seen a later
 *  plain-prose mention of a URL the backend first found via search). */
function reconcileSource(live: SessionUrlSource, backend: SessionUrlSource): SessionUrlSource {
  if (live.origin !== "web_search" && backend.origin === "web_search") {
    return { ...live, origin: "web_search" }
  }
  return live
}

/**
 * Hybrid data source for the workspace panel's output (files) + sources (URLs).
 *
 * - **Backend** ([`loadSessionArtifacts`]) aggregates the session's FULL history
 *   (the in-memory `messages` is only a paginated window), fetched on session
 *   change / panel mount and re-fetched when a turn completes (`turnActive`
 *   true→false, which persists that turn's new artifacts).
 * - **Live tail** (`useSessionFileChanges` / `useSessionUrlSources` over the
 *   in-memory `messages`) keeps the current streaming turn visible immediately,
 *   and carries the structured diff for window-内 files.
 *
 * Incognito sessions skip the backend entirely (short / fully in the loaded
 * window, and we honor "close-and-burn" by not reading their persisted rows
 * here) — live tail only.
 */
export function useWorkspaceArtifacts(
  sessionId: string | null | undefined,
  messages: Message[],
  opts: { incognito?: boolean; turnActive?: boolean } = {},
): WorkspaceArtifacts {
  const { incognito = false, turnActive = false } = opts

  const liveFiles = useSessionFileChanges(messages) // most-recent-first
  const liveSourcesChrono = useSessionUrlSources(messages) // chronological
  const liveSources = useMemo(
    () => [...liveSourcesChrono].reverse(), // unify to most-recent-first
    [liveSourcesChrono],
  )
  const liveBrowserChrono = useSessionBrowserActivity(messages)
  const liveBrowser = useMemo(
    () => [...liveBrowserChrono].reverse(), // unify to most-recent-first
    [liveBrowserChrono],
  )

  // Fetched data is tagged with its session id; the derivation below ignores a
  // snapshot whose id no longer matches (stale fetch after a session switch).
  const [backend, setBackend] = useState<{
    sid: string
    files: SessionFileEntry[]
    sources: SessionUrlSource[]
    browser: SessionBrowserActivity[]
    filesTruncated: boolean
    sourcesTruncated: boolean
    browserTruncated: boolean
  } | null>(null)

  // Monotonic request id: only the newest fetch's response is applied, so two
  // in-flight fetches for the same session can't land out of order.
  const reqRef = useRef(0)
  const fetchInto = useCallback((sid: string) => {
    const req = ++reqRef.current
    getTransport()
      .loadSessionArtifacts(sid)
      .then((res: SessionArtifacts) => {
        if (reqRef.current !== req) return
        setBackend({
          sid,
          files: res.files.map(backendFileToEntry),
          sources: res.sources,
          browser: (res.browser ?? []).map(backendBrowserToEntry),
          filesTruncated: res.filesTruncated,
          sourcesTruncated: res.sourcesTruncated,
          browserTruncated: res.browserTruncated ?? false,
        })
      })
      .catch((e) => {
        if (reqRef.current !== req) return
        // Leave any prior data in place; the live tail still renders.
        logger.error("ui", "useWorkspaceArtifacts", "Failed to load session artifacts", e)
      })
  }, [])

  // Initial load + on session change (fires even mid-turn so older history
  // shows immediately; the live tail overlays the in-flight turn).
  useEffect(() => {
    if (!sessionId || incognito) return
    fetchInto(sessionId)
  }, [sessionId, incognito, fetchInto])

  // Re-fetch on turn completion (true→false) — that turn's artifacts are now
  // persisted. Edge-detected via a ref so a session change alone doesn't fetch.
  const prevTurnActive = useRef(turnActive)
  useEffect(() => {
    const was = prevTurnActive.current
    prevTurnActive.current = turnActive
    if (was && !turnActive && sessionId && !incognito) {
      fetchInto(sessionId)
    }
  }, [turnActive, sessionId, incognito, fetchInto])

  // Ignore a backend snapshot from a different session, and skip it entirely
  // for incognito (live tail only).
  const data = !incognito && backend && backend.sid === sessionId ? backend : null

  const files = useMemo(
    () => mergeArtifacts(data?.files ?? [], liveFiles, (e) => e.path),
    [data, liveFiles],
  )
  const sources = useMemo(
    () => mergeArtifacts(data?.sources ?? [], liveSources, (s) => s.url, reconcileSource),
    [data, liveSources],
  )
  const browser = useMemo(
    () =>
      mergeArtifacts(
        data?.browser ?? [],
        liveBrowser,
        (activity) =>
          activity.callId ??
          [
            activity.at ?? "",
            activity.action,
            activity.op ?? "",
            activity.targetId ?? "",
            activity.url ?? "",
          ].join(":"),
      ),
    [data, liveBrowser],
  )

  return {
    files,
    sources,
    browser,
    filesTruncated: data?.filesTruncated ?? false,
    sourcesTruncated: data?.sourcesTruncated ?? false,
    browserTruncated: data?.browserTruncated ?? false,
  }
}
