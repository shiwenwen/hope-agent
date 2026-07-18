/**
 * Suspend page-level interactions for the duration of a drag gesture:
 * body cursor + text selection, and pointer events on every iframe (an
 * iframe under the pointer would otherwise swallow move events mid-drag).
 * Returns a restore function; calling it twice is harmless.
 */
export function suspendPageInteractions(cursor: string): () => void {
  const iframes = Array.from(document.querySelectorAll("iframe"))
  iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = "none"))
  document.body.style.cursor = cursor
  document.body.style.userSelect = "none"
  let restored = false
  return () => {
    if (restored) return
    restored = true
    document.body.style.cursor = ""
    document.body.style.userSelect = ""
    iframes.forEach((f) => ((f as HTMLElement).style.pointerEvents = ""))
  }
}
