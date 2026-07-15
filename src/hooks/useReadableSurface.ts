import { useEffect, useState } from "react"
import { isAppWindowFocused, subscribeAppWindowFocus } from "@/lib/notifications"

function documentIsVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState !== "hidden"
}

/**
 * True only while a product surface is actually available for reading: its
 * app-level view is selected, the document is visible, and the native/browser
 * window has focus. Message-tail visibility is intentionally composed by the
 * chat caller because it is specific to the transcript viewport.
 */
export function useReadableSurface(surfaceVisible: boolean): boolean {
  const [windowFocused, setWindowFocused] = useState(isAppWindowFocused)
  const [documentVisible, setDocumentVisible] = useState(documentIsVisible)

  useEffect(() => subscribeAppWindowFocus(setWindowFocused), [])

  useEffect(() => {
    if (typeof document === "undefined") return
    const handleVisibilityChange = () => setDocumentVisible(documentIsVisible())
    document.addEventListener("visibilitychange", handleVisibilityChange)
    return () => document.removeEventListener("visibilitychange", handleVisibilityChange)
  }, [])

  return surfaceVisible && documentVisible && windowFocused
}
