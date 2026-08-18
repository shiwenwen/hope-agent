import { useEffect, useState } from "react"

/**
 * Live window width, rAF-coalesced. Prefer `useViewportMediaQuery` for plain
 * breakpoints; this is for panels that shrink continuously with the window.
 */
export function useViewportWidth(): number {
  const [width, setWidth] = useState(() => (typeof window === "undefined" ? 0 : window.innerWidth))

  useEffect(() => {
    if (typeof window === "undefined") return
    let frame: number | null = null
    const update = () => {
      frame = null
      setWidth((current) => (current === window.innerWidth ? current : window.innerWidth))
    }
    const onResize = () => {
      if (frame === null) frame = window.requestAnimationFrame(update)
    }
    update()
    window.addEventListener("resize", onResize)
    return () => {
      if (frame !== null) window.cancelAnimationFrame(frame)
      window.removeEventListener("resize", onResize)
    }
  }, [])

  return width
}
