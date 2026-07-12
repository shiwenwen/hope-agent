import { isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"

const CLICKABLE_SELECTOR = [
  "button",
  "a[href]",
  "input[type='button']",
  "input[type='submit']",
  "input[type='reset']",
  "[role='button']",
  "[role='checkbox']",
  "[role='menuitem']",
  "[role='option']",
  "[role='radio']",
].join(",")

const PAIR_MAX_AGE_MS = 40
const PAIR_TIMESTAMP_TOLERANCE_MS = 1
const PAIR_POINT_TOLERANCE_PX = 2
const LATE_NATIVE_CLICK_WINDOW_MS = 120

export interface PointerClickSample {
  button: number
  buttons: number
  clientX: number
  clientY: number
  detail: number
  isPrimary: boolean
  pointerId: number
  pointerType: string
  timeStamp: number
}

interface PendingPointerUp {
  control: HTMLElement
  recordedAt: number
  sample: PointerClickSample
}

function toSample(event: PointerEvent): PointerClickSample {
  return {
    button: event.button,
    buttons: event.buttons,
    clientX: event.clientX,
    clientY: event.clientY,
    detail: event.detail,
    isPrimary: event.isPrimary,
    pointerId: event.pointerId,
    pointerType: event.pointerType,
    timeStamp: event.timeStamp,
  }
}

export function isInvertedClickPair(
  pointerUp: PointerClickSample,
  pointerDown: PointerClickSample,
  ageMs: number,
): boolean {
  return (
    ageMs >= 0 &&
    ageMs <= PAIR_MAX_AGE_MS &&
    pointerUp.pointerType === "mouse" &&
    pointerDown.pointerType === "mouse" &&
    pointerUp.isPrimary &&
    pointerDown.isPrimary &&
    pointerUp.pointerId === pointerDown.pointerId &&
    pointerUp.button === 0 &&
    pointerDown.button === 0 &&
    pointerUp.buttons === 0 &&
    pointerDown.buttons === 0 &&
    (pointerUp.detail === 0 || pointerUp.detail === 1) &&
    pointerDown.detail === 1 &&
    Math.abs(pointerUp.timeStamp - pointerDown.timeStamp) <= PAIR_TIMESTAMP_TOLERANCE_MS &&
    Math.abs(pointerUp.clientX - pointerDown.clientX) <= PAIR_POINT_TOLERANCE_PX &&
    Math.abs(pointerUp.clientY - pointerDown.clientY) <= PAIR_POINT_TOLERANCE_PX
  )
}

function findClickableControl(target: EventTarget | null): HTMLElement | null {
  if (!(target instanceof Element)) return null
  const control = target.closest<HTMLElement>(CLICKABLE_SELECTOR)
  if (!control || control.dataset.disableInvertedClickRecovery === "true") return null
  if (
    (control instanceof HTMLButtonElement || control instanceof HTMLInputElement) &&
    control.disabled
  ) {
    return null
  }
  if (control.getAttribute("aria-disabled") === "true") return null
  return control
}

function isMacOS(): boolean {
  return typeof navigator !== "undefined" && navigator.platform.toUpperCase().includes("MAC")
}

/**
 * Recover a WKWebView/macOS failure observed in Tauri where one physical click
 * is emitted as pointerup(detail=0|1) -> pointerdown(detail=1), with identical
 * pointer id, point and native timestamp, and no click event afterwards. The
 * first inverted gesture uses detail=0; subsequent gestures can use detail=1
 * because WebKit's internal pointer state remains inverted.
 *
 * Upstream reports:
 * - https://github.com/tauri-apps/tauri/issues/7219
 * - https://bugs.webkit.org/show_bug.cgi?id=219670
 *
 * WebKit #219670 reproduces this exact sequence with macOS tap-to-click after
 * switching to Japanese Romaji or some third-party Chinese input methods. It
 * remains open upstream, so keep this compatibility layer until system WebKit
 * ships and verifies a fix.
 *
 * The signature is deliberately strict so ordinary clicks, drags, touch,
 * keyboard activation and right-clicks remain entirely browser-native.
 */
export function installInvertedClickRecovery(): () => void {
  if (!isTauriMode() || !isMacOS()) return () => {}

  let pendingUp: PendingPointerUp | null = null
  let pendingUpExpiry: number | null = null
  let recoveryTimer: number | null = null
  let pendingRecoveryControl: HTMLElement | null = null
  let recoveredControl: HTMLElement | null = null
  let recoveredExpiry: number | null = null

  const clearPendingUp = () => {
    pendingUp = null
    if (pendingUpExpiry !== null) {
      window.clearTimeout(pendingUpExpiry)
      pendingUpExpiry = null
    }
  }

  const clearRecovered = () => {
    recoveredControl = null
    if (recoveredExpiry !== null) {
      window.clearTimeout(recoveredExpiry)
      recoveredExpiry = null
    }
  }

  const onPointerUp = (event: PointerEvent) => {
    const control = findClickableControl(event.target)
    if (
      !control ||
      !event.isTrusted ||
      event.pointerType !== "mouse" ||
      !event.isPrimary ||
      event.button !== 0 ||
      event.buttons !== 0 ||
      (event.detail !== 0 && event.detail !== 1)
    ) {
      clearPendingUp()
      return
    }
    clearPendingUp()
    pendingUp = { control, recordedAt: performance.now(), sample: toSample(event) }
    pendingUpExpiry = window.setTimeout(clearPendingUp, PAIR_MAX_AGE_MS)
  }

  const onPointerDown = (event: PointerEvent) => {
    const candidate = pendingUp
    const control = findClickableControl(event.target)
    if (
      !candidate ||
      !control ||
      control !== candidate.control ||
      !event.isTrusted ||
      !isInvertedClickPair(candidate.sample, toSample(event), performance.now() - candidate.recordedAt)
    ) {
      clearPendingUp()
      return
    }

    clearPendingUp()
    if (recoveryTimer !== null) window.clearTimeout(recoveryTimer)
    pendingRecoveryControl = control
    recoveryTimer = window.setTimeout(() => {
      recoveryTimer = null
      pendingRecoveryControl = null
      if (!control.isConnected || control.getAttribute("aria-disabled") === "true") return
      if (
        (control instanceof HTMLButtonElement || control instanceof HTMLInputElement) &&
        control.disabled
      ) {
        return
      }

      recoveredControl = control
      if (recoveredExpiry !== null) window.clearTimeout(recoveredExpiry)
      recoveredExpiry = window.setTimeout(clearRecovered, LATE_NATIVE_CLICK_WINDOW_MS)
      logger.debug(
        "ui_click_recovery",
        "installInvertedClickRecovery",
        "recovered inverted macOS pointer sequence",
        {
          tag: control.tagName,
          ariaLabel: control.getAttribute("aria-label"),
          pointerId: event.pointerId,
          timeStamp: event.timeStamp,
        },
      )
      control.click()
    }, 0)
  }

  const onClick = (event: MouseEvent) => {
    const control = findClickableControl(event.target)
    if (event.isTrusted && pendingRecoveryControl && control === pendingRecoveryControl) {
      if (recoveryTimer !== null) window.clearTimeout(recoveryTimer)
      recoveryTimer = null
      pendingRecoveryControl = null
      return
    }
    if (!recoveredControl || control !== recoveredControl) return
    if (!event.isTrusted) return

    // A late native click belongs to the same recovered gesture. Suppress it
    // so a toggle, send or destructive action cannot run twice.
    event.preventDefault()
    event.stopImmediatePropagation()
    clearRecovered()
  }

  document.addEventListener("pointerup", onPointerUp, true)
  document.addEventListener("pointerdown", onPointerDown, true)
  document.addEventListener("click", onClick, true)

  return () => {
    clearPendingUp()
    clearRecovered()
    if (recoveryTimer !== null) window.clearTimeout(recoveryTimer)
    pendingRecoveryControl = null
    document.removeEventListener("pointerup", onPointerUp, true)
    document.removeEventListener("pointerdown", onPointerDown, true)
    document.removeEventListener("click", onClick, true)
  }
}
