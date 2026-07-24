import { useEffect, useRef, useState } from "react"
import type { PetActivityStatus } from "@/types/pet"

export type PetAction =
  | "idle"
  | "run_right"
  | "run_left"
  | "wave"
  | "jump"
  | "sad"
  | "waiting"
  | "working"
  | "celebrate"

const ACTION_ROW: Record<PetAction, number> = {
  idle: 0,
  run_right: 1,
  run_left: 2,
  wave: 3,
  jump: 4,
  sad: 5,
  waiting: 6,
  working: 7,
  celebrate: 8,
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
  sad: { frames: [0, 1, 2, 3, 4, 5, 6, 7], durations: repeat(8, 140, 240) },
  waiting: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 150, 260) },
  working: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 120, 220) },
  celebrate: { frames: [0, 1, 2, 3, 4, 5], durations: repeat(6, 150, 280) },
}

function isOneShotAction(action: PetAction): boolean {
  return action === "wave" || action === "jump"
}

export function actionForStatus(status?: PetActivityStatus | null): PetAction {
  if (status === "needs_input") return "waiting"
  if (status === "blocked") return "sad"
  if (status === "ready") return "celebrate"
  if (status === "running") return "working"
  return "idle"
}

export function usePetAnimator(
  action: PetAction,
  onActionComplete?: (action: PetAction) => void,
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
  }, [action, reducedMotion])

  const descriptor = ACTION_ANIMATION[action]
  const frameIndex = cursor.action === action ? cursor.frameIndex : 0
  return {
    row: ACTION_ROW[action],
    frame: reducedMotion
      ? descriptor.frames[0]
      : (descriptor.frames[frameIndex] ?? descriptor.frames[0]),
  }
}
