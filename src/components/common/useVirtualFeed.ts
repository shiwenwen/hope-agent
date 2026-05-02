import { useEffect, useLayoutEffect, useRef, useState, useEffectEvent } from "react"
import { useVirtualizer } from "@tanstack/react-virtual"

type RowKey = string | number

interface UseVirtualFeedOptions<T> {
  rows: T[]
  getRowKey: (row: T, index: number) => RowKey
  estimateSize: (index: number) => number
  overscan?: number
  gap?: number
  paddingStart?: number
  paddingEnd?: number
  followOutput?: boolean
  followKey?: RowKey | null
  forceFollowKey?: RowKey | null
  resetKey?: RowKey | null
  canAnchorRow?: (row: T, index: number) => boolean
  onStartReached?: () => void | Promise<void>
  canLoadMore?: boolean
  loadingMore?: boolean
  startThreshold?: number
  bottomThreshold?: number
}

interface ScrollAnchor {
  key: RowKey
  offset: number
  rowCount: number
  scrollHeight: number
  scrollTop: number
}

function distanceFromBottom(el: HTMLElement): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight
}

export function useVirtualFeed<T>({
  rows,
  getRowKey,
  estimateSize,
  overscan = 8,
  gap = 0,
  paddingStart = 0,
  paddingEnd = 0,
  followOutput = false,
  followKey = null,
  forceFollowKey = null,
  resetKey = null,
  canAnchorRow,
  onStartReached,
  canLoadMore = false,
  loadingMore = false,
  startThreshold = 80,
  bottomThreshold = 80,
}: UseVirtualFeedOptions<T>) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const isAutoFollowPausedRef = useRef(false)
  const hasUnseenOutputRef = useRef(false)
  const isAtBottomRef = useRef(true)
  const lastScrollTopRef = useRef(0)
  const lastTouchYRef = useRef<number | null>(null)
  const startLoadPendingRef = useRef(false)
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null)
  const allowAnchorSizeCorrectionRef = useRef(false)
  const rafIdRef = useRef<number | null>(null)
  const anchorCorrectionRafRef = useRef<number | null>(null)
  const [isAutoFollowPaused, setIsAutoFollowPausedState] = useState(false)
  const [hasUnseenOutput, setHasUnseenOutputState] = useState(false)
  const [isAtBottom, setIsAtBottomState] = useState(true)

  const rowsRef = useRef(rows)
  const getRowKeyRef = useRef(getRowKey)
  const canAnchorRowRef = useRef(canAnchorRow)
  const onStartReachedRef = useRef(onStartReached)
  const canLoadMoreRef = useRef(canLoadMore)
  const loadingMoreRef = useRef(loadingMore)

  rowsRef.current = rows
  getRowKeyRef.current = getRowKey
  canAnchorRowRef.current = canAnchorRow
  onStartReachedRef.current = onStartReached
  canLoadMoreRef.current = canLoadMore
  loadingMoreRef.current = loadingMore

  const setIsAutoFollowPaused = (paused: boolean) => {
    if (isAutoFollowPausedRef.current === paused) return
    isAutoFollowPausedRef.current = paused
    setIsAutoFollowPausedState(paused)
  }
  const setIsAutoFollowPausedEffectEvent = useEffectEvent(setIsAutoFollowPaused)

  const setHasUnseenOutput = (unseen: boolean) => {
    if (hasUnseenOutputRef.current === unseen) return
    hasUnseenOutputRef.current = unseen
    setHasUnseenOutputState(unseen)
  }
  const setHasUnseenOutputEffectEvent = useEffectEvent(setHasUnseenOutput)

  const setIsAtBottom = (atBottom: boolean) => {
    if (isAtBottomRef.current === atBottom) return
    isAtBottomRef.current = atBottom
    setIsAtBottomState(atBottom)
  }
  const setIsAtBottomEffectEvent = useEffectEvent(setIsAtBottom)

  const updateAtBottom = (el: HTMLElement) => {
    const atBottom = distanceFromBottom(el) <= bottomThreshold
    setIsAtBottom(atBottom)
    return atBottom
  }
  const updateAtBottomEffectEvent = useEffectEvent(updateAtBottom)

  const pauseAutoFollow = (markUnseen = false) => {
    setIsAutoFollowPaused(true)
    if (markUnseen) {
      setHasUnseenOutput(true)
    }
  }
  const pauseAutoFollowEffectEvent = useEffectEvent(pauseAutoFollow)

  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual is isolated here so list callers don't pass its functions through memoized boundaries.
  const virtualizer = useVirtualizer<HTMLDivElement, HTMLDivElement>({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    getItemKey: (index) => {
      const row = rows[index]
      return row ? getRowKey(row, index) : index
    },
    gap,
    overscan,
    paddingStart,
    paddingEnd,
    useAnimationFrameWithResizeObserver: true,
  })
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, instance) => {
    const shouldMaintainViewport = item.start < (instance.scrollOffset ?? 0)
    if (allowAnchorSizeCorrectionRef.current) return shouldMaintainViewport
    if (isAutoFollowPausedRef.current) return false
    return shouldMaintainViewport
  }

  const scrollToBottom = (behavior: ScrollBehavior = "auto") => {
    if (rowsRef.current.length === 0) return
    requestAnimationFrame(() => {
      const el = scrollRef.current
      if (!el || isAutoFollowPausedRef.current) return
      virtualizer.scrollToIndex(rowsRef.current.length - 1, {
        align: "end",
        behavior,
      })
      requestAnimationFrame(() => {
        const latest = scrollRef.current
        if (!latest || isAutoFollowPausedRef.current) return
        latest.scrollTop = Math.max(0, latest.scrollHeight - latest.clientHeight)
        updateAtBottom(latest)
      })
    })
  }
  const scrollToBottomEffectEvent = useEffectEvent(scrollToBottom)

  const resumeAutoFollow = (behavior: ScrollBehavior = "auto") => {
    setIsAutoFollowPaused(false)
    setHasUnseenOutput(false)
    scrollToBottom(behavior)
  }
  const resumeAutoFollowEffectEvent = useEffectEvent(resumeAutoFollow)

  const captureAnchor = () => {
    const el = scrollRef.current
    if (!el) return
    const first = virtualizer.getVirtualItems().find((item) => {
      const row = rowsRef.current[item.index]
      return row && (canAnchorRowRef.current?.(row, item.index) ?? true)
    })
    if (!first) return
    const row = rowsRef.current[first.index]
    if (!row) return
    pendingAnchorRef.current = {
      key: getRowKeyRef.current(row, first.index),
      offset: first.start - el.scrollTop,
      rowCount: rowsRef.current.length,
      scrollHeight: el.scrollHeight,
      scrollTop: el.scrollTop,
    }
  }

  const scheduleAnchorCorrectionEnd = () => {
    if (anchorCorrectionRafRef.current !== null) {
      cancelAnimationFrame(anchorCorrectionRafRef.current)
    }

    anchorCorrectionRafRef.current = requestAnimationFrame(() => {
      anchorCorrectionRafRef.current = requestAnimationFrame(() => {
        allowAnchorSizeCorrectionRef.current = false
        anchorCorrectionRafRef.current = null
      })
    })
  }

  const triggerStartLoad = () => {
    if (!onStartReachedRef.current) return
    if (!canLoadMoreRef.current || loadingMoreRef.current || startLoadPendingRef.current) return

    captureAnchor()
    pauseAutoFollow(false)
    startLoadPendingRef.current = true
    void Promise.resolve(onStartReachedRef.current())
      .catch(() => {
        startLoadPendingRef.current = false
        pendingAnchorRef.current = null
      })
      .finally(() => {
        window.setTimeout(() => {
          if (!startLoadPendingRef.current) return
          startLoadPendingRef.current = false
          pendingAnchorRef.current = null
        }, 250)
      })
  }
  const triggerStartLoadEffectEvent = useEffectEvent(triggerStartLoad)
  const scheduleAnchorCorrectionEndEffectEvent = useEffectEvent(scheduleAnchorCorrectionEnd)

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    lastScrollTopRef.current = el.scrollTop

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY < 0) pauseAutoFollowEffectEvent(/* markUnseen */ followOutput)
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "PageUp" || event.key === "ArrowUp" || event.key === "Home") {
        pauseAutoFollowEffectEvent(/* markUnseen */ followOutput)
      }
    }

    const handleTouchStart = (event: TouchEvent) => {
      lastTouchYRef.current = event.touches[0]?.clientY ?? null
    }

    const handleTouchMove = (event: TouchEvent) => {
      const currentY = event.touches[0]?.clientY
      const previousY = lastTouchYRef.current
      if (currentY === undefined || previousY === null) {
        lastTouchYRef.current = currentY ?? null
        return
      }
      if (currentY > previousY) {
        pauseAutoFollowEffectEvent(/* markUnseen */ followOutput)
      }
      lastTouchYRef.current = currentY
    }

    const handleTouchEnd = () => {
      lastTouchYRef.current = null
    }

    const handleScroll = () => {
      const currentTop = el.scrollTop
      const prevTop = lastScrollTopRef.current
      lastScrollTopRef.current = currentTop

      if (currentTop <= startThreshold) {
        triggerStartLoadEffectEvent()
      }

      const bottomDistance = distanceFromBottom(el)
      const atBottom = bottomDistance <= bottomThreshold
      setIsAtBottomEffectEvent(atBottom)

      if (currentTop < prevTop && !atBottom) {
        pauseAutoFollowEffectEvent(/* markUnseen */ followOutput)
        return
      }

      if (atBottom) {
        setIsAutoFollowPausedEffectEvent(false)
        setHasUnseenOutputEffectEvent(false)
      }
    }

    el.addEventListener("wheel", handleWheel, { passive: true })
    el.addEventListener("touchstart", handleTouchStart, { passive: true })
    el.addEventListener("touchmove", handleTouchMove, { passive: true })
    el.addEventListener("touchend", handleTouchEnd)
    el.addEventListener("touchcancel", handleTouchEnd)
    el.addEventListener("keydown", handleKeyDown)
    el.addEventListener("scroll", handleScroll, { passive: true })
    return () => {
      el.removeEventListener("wheel", handleWheel)
      el.removeEventListener("touchstart", handleTouchStart)
      el.removeEventListener("touchmove", handleTouchMove)
      el.removeEventListener("touchend", handleTouchEnd)
      el.removeEventListener("touchcancel", handleTouchEnd)
      el.removeEventListener("keydown", handleKeyDown)
      el.removeEventListener("scroll", handleScroll)
    }
  }, [bottomThreshold, followOutput, startThreshold])

  useLayoutEffect(() => {
    const anchor = pendingAnchorRef.current
    if (!anchor) return
    const el = scrollRef.current
    if (!el) return
    if (rows.length === anchor.rowCount && el.scrollHeight === anchor.scrollHeight) return

    const index = rows.findIndex((row, i) => getRowKey(row, i) === anchor.key)
    allowAnchorSizeCorrectionRef.current = true

    const fallbackTop = Math.max(0, anchor.scrollTop + el.scrollHeight - anchor.scrollHeight)

    if (index >= 0) {
      const beforeIndexScrollTop = el.scrollTop
      virtualizer.scrollToIndex(index, { align: "start" })
      el.scrollTop =
        el.scrollTop === beforeIndexScrollTop
          ? fallbackTop
          : Math.max(0, el.scrollTop - anchor.offset)
    } else {
      el.scrollTop = fallbackTop
    }

    lastScrollTopRef.current = el.scrollTop
    updateAtBottom(el)
    pendingAnchorRef.current = null
    startLoadPendingRef.current = false
    scheduleAnchorCorrectionEndEffectEvent()
  }, [getRowKey, rows, virtualizer])

  useEffect(() => {
    return () => {
      if (anchorCorrectionRafRef.current !== null) {
        cancelAnimationFrame(anchorCorrectionRafRef.current)
        anchorCorrectionRafRef.current = null
      }
    }
  }, [])

  const lastResetKeyRef = useRef<RowKey | null>(null)
  useLayoutEffect(() => {
    if (resetKey === null) return
    if (resetKey === lastResetKeyRef.current) return
    lastResetKeyRef.current = resetKey
    setIsAutoFollowPausedEffectEvent(false)
    setHasUnseenOutputEffectEvent(false)
    scrollToBottomEffectEvent("auto")
  }, [resetKey])

  const lastFollowKeyRef = useRef<RowKey | null>(followKey)
  useLayoutEffect(() => {
    if (followKey === null || followKey === lastFollowKeyRef.current) return
    lastFollowKeyRef.current = followKey
    if (!isAutoFollowPausedRef.current) {
      scrollToBottomEffectEvent("auto")
      return
    }
    setHasUnseenOutputEffectEvent(true)
  }, [followKey])

  const lastForceFollowKeyRef = useRef<RowKey | null>(forceFollowKey)
  useLayoutEffect(() => {
    if (forceFollowKey === null || forceFollowKey === lastForceFollowKeyRef.current) return
    lastForceFollowKeyRef.current = forceFollowKey
    resumeAutoFollowEffectEvent("auto")
  }, [forceFollowKey])

  const wasFollowingOutputRef = useRef(false)
  useEffect(() => {
    if (!followOutput) {
      wasFollowingOutputRef.current = false
      return
    }

    if (isAutoFollowPaused) {
      return
    }

    if (!wasFollowingOutputRef.current) {
      wasFollowingOutputRef.current = true
    }

    const tick = () => {
      const el = scrollRef.current
      if (el && !isAutoFollowPausedRef.current) {
        const target = el.scrollHeight - el.clientHeight
        if (target - el.scrollTop > 1) {
          el.scrollTop = target
        }
        updateAtBottomEffectEvent(el)
      }
      rafIdRef.current = requestAnimationFrame(tick)
    }
    rafIdRef.current = requestAnimationFrame(tick)

    return () => {
      if (rafIdRef.current !== null) {
        cancelAnimationFrame(rafIdRef.current)
        rafIdRef.current = null
      }
    }
  }, [followOutput, isAutoFollowPaused])

  return {
    scrollRef,
    virtualizer,
    virtualItems: virtualizer.getVirtualItems(),
    totalSize: Math.max(virtualizer.getTotalSize(), virtualizer.scrollRect?.height ?? 0),
    scrollToBottom,
    resumeAutoFollow,
    pauseAutoFollow,
    isAtBottom,
    isAutoFollowPaused,
    hasUnseenOutput,
  }
}
