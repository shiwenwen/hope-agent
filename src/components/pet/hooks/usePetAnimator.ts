import { useEffect, useRef, useState } from "react"
import type { PetActivityStatus } from "@/types/pet"

export type PetAction =
  | "idle"
  | "run_right"
  | "run_left"
  | "wave"
  | "jump"
  | "failed"
  | "waiting"
  | "running"
  | "review"

export type PetLookTarget = "neutral" | number | null

interface PetLookRect {
  left: number
  top: number
  width: number
  height: number
}

const ACTION_ROW: Record<PetAction, number> = {
  idle: 0,
  run_right: 1,
  run_left: 2,
  wave: 3,
  jump: 4,
  failed: 5,
  waiting: 6,
  running: 7,
  review: 8,
}

type AnimationDescriptor = {
  frames: readonly number[]
  durations: readonly number[]
}

const repeat = (count: number, duration: number, lastDuration: number): readonly number[] =>
  Array.from({ length: count }, (_, index) => (index === count - 1 ? lastDuration : duration))

/** Codex-compatible first-nine-row timing contract. */
const ACTION_ANIMATION: Record<PetAction, AnimationDescriptor> = {
  idle: { frames: [0, 1, 2, 3, 4, 5], durations: [280, 110, 110, 140, 140, 320] },
  run_right: { frames: [0, 1, 2, 3, 4, 5, 6, 7], durations: repeat(8, 120, 220) },
  run_left: { frames: [0, 1, 2, 3, 4, 5, 6, 7], durations: repeat(8, 120, 220) },
  wave: {
    frames: [0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3],
    durations: [140, 140, 140, 280, 140, 140, 140, 280, 140, 140, 140, 280],
  },
  jump: { frames: [0, 1, 2, 3, 4], durations: repeat(5, 140, 280) },
  failed: { frames: [0, 1, 2, 3, 4, 5, 6, 7], durations: repeat(8, 140, 240) },
  waiting: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 150, 260) },
  running: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 120, 220) },
  review: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 150, 280) },
}

function isOneShotAction(action: PetAction): boolean {
  return action === "wave" || action === "jump"
}

export function actionForStatus(status?: PetActivityStatus | null): PetAction {
  if (status === "needs_input") return "waiting"
  if (status === "blocked") return "failed"
  if (status === "ready") return "review"
  if (status === "running") return "running"
  return "idle"
}

/** Map a pointer vector to Codex v2's 16 clockwise look frames (0° is up). */
export function lookTargetForPointer(
  clientX: number,
  clientY: number,
  rect: PetLookRect,
): PetLookTarget {
  if (!Number.isFinite(clientX) || !Number.isFinite(clientY) || rect.width <= 0 || rect.height <= 0)
    return null
  const dx = clientX - (rect.left + rect.width / 2)
  const dy = clientY - (rect.top + rect.height / 2)
  const deadzone = Math.max(8, Math.min(rect.width, rect.height) * 0.16)
  if (Math.hypot(dx, dy) <= deadzone) return "neutral"
  const clockwiseDegrees = (Math.atan2(dx, -dy) * 180) / Math.PI
  return Math.round((clockwiseDegrees + 360) / 22.5) % 16
}

export function usePetAnimator(
  action: PetAction,
  onActionComplete?: (action: PetAction) => void,
  lookTarget: PetLookTarget = null,
): { row: number; frame: number } {
  const [cursor, setCursor] = useState<{ action: PetAction; frameIndex: number }>(() => ({
    action,
    frameIndex: 0,
  }))
  const [reducedMotion, setReducedMotion] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  )
  const onActionCompleteRef = useRef(onActionComplete)

  useEffect(() => {
    onActionCompleteRef.current = onActionComplete
  }, [onActionComplete])

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)")
    const update = () => setReducedMotion(query.matches)
    query.addEventListener("change", update)
    return () => query.removeEventListener("change", update)
  }, [])

  useEffect(() => {
    const descriptor = ACTION_ANIMATION[action]
    const oneShot = isOneShotAction(action)
    if (action === "idle" && lookTarget !== null) return
    if (reducedMotion) {
      if (!oneShot) return
      const completionTimer = setTimeout(() => onActionCompleteRef.current?.(action), 240)
      return () => clearTimeout(completionTimer)
    }
    let timer: ReturnType<typeof setTimeout> | null = null
    let cancelled = false
    let index = 0
    let deadline = performance.now() + (descriptor.durations[index] ?? 150)
    const clearTimer = () => {
      if (timer) clearTimeout(timer)
      timer = null
    }
    const schedule = () => {
      clearTimer()
      if (cancelled || document.visibilityState !== "visible") return
      timer = setTimeout(
        () => {
          if (cancelled || document.visibilityState !== "visible") return
          const next = index + 1
          if (oneShot && next >= descriptor.frames.length) {
            onActionCompleteRef.current?.(action)
            return
          }
          index = next % descriptor.frames.length
          setCursor({ action, frameIndex: index })
          // Advance exactly one logical frame. We intentionally do not catch up
          // elapsed intervals after background throttling.
          deadline = performance.now() + (descriptor.durations[index] ?? 150)
          schedule()
        },
        Math.max(0, deadline - performance.now()),
      )
    }
    const onVisibilityChange = () => {
      clearTimer()
      if (document.visibilityState === "visible") {
        deadline = performance.now() + (descriptor.durations[index] ?? 150)
        schedule()
      }
    }
    document.addEventListener("visibilitychange", onVisibilityChange)
    schedule()
    return () => {
      cancelled = true
      clearTimer()
      document.removeEventListener("visibilitychange", onVisibilityChange)
    }
  }, [action, lookTarget, reducedMotion])

  if (!reducedMotion && action === "idle" && lookTarget !== null) {
    if (lookTarget === "neutral") return { row: 0, frame: 6 }
    const direction = Math.max(0, Math.min(15, Math.trunc(lookTarget)))
    return { row: 9 + Math.floor(direction / 8), frame: direction % 8 }
  }

  const descriptor = ACTION_ANIMATION[action]
  const frameIndex = cursor.action === action ? cursor.frameIndex : 0
  return {
    row: ACTION_ROW[action],
    frame: reducedMotion
      ? descriptor.frames[0]
      : (descriptor.frames[frameIndex] ?? descriptor.frames[0]),
  }
}
