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

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  const editable = target.closest(
    'input, textarea, [contenteditable="true"], [contenteditable=""], [role="textbox"]',
  )
  if (!editable) return false
  return !(editable instanceof HTMLInputElement) || TEXT_ENTRY_INPUT_TYPES.has(editable.type)
}

function isTextEntryKeystroke(event: KeyboardEvent): boolean {
  if (event.isComposing || event.key === "Process" || event.key === "Dead") return true
  if (event.getModifierState("AltGraph")) return true
  if (!event.metaKey && !event.ctrlKey && !event.altKey) return true
  // macOS Option+character produces text rather than an application shortcut.
  return event.altKey && !event.metaKey && !event.ctrlKey && event.key.length === 1
}

/**
 * Text typed into a pointer-focused editor must not suddenly paint a focus
 * ring. Tab always means keyboard navigation; shortcuts and key interaction
 * on non-editable controls do too.
 */
export function shouldEnterKeyboardModality(event: KeyboardEvent): boolean {
  if (MODIFIER_ONLY_KEYS.has(event.key)) return false
  if (event.key === "Tab") return true
  if (isEditableTarget(event.target) && isTextEntryKeystroke(event)) {
    return false
  }
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
