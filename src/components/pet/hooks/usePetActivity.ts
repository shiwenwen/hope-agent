import { useCallback, useEffect, useRef, useState } from "react"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import type { PetActivitySnapshot } from "@/types/pet"

const EMPTY_SNAPSHOT: PetActivitySnapshot = {
  revision: 0,
  generatedAt: new Date(0).toISOString(),
  stale: false,
  dominant: null,
  activities: [],
  total: 0,
  truncated: false,
}

export function usePetActivity(): {
  snapshot: PetActivitySnapshot
  refresh: () => Promise<void>
  initialized: boolean
} {
  const [snapshot, setSnapshot] = useState(EMPTY_SNAPSHOT)
  const [initialized, setInitialized] = useState(false)
  const requestRevision = useRef(0)

  const refresh = useCallback(async () => {
    const revision = ++requestRevision.current
    try {
      const next = await getTransport().call<PetActivitySnapshot>("pet_activity_snapshot_cmd")
      if (revision !== requestRevision.current) return
      setSnapshot(next)
      setInitialized(true)
    } catch {
      if (revision !== requestRevision.current) return
      setSnapshot((current) => ({ ...current, stale: true }))
      setInitialized(true)
    }
  }, [])

  useEffect(() => {
    const initialRefresh = setTimeout(() => void refresh(), 0)
    let timer: ReturnType<typeof setTimeout> | null = null
    const schedule = () => {
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => void refresh(), 120)
    }
    const transport = getTransport()
    const unlisteners = [
      transport.listen("pet:activity_changed", schedule),
      transport.listen("session:title_updated", schedule),
      transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, schedule),
    ]
    const reconcile = setInterval(() => {
      if (document.visibilityState === "visible") void refresh()
    }, 5_000)
    return () => {
      clearTimeout(initialRefresh)
      if (timer) clearTimeout(timer)
      clearInterval(reconcile)
      requestRevision.current += 1
      for (const unlisten of unlisteners) unlisten()
    }
  }, [refresh])

  return { snapshot, refresh, initialized }
}
