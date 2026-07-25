import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react"
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

export type PetOverlayMode = "none" | "bubble" | "tray" | "menu"
export type HorizontalPlacement = "left" | "right"
export type VerticalPlacement = "above" | "below"

export interface OverlaySize {
  width: number
  height: number
}

export interface CommittedPetLayout {
  mode: PetOverlayMode
  horizontal: HorizontalPlacement
  vertical: VerticalPlacement
  visible: boolean
}

export interface PetAvailableSpace {
  left: number
  right: number
  above: number
  below: number
}

const PET_WIDTH = 120
const PET_HEIGHT = 128
const PET_ANCHOR_X = 60
const PET_ANCHOR_Y = 116
const OVERLAY_GAP = 8
const SAFE_MARGIN = 12
const OVERLAY_TRANSITION_MS = 180

function waitForPaint(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()))
}

function placementOverflow(
  availableBefore: number,
  availableAfter: number,
  before: number,
  after: number,
) {
  return Math.max(0, before - availableBefore) + Math.max(0, after - availableAfter)
}

/**
 * Chooses the orientation that keeps the pet's foot anchor stable while
 * minimizing work-area overflow. Keeping the current orientation on a tie
 * prevents edge jitter as localized text changes by a pixel or two.
 */
export function chooseOverlayPlacement(
  width: number,
  height: number,
  space: PetAvailableSpace,
  preferred: Pick<CommittedPetLayout, "horizontal" | "vertical">,
): Pick<CommittedPetLayout, "horizontal" | "vertical"> {
  const leftOverflow = placementOverflow(
    space.left,
    space.right,
    width - PET_ANCHOR_X,
    PET_ANCHOR_X,
  )
  const rightOverflow = placementOverflow(
    space.left,
    space.right,
    PET_ANCHOR_X,
    width - PET_ANCHOR_X,
  )
  const aboveOverflow = placementOverflow(
    space.above,
    space.below,
    height - SAFE_MARGIN,
    SAFE_MARGIN,
  )
  const belowOverflow = placementOverflow(
    space.above,
    space.below,
    PET_ANCHOR_Y,
    height - PET_ANCHOR_Y,
  )

  return {
    horizontal:
      leftOverflow === rightOverflow
        ? preferred.horizontal
        : leftOverflow < rightOverflow
          ? "left"
          : "right",
    vertical:
      aboveOverflow === belowOverflow
        ? preferred.vertical
        : aboveOverflow < belowOverflow
          ? "above"
          : "below",
  }
}

interface KeyedOverlaySize {
  key: string
  size: OverlaySize
}

interface CommittedPetGeometry {
  width: number
  height: number
  anchor: { x: number; y: number }
}

