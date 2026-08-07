import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type PointerEvent,
} from "react"

const HOVER_CARD_DELAY_MS = 450

function targetSuppressesHoverCard(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    Boolean(target.closest("button, input, [data-ha-tip], [data-ha-title-tip]"))
  )
}

/** Keeps the row-level card out of the way of its existing buttons and icon tips. */
export function useSessionHoverCard(enabled: boolean) {
  const [open, setOpen] = useState(false)
  const timerRef = useRef<number | null>(null)

  const close = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    setOpen(false)
  }, [])

  const schedule = useCallback(() => {
    if (!enabled || open || timerRef.current !== null) return
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null
      setOpen(true)
    }, HOVER_CARD_DELAY_MS)
  }, [enabled, open])

  useEffect(
    () => () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current)
    },
    [],
  )

  useEffect(() => {
    if (!enabled) queueMicrotask(close)
  }, [close, enabled])

  const triggerProps = useMemo(
    () => ({
      onPointerEnter: (event: PointerEvent<HTMLElement>) => {
        if (!targetSuppressesHoverCard(event.target)) schedule()
      },
      onPointerMove: (event: PointerEvent<HTMLElement>) => {
        if (targetSuppressesHoverCard(event.target)) close()
        else schedule()
      },
      onPointerLeave: close,
      onFocus: (event: FocusEvent<HTMLElement>) => {
        if (event.target !== event.currentTarget) {
          close()
          return
        }
        if (enabled) setOpen(true)
      },
      onBlur: (event: FocusEvent<HTMLElement>) => {
        if (!event.currentTarget.contains(event.relatedTarget)) close()
      },
    }),
    [close, enabled, schedule],
  )

  return { open: enabled && open, close, triggerProps }
}
