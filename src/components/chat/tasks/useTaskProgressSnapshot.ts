import { useEffect, useMemo, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload, TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { logger } from "@/lib/logger"
import type { Message } from "@/types/chat"
import {
  createCurrentTaskProgressSnapshot,
  extractLatestTaskProgressSnapshot,
  taskProgressSnapshotFromTasks,
  type TaskProgressSnapshot,
} from "./taskProgress"

interface TaskUpdatedPayload {
  sessionId?: string
  tasks?: unknown
}

function currentSnapshot(snapshot: TaskProgressSnapshot | null): TaskProgressSnapshot | null {
  return snapshot ? createCurrentTaskProgressSnapshot(snapshot.tasks) : null
}

export function useTaskProgressSnapshot(
  sessionId: string | null,
  messages: Message[],
): TaskProgressSnapshot | null {
  const messageSnapshot = useMemo(
    () => sessionId ? extractLatestTaskProgressSnapshot(messages, sessionId) : null,
    [messages, sessionId],
  )
  const [eventState, setEventState] = useState<{
    sessionId: string | null
    snapshot: TaskProgressSnapshot | null
  }>({ sessionId, snapshot: null })

  if (eventState.sessionId !== sessionId) {
    setEventState({ sessionId, snapshot: null })
  }

  useEffect(() => {
    if (!sessionId) return
    let alive = true
    let eventVersion = 0
    let requestVersion = 0
    const accept = (tasks: unknown) => {
      const snapshot = taskProgressSnapshotFromTasks(tasks)
      if (snapshot) setEventState({ sessionId, snapshot })
    }
    const reload = () => {
      const request = ++requestVersion
      const beforeEvents = eventVersion
      void getTransport().call<unknown>("list_session_tasks", { sessionId }).then((tasks) => {
        if (alive && request === requestVersion && beforeEvents === eventVersion) accept(tasks)
      }).catch((error) => {
        if (alive) logger.warn("chat", "useTaskProgressSnapshot", "Failed to restore tasks", error)
      })
    }
    // Subscribe before loading so a slower seed cannot overwrite a live edit.
    const unlisten = getTransport().listen("task_updated", (raw) => {
      const payload = parsePayload<TaskUpdatedPayload>(raw)
      if (!alive || payload?.sessionId !== sessionId) return
      eventVersion += 1
      accept(payload.tasks)
    })
    const unlistenResync = getTransport().listen(TRANSPORT_EVENT_RESYNC_REQUIRED, reload)
    reload()
    return () => {
      alive = false
      unlisten()
      unlistenResync()
    }
  }, [sessionId])

  const eventSnapshot = eventState.sessionId === sessionId ? eventState.snapshot : null
  // Live/read ledger snapshots are authoritative even after a deletion makes
  // their newest timestamp older than the immutable transcript's snapshot.
  return currentSnapshot(eventSnapshot ?? messageSnapshot)
}
