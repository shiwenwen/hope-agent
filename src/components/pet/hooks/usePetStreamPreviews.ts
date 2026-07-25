import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { logger } from "@/lib/logger"
import { getTransport } from "@/lib/transport-provider"
import type { PetActivity } from "@/types/pet"
import { petPreviewText } from "../petPreviewText"

const EVENT_CHAT_STREAM_DELTA = "chat:stream_delta"
const MAX_BUFFER_CHARACTERS = 4_096
const MAX_PREVIEW_CHARACTERS = 240

interface StreamDeltaPayload {
  sessionId: string
  seq: number
  streamId?: string | null
  event: string
}

interface StreamSnapshot {
  streamId?: string | null
  throughSeq: number
  status: string
  events: Array<{ seq: number; event: string }>
}

interface StreamAccumulator {
  streamId?: string | null
  throughSeq: number
  content: string
}

interface SnapshotHandshake {
  identity: string
  buffered: StreamDeltaPayload[]
}

function boundedAppend(current: string, delta: string): string {
  const characters = Array.from(current + delta)
  if (characters.length <= MAX_BUFFER_CHARACTERS) return characters.join("")
  return characters.slice(-MAX_BUFFER_CHARACTERS).join("")
}

function textDelta(event: string): string | null {
  try {
    const parsed = JSON.parse(event) as { type?: unknown; content?: unknown }
    return parsed.type === "text_delta" && typeof parsed.content === "string"
      ? parsed.content
      : null
  } catch {
    return null
  }
}

function applyDelta(
  accumulator: StreamAccumulator,
  payload: StreamDeltaPayload,
): { accumulator: StreamAccumulator; contentChanged: boolean } {
  const nextStreamId = payload.streamId ?? accumulator.streamId
  const streamChanged =
    !!payload.streamId && !!accumulator.streamId && payload.streamId !== accumulator.streamId
  const base = streamChanged
    ? { streamId: nextStreamId, throughSeq: 0, content: "" }
    : { ...accumulator, streamId: nextStreamId }
  if (!streamChanged && payload.seq <= base.throughSeq) {
    return { accumulator: base, contentChanged: false }
  }
  const delta = textDelta(payload.event)
  return {
    accumulator: {
      ...base,
      throughSeq: Math.max(base.throughSeq, payload.seq),
      content: delta ? boundedAppend(base.content, delta) : base.content,
    },
    contentChanged: !!delta,
  }
}

export function petStreamPreviewLine(value: string): string {
  const collapsed = petPreviewText(value)
  const characters = Array.from(collapsed)
  if (characters.length <= MAX_PREVIEW_CHARACTERS) return collapsed
  return `…${characters.slice(-(MAX_PREVIEW_CHARACTERS - 1)).join("")}`
}

function activityIdentity(activity: PetActivity): string {
  return `${activity.activityId}:${activity.boundary ?? activity.updatedAt}`
}

/**
 * Projects the existing parent-chat stream into bounded, single-line Pet
 * previews. Only sessions already admitted by the authoritative Pet activity
 * snapshot are observed, so background/side-query model calls never create a
 * preview. Incognito activities deliberately remain text-free.
 */
