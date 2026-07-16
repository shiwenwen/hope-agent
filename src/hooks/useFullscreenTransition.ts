import { useCallback, useEffect, useRef, useState, type RefCallback } from "react"
import { flushSync } from "react-dom"

import { UI_EASING, UI_MOTION } from "@/components/ui/motion"

interface UseFullscreenTransitionOptions {
  maximized: boolean
  onMaximizedChange: (maximized: boolean) => void
}

interface UseFullscreenTransitionResult<T extends HTMLElement> {
  ref: RefCallback<T>
  animating: boolean
  transitionTo: (maximized: boolean) => void
  toggle: () => void
  reset: () => void
}

/**
 * Animates an element between its current layout rect and a controlled
 * fullscreen layout. The caller still owns the `maximized` state and the CSS
 * that applies `fixed inset-0`; this hook only coordinates the FLIP motion.
 */
export function useFullscreenTransition<T extends HTMLElement = HTMLDivElement>({
  maximized,
  onMaximizedChange,
}: UseFullscreenTransitionOptions): UseFullscreenTransitionResult<T> {
  const elementRef = useRef<T | null>(null)
  const animationRef = useRef<Animation | null>(null)
  const [animating, setAnimating] = useState(false)

  const ref = useCallback<RefCallback<T>>((node) => {
    elementRef.current = node
  }, [])

  const cancelAnimation = useCallback(() => {
    const animation = animationRef.current
    if (!animation) return
    animation.onfinish = null
    animation.oncancel = null
    animation.cancel()
    animationRef.current = null
  }, [])

  const reset = useCallback(() => {
    cancelAnimation()
    setAnimating(false)
    onMaximizedChange(false)
  }, [cancelAnimation, onMaximizedChange])

  const transitionTo = useCallback(
    (nextMaximized: boolean) => {
      if (animating || maximized === nextMaximized) return

      const element = elementRef.current
      const reduceMotion =
        window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false
      if (!element || reduceMotion || typeof element.animate !== "function") {
        onMaximizedChange(nextMaximized)
        return
      }

      cancelAnimation()
      const startRect = element.getBoundingClientRect()
      let endRect: DOMRect

      if (nextMaximized) {
        flushSync(() => onMaximizedChange(true))
        endRect = element.getBoundingClientRect()
      } else {
        // Measure the real restored layout, then return to fullscreen before
        // the browser paints. The live subtree stays mounted throughout.
        flushSync(() => onMaximizedChange(false))
        endRect = element.getBoundingClientRect()
        flushSync(() => onMaximizedChange(true))
      }

      if (
        startRect.width <= 0 ||
        startRect.height <= 0 ||
        endRect.width <= 0 ||
        endRect.height <= 0
      ) {
        onMaximizedChange(nextMaximized)
        return
      }

      const currentRect = element.getBoundingClientRect()
      const targetRect = nextMaximized ? startRect : endRect
      const x = targetRect.left - currentRect.left
      const y = targetRect.top - currentRect.top
      const scaleX = targetRect.width / currentRect.width
      const scaleY = targetRect.height / currentRect.height
      const sameRect =
        Math.abs(x) < 0.5 &&
        Math.abs(y) < 0.5 &&
        Math.abs(scaleX - 1) < 0.001 &&
        Math.abs(scaleY - 1) < 0.001
      const layoutTransform = sameRect
        ? "translate3d(0, 0, 0) scale(0.985, 0.985)"
        : `translate3d(${x}px, ${y}px, 0) scale(${scaleX}, ${scaleY})`
      const identityTransform = "translate3d(0, 0, 0) scale(1, 1)"
      const transformOrigin = sameRect ? "center center" : "top left"
      const animation = element.animate(
        nextMaximized
          ? [
              { transform: layoutTransform, transformOrigin },
              { transform: identityTransform, transformOrigin },
            ]
          : [
              { transform: identityTransform, transformOrigin },
              { transform: layoutTransform, transformOrigin },
            ],
        {
          duration: UI_MOTION.panelSurface,
          easing: UI_EASING.emphasized,
          fill: "both",
        },
      )

      animationRef.current = animation
      setAnimating(true)
      animation.onfinish = () => {
        if (animationRef.current !== animation) return
        animation.onfinish = null
        animation.oncancel = null
        animationRef.current = null
        if (!nextMaximized) flushSync(() => onMaximizedChange(false))
        animation.cancel()
        setAnimating(false)
      }
      animation.oncancel = () => {
        if (animationRef.current !== animation) return
        animationRef.current = null
        setAnimating(false)
      }
    },
    [animating, cancelAnimation, maximized, onMaximizedChange],
  )

  const toggle = useCallback(() => {
    transitionTo(!maximized)
  }, [maximized, transitionTo])

  useEffect(
    () => () => {
      cancelAnimation()
    },
    [cancelAnimation],
  )

  return { ref, animating, transitionTo, toggle, reset }
}
