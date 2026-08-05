import { useEffect, useRef, useState } from "react"
import { parsePayload } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"

const PET_INACTIVE_POINTER_EVENT = "pet:inactive_pointer"

interface PetInactivePointerEvent {
  inside: boolean
  x: number
  y: number
}

export type PetInactiveHoverAction = "dismiss" | "reply" | "stop" | null

export interface PetInactiveHoverTarget {
  activityId: string | null
  action: PetInactiveHoverAction
  pet: boolean
}

export type PetInactivePointerCallback = (
  x: number | null,
  y: number | null,
  target: PetInactiveHoverTarget,
) => void

const EMPTY_TARGET: PetInactiveHoverTarget = {
  activityId: null,
  action: null,
  pet: false,
}

function sameTarget(a: PetInactiveHoverTarget, b: PetInactiveHoverTarget): boolean {
  return a.activityId === b.activityId && a.action === b.action && a.pet === b.pet
}

export function resolvePetInactiveHoverTarget(x: number, y: number): PetInactiveHoverTarget {
  const element = document.elementFromPoint(x, y)
  if (!element) return EMPTY_TARGET
  const activityId = element.closest<HTMLElement>("[data-pet-activity-id]")?.dataset.petActivityId
  const rawAction = element.closest<HTMLElement>("[data-pet-hover-action]")?.dataset.petHoverAction
  const action: PetInactiveHoverAction = ["dismiss", "reply", "stop"].includes(rawAction ?? "")
    ? (rawAction as PetInactiveHoverAction)
    : null
  return {
    activityId: activityId || null,
    action,
    pet: Boolean(element.closest("[data-pet-sprite]")),
  }
}

/**
 * WebKit intentionally suppresses DOM hover for background windows on macOS.
 * The native PetWindow bridge emits only pointer coordinates that fall inside
 * this window; resolve them against the live DOM and expose a small declarative
 * hover target without synthesizing mouse events or stealing application focus.
 */
export function usePetInactivePointer(
  onPointer?: PetInactivePointerCallback,
): PetInactiveHoverTarget {
  const [target, setTarget] = useState<PetInactiveHoverTarget>(EMPTY_TARGET)
  const onPointerRef = useRef(onPointer)

  useEffect(() => {
    onPointerRef.current = onPointer
  }, [onPointer])

  useEffect(
    () =>
      getTransport().listen(PET_INACTIVE_POINTER_EVENT, (raw) => {
        const payload = parsePayload<PetInactivePointerEvent>(raw)
        const inside = payload?.inside && Number.isFinite(payload.x) && Number.isFinite(payload.y)
        const next = inside ? resolvePetInactiveHoverTarget(payload.x, payload.y) : EMPTY_TARGET
        onPointerRef.current?.(inside ? payload.x : null, inside ? payload.y : null, next)
        setTarget((current) => (sameTarget(current, next) ? current : next))
      }),
    [],
  )

  return target
}
