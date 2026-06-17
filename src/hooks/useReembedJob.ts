import { useCallback, useEffect, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import {
  isLocalModelJobTerminal,
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobKind,
  type LocalModelJobSnapshot,
} from "@/types/local-model-jobs"

interface UseReembedJobOptions {
  /** Which reembed job kind to track ("memory_reembed" / "knowledge_reembed"). */
  kind: LocalModelJobKind
  /** Invoked once when a tracked job transitions to "completed". */
  onCompleted?: () => void
}

/**
 * Track the single global reembed job of a given kind via the LocalModelJobs
 * event stream. Survives navigation / refresh / app restart (interrupted jobs
 * replay through the same channel). Shared by the memory and knowledge embedding
 * settings panels — the only difference between them is the `kind` filter and
 * the `onCompleted` side effect.
 *
 * The snapshot reducer dedups: a new spawn replaces a terminal/cancelling
 * predecessor; an unchanged snapshot for the tracked job is dropped so the
 * consuming panel doesn't re-render on every repeated/per-batch progress frame.
 */
export function useReembedJob({ kind, onCompleted }: UseReembedJobOptions) {
  const [job, setJob] = useState<LocalModelJobSnapshot | null>(null)

  useEffect(() => {
    let cancelled = false

    void getTransport()
      .call<LocalModelJobSnapshot[]>("local_model_job_list")
      .then((jobs) => {
        if (cancelled) return
        // Snapshots come back in descending createdAt order; first match is freshest.
        setJob(jobs.find((j) => j.kind === kind) ?? null)
      })
      .catch((e) => logger.warn("settings", "useReembedJob::load", "Failed to load jobs", e))

    const handleSnapshot = (raw: unknown): LocalModelJobSnapshot | null => {
      const snap = parsePayload<LocalModelJobSnapshot>(raw)
      if (!snap) return null
      if (snap.kind !== kind) return null
      setJob((current) => {
        const next = (() => {
          if (!current) return snap
          if (current.jobId === snap.jobId) return snap
          if (isLocalModelJobTerminal(current)) return snap
          if (current.status === "cancelling" && snap.createdAt >= current.createdAt) return snap
          return current
        })()
        if (next === current) return current
        // Skip the re-render when nothing observable changed — the backend emits
        // per-batch progress and repeats status snapshots on completion.
        if (
          current &&
          next.jobId === current.jobId &&
          next.status === current.status &&
          next.phase === current.phase &&
          (next.percent ?? null) === (current.percent ?? null) &&
          (next.bytesCompleted ?? null) === (current.bytesCompleted ?? null) &&
          (next.bytesTotal ?? null) === (current.bytesTotal ?? null) &&
          (next.error ?? null) === (current.error ?? null)
        ) {
          return current
        }
        return next
      })
      return snap
    }

    const handleCompleted = (raw: unknown) => {
      const snap = handleSnapshot(raw)
      if (snap?.status === "completed" && !cancelled) onCompleted?.()
    }

    const unlistenCreated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.created, handleSnapshot)
    const unlistenUpdated = getTransport().listen(LOCAL_MODEL_JOB_EVENTS.updated, handleSnapshot)
    const unlistenCompleted = getTransport().listen(
      LOCAL_MODEL_JOB_EVENTS.completed,
      handleCompleted,
    )
    return () => {
      cancelled = true
      unlistenCreated()
      unlistenUpdated()
      unlistenCompleted()
    }
  }, [kind, onCompleted])

  const dismiss = useCallback(() => {
    setJob((current) => {
      if (!current || !isLocalModelJobTerminal(current)) return current
      void getTransport()
        .call("local_model_job_clear", { jobId: current.jobId })
        .catch((e) => logger.warn("settings", "useReembedJob::dismiss", "Failed to clear job", e))
      return null
    })
  }, [])

  return { job, dismiss }
}
