import { useEffect, useState } from "react"

/**
 * Reactive `window.matchMedia` wrapper. Returns whether `query` currently
 * matches and re-renders when it crosses the boundary. The query string is the
 * effect dependency, so callers can pass a dynamically computed breakpoint
 * (e.g. derived from live panel widths) and the listener re-binds on change.
 *
 * Used by both the chat view and the knowledge view to drive responsive
 * auto-collapse of side panels without a debounced resize listener.
 */
export function useViewportMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(query).matches,
  )

  useEffect(() => {
    if (typeof window === "undefined") return

    const media = window.matchMedia(query)
    const handleChange = () => setMatches(media.matches)
    handleChange()
    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", handleChange)
      return () => media.removeEventListener("change", handleChange)
    }
    media.addListener(handleChange)
    return () => media.removeListener(handleChange)
  }, [query])

  return matches
}
