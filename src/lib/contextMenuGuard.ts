import { isTauriMode } from "@/lib/transport"

export function installDesktopContextMenuGuard(): void {
  if (!import.meta.env.PROD || !isTauriMode()) return
  if (typeof document === "undefined") return

  document.addEventListener("contextmenu", (event) => {
    if (!shouldSuppressNativeContextMenu(event)) return
    event.preventDefault()
  })
}

export function shouldSuppressNativeContextMenu(
  event: Pick<MouseEvent, "defaultPrevented" | "target">,
): boolean {
  if (event.defaultPrevented) return false
  // A live text selection is precisely the case where the user wants the
  // native "Copy" (respecting exactly what they highlighted). Suppressing it
  // here would strand them with no selection-aware copy, so let the native
  // menu through — same rationale as the input/contenteditable exception.
  if (hasActiveTextSelection(event.target)) return false
  return !isNativeContextMenuAllowedTarget(event.target)
}

/** True when there is a non-collapsed text selection and the right-click
 *  landed inside it — the standard-browser precondition for a selection-aware
 *  "Copy". Shared by the desktop context-menu guard and the chat message
 *  context menu so both defer to native copy instead of overriding the
 *  clipboard with whole-element text. Pure DOM/selection logic, so it behaves
 *  identically in the Tauri webview and the web (HTTP) client. */
export function hasActiveTextSelection(target: EventTarget | null): boolean {
  if (typeof window === "undefined" || typeof window.getSelection !== "function") {
    return false
  }
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return false
  if (!selection.toString().trim()) return false

  const node = target instanceof Node ? target : null
  if (!node) return true
  for (let i = 0; i < selection.rangeCount; i++) {
    if (selection.getRangeAt(i).intersectsNode(node)) return true
  }
  return false
}

export function isNativeContextMenuAllowedTarget(target: EventTarget | null): boolean {
  const element = elementFromTarget(target)
  if (!element) return false

  const host = element.closest("input, textarea, [contenteditable]")
  if (!host) return false
  if (host instanceof HTMLInputElement) return true
  if (host instanceof HTMLTextAreaElement) return true
  if (!(host instanceof HTMLElement)) return false

  const attr = host.getAttribute("contenteditable")?.toLowerCase()
  return (
    attr === "" ||
    attr === "true" ||
    attr === "plaintext-only" ||
    host.isContentEditable
  )
}

function elementFromTarget(target: EventTarget | null): Element | null {
  if (typeof Element !== "undefined" && target instanceof Element) {
    return target
  }
  if (typeof Node !== "undefined" && target instanceof Node) {
    return target.parentElement
  }
  return null
}
