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
  return !isNativeContextMenuAllowedTarget(event.target)
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