export function usePetStreamPreviews(activities: PetActivity[]): ReadonlyMap<string, string> {
  const running = useMemo(
    () =>
      activities.filter(
        (activity) => activity.status === "running" && activity.titleKind !== "incognito",
      ),
    [activities],
  )
  const runningIdentity = running.map(activityIdentity).join("|")
  const eligibleRef = useRef(new Map<string, string>())
  eligibleRef.current = new Map(
    running.map((activity) => [activity.activityId, activityIdentity(activity)]),
  )

  const accumulatorsRef = useRef(new Map<string, StreamAccumulator>())
  const identitiesRef = useRef(new Map<string, string>())
  const handshakesRef = useRef(new Map<string, SnapshotHandshake>())
  const frameRef = useRef<number | null>(null)
  const mountedRef = useRef(true)
  const [previews, setPreviews] = useState<ReadonlyMap<string, string>>(new Map())

  const publish = useCallback(() => {
    if (frameRef.current !== null) return
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null
      if (!mountedRef.current) return
      const next = new Map<string, string>()
      for (const [sessionId, accumulator] of accumulatorsRef.current) {
        const preview = petStreamPreviewLine(accumulator.content)
        if (preview) next.set(sessionId, preview)
      }
      setPreviews(next)
    })
  }, [])

  const applyLiveDelta = useCallback(
    (payload: StreamDeltaPayload) => {
      const current = accumulatorsRef.current.get(payload.sessionId) ?? {
        streamId: payload.streamId,
        throughSeq: 0,
        content: "",
      }
      const result = applyDelta(current, payload)
      accumulatorsRef.current.set(payload.sessionId, result.accumulator)
      if (result.contentChanged) publish()
    },
    [publish],
  )

  useEffect(() => {
    return getTransport().listen(EVENT_CHAT_STREAM_DELTA, (raw) => {
      const payload = raw as Partial<StreamDeltaPayload> | null
      if (
        !payload?.sessionId ||
        typeof payload.seq !== "number" ||
        typeof payload.event !== "string" ||
        !eligibleRef.current.has(payload.sessionId)
      ) {
        return
      }
      const frame: StreamDeltaPayload = {
        sessionId: payload.sessionId,
        seq: payload.seq,
        streamId: payload.streamId,
        event: payload.event,
      }
      const handshake = handshakesRef.current.get(payload.sessionId)
      if (handshake) {
        handshake.buffered.push(frame)
      } else {
        applyLiveDelta(frame)
      }
    })
  }, [applyLiveDelta])

  useEffect(() => {
    const eligible = new Map(
      running.map((activity) => [activity.activityId, activityIdentity(activity)]),
    )
    let removed = false
    for (const sessionId of identitiesRef.current.keys()) {
      if (eligible.has(sessionId)) continue
      identitiesRef.current.delete(sessionId)
      accumulatorsRef.current.delete(sessionId)
      handshakesRef.current.delete(sessionId)
      removed = true
    }
    if (removed) publish()

    for (const activity of running) {
      const sessionId = activity.activityId
      const identity = activityIdentity(activity)
      if (
        identitiesRef.current.get(sessionId) === identity &&
        (accumulatorsRef.current.has(sessionId) || handshakesRef.current.has(sessionId))
      ) {
        continue
      }

      identitiesRef.current.set(sessionId, identity)
      accumulatorsRef.current.delete(sessionId)
      const handshake: SnapshotHandshake = { identity, buffered: [] }
      handshakesRef.current.set(sessionId, handshake)
      publish()

      void getTransport()
        .call<StreamSnapshot | null>("get_session_stream_snapshot", { sessionId })
        .then((snapshot) => {
          if (
            !mountedRef.current ||
            identitiesRef.current.get(sessionId) !== identity ||
            handshakesRef.current.get(sessionId) !== handshake
          ) {
            return
          }
          let accumulator: StreamAccumulator = {
            streamId: snapshot?.streamId,
            throughSeq: 0,
            content: "",
          }
          if (snapshot?.status === "running") {
            for (const event of snapshot.events) {
              accumulator = applyDelta(accumulator, {
                sessionId,
                streamId: snapshot.streamId,
                seq: event.seq,
                event: event.event,
              }).accumulator
            }
            accumulator.throughSeq = Math.max(accumulator.throughSeq, snapshot.throughSeq)
          }
          for (const payload of handshake.buffered) {
            accumulator = applyDelta(accumulator, payload).accumulator
          }
          handshakesRef.current.delete(sessionId)
          accumulatorsRef.current.set(sessionId, accumulator)
          publish()
        })
        .catch((error) => {
          if (
            !mountedRef.current ||
            identitiesRef.current.get(sessionId) !== identity ||
            handshakesRef.current.get(sessionId) !== handshake
          ) {
            return
          }
          let accumulator: StreamAccumulator = {
            throughSeq: 0,
            content: "",
          }
          for (const payload of handshake.buffered) {
            accumulator = applyDelta(accumulator, payload).accumulator
          }
          handshakesRef.current.delete(sessionId)
          accumulatorsRef.current.set(sessionId, accumulator)
          publish()
          logger.warn(
            "pet",
            "stream_preview_snapshot",
            "Failed to restore a running Pet stream preview",
            error,
          )
        })
    }
    // `runningIdentity` is the stable semantic dependency; snapshot object
    // refreshes with the same activities must not restart every handshake.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runningIdentity, publish])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current)
        frameRef.current = null
      }
      handshakesRef.current.clear()
    }
  }, [])

  return previews
}
