export type FocusInputModality = "pointer" | "keyboard"

const MODIFIER_ONLY_KEYS = new Set(["Alt", "AltGraph", "Control", "Meta", "Shift"])
const TEXT_ENTRY_INPUT_TYPES = new Set([
  "email",
  "number",
  "password",
  "search",
  "tel",
  "text",
  "url",
])

let uninstallCurrentTracker: (() => void) | null = null
const modalityBeforeTabKeydown = new WeakMap<KeyboardEvent, FocusInputModality>()

function editableTarget(target: EventTarget | null): Element | null {
  if (!(target instanceof Element)) return null
  const editable = target.closest(
    'input, textarea, [contenteditable="true"], [contenteditable=""], [role="textbox"]',
  )
  if (!editable) return null
  if (editable instanceof HTMLInputElement && !TEXT_ENTRY_INPUT_TYPES.has(editable.type)) {
    return null
  }
  return editable
}

/**
 * Keyboard use inside a pointer-focused editor must not suddenly paint a focus
 * ring: the caret already communicates focus, and editing shortcuts do not
 * move it. Tab always means keyboard navigation; keyboard interaction on
 * non-editable controls does too.
 */
export function shouldEnterKeyboardModality(event: KeyboardEvent): boolean {
  if (MODIFIER_ONLY_KEYS.has(event.key)) return false
  if (event.key === "Tab") return true
  if (editableTarget(event.target)) return false
  return true
}

export function setFocusInputModality(modality: FocusInputModality): void {
  document.documentElement.dataset.inputModality = modality
}

/**
 * A composer picker may use Tab as an accept key without moving focus. Restore
 * the modality captured before the global keydown listener treated that Tab as
 * navigation, so pointer-focused editors do not gain a keyboard focus ring.
 */
export function restoreFocusInputModalityAfterConsumedTab(event: KeyboardEvent): void {
  if (event.key !== "Tab") return
  const previousModality = modalityBeforeTabKeydown.get(event)
  if (!previousModality) return
  modalityBeforeTabKeydown.delete(event)
  setFocusInputModality(previousModality)
}

/** Install once per WebView; all Hope Agent window variants share this entrypoint. */
export function installFocusVisibilityTracker(): () => void {
  const root = document.documentElement
  root.dataset.inputModality ||= "pointer"
  root.dataset.focusIndicators ||= "auto"

  if (uninstallCurrentTracker) return uninstallCurrentTracker

  const onPointerDown = () => setFocusInputModality("pointer")
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Tab") {
      modalityBeforeTabKeydown.set(
        event,
        root.dataset.inputModality === "keyboard" ? "keyboard" : "pointer",
      )
    }
    if (shouldEnterKeyboardModality(event)) setFocusInputModality("keyboard")
  }

  document.addEventListener("pointerdown", onPointerDown, true)
  document.addEventListener("keydown", onKeyDown, true)

  uninstallCurrentTracker = () => {
    document.removeEventListener("pointerdown", onPointerDown, true)
    document.removeEventListener("keydown", onKeyDown, true)
    uninstallCurrentTracker = null
  }
  return uninstallCurrentTracker
}
