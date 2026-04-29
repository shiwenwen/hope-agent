import { useEffect, useMemo, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
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

function latestSnapshot(
  fromMessages: TaskProgressSnapshot | null,
  fromEvent: TaskProgressSnapshot | null,
): TaskProgressSnapshot | null {
  if (!fromMessages) return fromEvent
  if (!fromEvent) return fromMessages

  const messageUpdatedAt = Math.max(
    ...fromMessages.tasks.map((task) => Date.parse(task.updatedAt)).filter(Number.isFinite),
    0,
  )
  const eventUpdatedAt = Math.max(
    ...fromEvent.tasks.map((task) => Date.parse(task.updatedAt)).filter(Number.isFinite),
    0,
  )

  return eventUpdatedAt >= messageUpdatedAt ? fromEvent : fromMessages
}

function currentSnapshot(snapshot: TaskProgressSnapshot | null): TaskProgressSnapshot | null {
  return snapshot ? createCurrentTaskProgressSnapshot(snapshot.tasks) : null
}

export function useTaskProgressSnapshot(
  sessionId: string | null,
  messages: Message[],
): TaskProgressSnapshot | null {
  const messageSnapshot = useMemo(
    () => extractLatestTaskProgressSnapshot(messages),
    [messages],
  )
  const [eventSnapshot, setEventSnapshot] = useState<TaskProgressSnapshot | null>(null)

  useEffect(() => {
    setEventSnapshot(null)
  }, [sessionId])

  useEffect(() => {
    if (!sessionId) return
    return getTransport().listen("task_updated", (raw) => {
      const payload = raw as TaskUpdatedPayload
      if (payload.sessionId !== sessionId) return
      setEventSnapshot(taskProgressSnapshotFromTasks(payload.tasks))
    })
  }, [sessionId])

  return currentSnapshot(latestSnapshot(messageSnapshot, eventSnapshot))
}
