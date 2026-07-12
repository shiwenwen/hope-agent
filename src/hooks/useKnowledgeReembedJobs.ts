import { useCallback, useEffect, useMemo, useState } from "react"

import { logger } from "@/lib/logger"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import {
  isLocalModelJobActive,
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"

const KNOWLEDGE_REEMBED_KIND = "knowledge_reembed"

function matchesKb(job: LocalModelJobSnapshot, kbId: string): boolean {
  return !job.targetKbIds || job.targetKbIds.length === 0 || job.targetKbIds.includes(kbId)
}

/**
 * Unified subscription for knowledge-space `reembed`/scan jobs — covers all
 * three triggers that share the `knowledge_reembed` job kind (see
 * `reembed.rs`): the settings-page "rebuild everything" button, the per-space
 * "Reindex" context-menu action, and binding a new external space. Consumed
 * by the empty-state component, the activity panel, the sidebar busy badge,
 * and the toolbar spinner — a single source of truth instead of each of them
 * separately subscribing to `local_model_job:*` events.
 */
export function useKnowledgeReembedJobs() {
  const [jobs, setJobs] = useState<LocalModelJobSnapshot[]>([])
  const [loadError, setLoadError] = useState<unknown>(null)

  const upsert = useCallback((job: LocalModelJobSnapshot) => {
    if (job.kind !== KNOWLEDGE_REEMBED_KIND) return
    setJobs((prev) => {
      const idx = prev.findIndex((j) => j.jobId === job.jobId)
      if (idx === -1) return [job, ...prev]
      const next = [...prev]
      next[idx] = job
      return next
    })
  }, [])

  // Seed via an inline async IIFE (not a hoisted `useCallback`) — calling a
  // separately-memoized async function from this effect trips
  // `react-hooks/set-state-in-effect`'s static analysis inside a custom hook,
  // even though the actual `setJobs` only ever lands after the `await`
  // (never synchronously within the effect body).
  useEffect(() => {
    ;(async () => {
      try {
        const list = await getTransport().call<LocalModelJobSnapshot[]>("local_model_job_list")
        setJobs((list ?? []).filter((j) => j.kind === KNOWLEDGE_REEMBED_KIND))
        setLoadError(null)
      } catch (e) {
        logger.warn("knowledge", "useKnowledgeReembedJobs::refresh", "Failed to load jobs", e)
        setLoadError(e)
      }
    })()
    const onSnap = (raw: unknown) => {
      const job = parsePayload<LocalModelJobSnapshot>(raw)
      if (!job) return
      upsert(job)
    }
    const un1 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, onSnap)
    const un2 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, onSnap)
    const un3 = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.completed, onSnap)
    return () => {
      un1()
      un2()
      un3()
    }
  }, [upsert])

  const sorted = useMemo(() => [...jobs].sort((a, b) => b.createdAt - a.createdAt), [jobs])
  const activeCount = useMemo(() => sorted.filter(isLocalModelJobActive).length, [sorted])

  /**
   * Most relevant job for `kbId` — prefers an active job over a terminal one,
   * so a KB with a failed bind-scan keeps surfacing "failed, retry" (via the
   * empty state) until the user retries or the job is cleared, instead of
   * silently reverting to an unexplained "empty" state.
   *
   * The terminal fallback only matches jobs scoped *to this KB specifically*
   * (`targetKbIds` includes it) — unlike the active branch, it does NOT fall
   * back to a full-scope job (`targetKbIds == null`). A failed full-app
   * rebuild isn't "this space's scan failed"; matching it here would hijack
   * an unrelated, genuinely-empty space's empty-state with a misleading
   * "Scan failed / Retry" UI whose retry button would actually re-trigger the
   * full rebuild, not anything about that specific space.
   */
  const jobForKb = useCallback(
    (kbId: string | null | undefined): LocalModelJobSnapshot | null => {
      if (!kbId) return null
      const active = sorted.find((j) => isLocalModelJobActive(j) && matchesKb(j, kbId))
      if (active) return active
      return sorted.find((j) => j.targetKbIds?.includes(kbId)) ?? null
    },
    [sorted],
  )

  const isKbBusy = useCallback(
    (kbId: string | null | undefined): boolean => {
      if (!kbId) return false
      return sorted.some((j) => isLocalModelJobActive(j) && matchesKb(j, kbId))
    },
    [sorted],
  )

  // Explicit local removal after a successful `local_model_job_clear` call —
  // that backend path deletes the DB row but emits no `local_model_job:*`
  // event (there is nothing left to broadcast a snapshot of), so without this
  // the cleared row would keep showing in the activity panel until the next
  // full reseed (navigation / remount).
  const dismiss = useCallback((jobId: string) => {
    setJobs((prev) => prev.filter((j) => j.jobId !== jobId))
  }, [])

  return { jobs: sorted, isKbBusy, jobForKb, activeCount, dismiss, loadError }
}