function useMeasuredSize(
  ref: RefObject<HTMLElement | null>,
  active: boolean,
  measurementKey: string,
): OverlaySize | null {
  const [measurement, setMeasurement] = useState<KeyedOverlaySize | null>(null)

  useLayoutEffect(() => {
    if (!active || !ref.current) return
    const node = ref.current
    let cancelled = false
    const update = () => {
      if (cancelled) return
      const rect = node.getBoundingClientRect()
      const next = { width: Math.ceil(rect.width), height: Math.ceil(rect.height) }
      setMeasurement((current) =>
        current?.key === measurementKey &&
        Math.abs(current.size.width - next.width) <= 1 &&
        Math.abs(current.size.height - next.height) <= 1
          ? current
          : { key: measurementKey, size: next },
      )
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(node)
    void document.fonts?.ready.then(update)
    return () => {
      cancelled = true
      observer.disconnect()
    }
  }, [active, measurementKey, ref])

  return active && measurement?.key === measurementKey ? measurement.size : null
}

export function selectOverlayMeasurement(
  hidden: OverlaySize | null,
  live: OverlaySize | null,
  preferLive: boolean,
): OverlaySize | null {
  return preferLive ? (live ?? hidden) : hidden
}

export function usePetWindowLayout(
  desiredMode: PetOverlayMode,
  measureRef: RefObject<HTMLElement | null>,
  suspended = false,
  liveOverlayRef?: RefObject<HTMLElement | null>,
): CommittedPetLayout {
  const [committed, setCommitted] = useState<CommittedPetLayout>({
    mode: "none",
    horizontal: "left",
    vertical: "above",
    visible: true,
  })
  const emptyLiveRef = useRef<HTMLElement | null>(null)
  const hiddenMeasured = useMeasuredSize(measureRef, desiredMode !== "none", desiredMode)
  const liveMeasurementKey = `${committed.mode}:${committed.horizontal}:${committed.vertical}`
  const liveMeasured = useMeasuredSize(
    liveOverlayRef ?? emptyLiveRef,
    desiredMode !== "none" && committed.mode === desiredMode,
    liveMeasurementKey,
  )
  // The hidden twin is authoritative before the native window expands. Once
  // the desired overlay is committed, observe the real card so internal Pet
  // state (Ask pagination, Other input, async error copy) can resize the
  // native window even though the hidden twin has independent React state.
  const measured = selectOverlayMeasurement(
    hiddenMeasured,
    liveMeasured,
    committed.mode === desiredMode && committed.visible,
  )
  const committedRef = useRef(committed)
  const lastStableLayoutRef = useRef(committed)
  const anchorRef = useRef({ x: PET_ANCHOR_X, y: PET_ANCHOR_Y })
  const geometryRef = useRef<CommittedPetGeometry>({
    width: PET_WIDTH,
    height: PET_HEIGHT,
    anchor: { x: PET_ANCHOR_X, y: PET_ANCHOR_Y },
  })
  const lastStableGeometryRef = useRef<CommittedPetGeometry>(geometryRef.current)
  const revisionRef = useRef(0)
  const desiredGenerationRef = useRef(0)
  const previousModeRef = useRef(desiredMode)
  const [reducedMotion, setReducedMotion] = useState(false)

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)")
    const update = () => setReducedMotion(query.matches)
    update()
    query.addEventListener("change", update)
    return () => query.removeEventListener("change", update)
  }, [])

  useEffect(() => {
    if (previousModeRef.current !== desiredMode) {
      previousModeRef.current = desiredMode
      desiredGenerationRef.current += 1
    }
    const generation = desiredGenerationRef.current
    let closeTimer: ReturnType<typeof setTimeout> | null = null
    let retryTimer: ReturnType<typeof setTimeout> | null = null
    let retryCount = 0
    let cancelled = false
    // This effect owns the whole transition, including retries. Recovery must
    // return to the last fully committed frame, never the temporary hidden
    // destination used while a native resize is in flight.
    const rollbackLayout = lastStableLayoutRef.current
    const rollbackGeometry = {
      ...lastStableGeometryRef.current,
      anchor: { ...lastStableGeometryRef.current.anchor },
    }

    if (suspended) return

    const apply = async () => {
      let width = PET_WIDTH
      let height = PET_HEIGHT
      let horizontal: HorizontalPlacement = committedRef.current.horizontal
      let vertical: VerticalPlacement = committedRef.current.vertical

      if (desiredMode !== "none") {
        if (!measured) return
        width = Math.min(440, Math.max(PET_WIDTH, measured.width))
        height = Math.min(640, PET_HEIGHT + OVERLAY_GAP + measured.height)
        const win = getCurrentWindow()
        const [position, scale, monitor] = await Promise.all([
          win.outerPosition(),
          win.scaleFactor(),
          currentMonitor(),
        ])
        if (cancelled || generation !== desiredGenerationRef.current) return
        if (monitor) {
          const anchorScreenX = position.x + anchorRef.current.x * scale
          const anchorScreenY = position.y + anchorRef.current.y * scale
          const area = monitor.workArea
          const placement = chooseOverlayPlacement(
            width,
            height,
            {
              left: (anchorScreenX - area.position.x) / scale - SAFE_MARGIN,
              right: (area.position.x + area.size.width - anchorScreenX) / scale - SAFE_MARGIN,
              above: (anchorScreenY - area.position.y) / scale - SAFE_MARGIN,
              below: (area.position.y + area.size.height - anchorScreenY) / scale - SAFE_MARGIN,
            },
            committedRef.current,
          )
          horizontal = placement.horizontal
          vertical = placement.vertical
        }
      }

      // Opening from PetOnly needs a real hidden frame before native resize.
      // Besides enabling the CSS enter transition, committing the destination
      // orientation first keeps the pet aligned to the same foot anchor while
      // the transparent WebView grows, avoiding a one-frame jump/flash near a
      // monitor edge where the overlay opens right or below.
      const opening = desiredMode !== "none" && committedRef.current.mode === "none"
      if (opening) {
        const prepared = {
          mode: desiredMode,
          horizontal,
          vertical,
          visible: false,
        } satisfies CommittedPetLayout
        committedRef.current = prepared
        setCommitted(prepared)
        await waitForPaint()
        if (cancelled || generation !== desiredGenerationRef.current) return
      }

      const nextAnchor = {
        x: horizontal === "left" ? width - PET_ANCHOR_X : PET_ANCHOR_X,
        y: vertical === "above" ? height - SAFE_MARGIN : PET_ANCHOR_Y,
      }
      let result: { applied: boolean; layoutRevision: number } | null = null
      let layoutRevision = 0
      // A renderer reload resets its local counter while the native window can
      // retain a newer revision. Adopt the native counter and retry once so a
      // hot reload or renderer recovery cannot leave an invisible stale layout.
      for (let attempt = 0; attempt < 2; attempt += 1) {
        layoutRevision = ++revisionRef.current
        result = await getTransport().call<{ applied: boolean; layoutRevision: number }>(
          "pet_apply_window_bounds_cmd",
          {
            request: {
              layoutRevision,
              width,
              height,
              previousAnchorX: anchorRef.current.x,
              previousAnchorY: anchorRef.current.y,
              nextAnchorX: desiredMode === "none" ? PET_ANCHOR_X : nextAnchor.x,
              nextAnchorY: desiredMode === "none" ? PET_ANCHOR_Y : nextAnchor.y,
            },
          },
        )
        if (result.applied) break
        revisionRef.current = Math.max(revisionRef.current, result.layoutRevision)
      }
      if (
        cancelled ||
        generation !== desiredGenerationRef.current
      ) {
        return
      }
      if (!result?.applied || result.layoutRevision !== layoutRevision) {
        throw new Error("pet_window_bounds_not_applied")
      }
      anchorRef.current = desiredMode === "none" ? { x: PET_ANCHOR_X, y: PET_ANCHOR_Y } : nextAnchor
      geometryRef.current = {
        width,
        height,
        anchor: { ...anchorRef.current },
      }
      const nextCommitted = {
        mode: desiredMode,
        horizontal,
        vertical,
        visible: true,
      } satisfies CommittedPetLayout
      if (opening) {
        // Let the resized native surface paint once while the overlay is still
        // transparent, then reveal it. Mounting directly in the visible state
        // skips CSS transitions and makes WebView resize repaint look like the
        // whole pet blinked.
        await waitForPaint()
        if (cancelled || generation !== desiredGenerationRef.current) return
      }
      committedRef.current = nextCommitted
      lastStableLayoutRef.current = nextCommitted
      lastStableGeometryRef.current = geometryRef.current
      setCommitted(nextCommitted)
    }
    const runApply = () => {
      void apply().catch(async () => {
        logger.warn("pet", "window_layout", "Failed to apply pet window bounds", {
          retryCount,
        })
        if (!cancelled && generation === desiredGenerationRef.current && retryCount < 2) {
          retryCount += 1
          retryTimer = setTimeout(runApply, 120 * retryCount)
          return
        }
        if (!cancelled && generation === desiredGenerationRef.current) {
          // A rejected invoke may still have reached the native side. Apply a
          // newer revision for the old geometry before restoring React state,
          // so an expanded WebView cannot expose a bubble in PetOnly bounds.
          try {
            let restored = false
            for (let attempt = 0; attempt < 2; attempt += 1) {
              const layoutRevision = ++revisionRef.current
              const result = await getTransport().call<{
                applied: boolean
                layoutRevision: number
              }>("pet_apply_window_bounds_cmd", {
                request: {
                  layoutRevision,
                  width: rollbackGeometry.width,
                  height: rollbackGeometry.height,
                  previousAnchorX: anchorRef.current.x,
                  previousAnchorY: anchorRef.current.y,
                  nextAnchorX: rollbackGeometry.anchor.x,
                  nextAnchorY: rollbackGeometry.anchor.y,
                },
              })
              if (result.applied && result.layoutRevision === layoutRevision) {
                restored = true
                break
              }
              revisionRef.current = Math.max(revisionRef.current, result.layoutRevision)
            }
            if (!restored) {
              throw new Error("pet_window_bounds_rollback_not_applied")
            }
          } catch {
            logger.warn(
              "pet",
              "window_layout_rollback",
              "Failed to restore the previous pet window bounds",
            )
          }
          if (cancelled || generation !== desiredGenerationRef.current) return
          anchorRef.current = { ...rollbackGeometry.anchor }
          geometryRef.current = rollbackGeometry
          committedRef.current = rollbackLayout
          setCommitted(rollbackLayout)
        }
      })
    }

    // Fade the currently committed overlay before *any* mode replacement.
    // Without this bubble -> tray (or tray -> bubble) resizes the native
    // window while the old surface is still painted, producing a clipped or
    // stretched frame during the command round-trip.
    if (committedRef.current.mode !== "none" && committedRef.current.mode !== desiredMode) {
      const closing = { ...committedRef.current, visible: false }
      committedRef.current = closing
      setCommitted(closing)
      closeTimer = setTimeout(runApply, reducedMotion ? 0 : OVERLAY_TRANSITION_MS)
    } else {
      runApply()
    }

    return () => {
      cancelled = true
      if (closeTimer) clearTimeout(closeTimer)
      if (retryTimer) clearTimeout(retryTimer)
    }
  }, [desiredMode, measured, reducedMotion, suspended])

  return committed
}
