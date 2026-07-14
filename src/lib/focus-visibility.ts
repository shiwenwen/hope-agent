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

/** Install once per WebView; all Hope Agent window variants share this entrypoint. */
export function installFocusVisibilityTracker(): () => void {
  const root = document.documentElement
  root.dataset.inputModality ||= "pointer"
  root.dataset.focusIndicators ||= "auto"

  if (uninstallCurrentTracker) return uninstallCurrentTracker

  const onPointerDown = () => setFocusInputModality("pointer")
  const onKeyDown = (event: KeyboardEvent) => {
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
