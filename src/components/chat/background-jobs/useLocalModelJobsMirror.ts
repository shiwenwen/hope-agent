import { useEffect, useRef, useState } from "react"

import { getTransport } from "@/lib/transport-provider"
import {
  LOCAL_MODEL_JOB_EVENTS,
  type LocalModelJobSnapshot,
  isLocalModelJobVisible,
} from "@/types/local-model-jobs"

/**
 * R4: read-only mirror of the GLOBAL local-model jobs (downloads / installs /
 * reembeds) for the background-jobs panel. Subscribes to the same
 * `local_model_job:*` stream the settings / dashboard / knowledge surfaces use,
 * but never mutates — the panel folds these in so "什么在后台跑" lives in one
 * place, without taking over their dedicated controls. Returns only jobs worth
 * showing (active / paused / interrupted / failed).
 */
export function useLocalModelJobsMirror(): LocalModelJobSnapshot[] {
  const [jobs, setJobs] = useState<LocalModelJobSnapshot[]>([])
  const aliveRef = useRef(true)

  useEffect(() => {
    aliveRef.current = true
    const transport = getTransport()
    transport
      .call<LocalModelJobSnapshot[]>("local_model_job_list")
      .then((rows) => {
        if (aliveRef.current) setJobs(rows ?? [])
      })
      .catch(() => {
        /* best-effort mirror */
      })

    const upsert = (raw: unknown) => {
      const job = raw as LocalModelJobSnapshot
      if (!job?.jobId) return
      setJobs((prev) => {
        const idx = prev.findIndex((j) => j.jobId === job.jobId)
        if (idx === -1) return [job, ...prev]
        const next = prev.slice()
        next[idx] = job
        return next
      })
    }
    const offs = [
      transport.listen(LOCAL_MODEL_JOB_EVENTS.created, upsert),
      transport.listen(LOCAL_MODEL_JOB_EVENTS.updated, upsert),
      transport.listen(LOCAL_MODEL_JOB_EVENTS.completed, upsert),
    ]
    return () => {
      aliveRef.current = false
      for (const off of offs) off()
    }
  }, [])

  return jobs.filter(isLocalModelJobVisible)
}
