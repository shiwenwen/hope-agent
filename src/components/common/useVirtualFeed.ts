import { useCallback, useEffect, useLayoutEffect, useRef } from "react"
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
  scrollHeight: number
  scrollTop: number
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
  resetKey = null,
  canAnchorRow,
  onStartReached,
  canLoadMore = false,
  loadingMore = false,
  startThreshold = 80,
  bottomThreshold = 80,
}: UseVirtualFeedOptions<T>) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const isUserScrolledUpRef = useRef(false)
  const lastScrollTopRef = useRef(0)
  const startLoadPendingRef = useRef(false)
  const pendingAnchorRef = useRef<ScrollAnchor | null>(null)
  const rafIdRef = useRef<number | null>(null)

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

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      if (rowsRef.current.length === 0) return
      requestAnimationFrame(() => {
        const el = scrollRef.current
        if (!el) return
        virtualizer.scrollToIndex(rowsRef.current.length - 1, {
          align: "end",
          behavior,
        })
        requestAnimationFrame(() => {
          const latest = scrollRef.current
          if (!latest || isUserScrolledUpRef.current) return
          latest.scrollTop = Math.max(0, latest.scrollHeight - latest.clientHeight)
        })
      })
    },
    [virtualizer],
  )

  const captureAnchor = useCallback(() => {
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
      scrollHeight: el.scrollHeight,
      scrollTop: el.scrollTop,
    }
  }, [virtualizer])

  const triggerStartLoad = useCallback(() => {
    if (!onStartReachedRef.current) return
    if (!canLoadMoreRef.current || loadingMoreRef.current || startLoadPendingRef.current) return

    captureAnchor()
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
  }, [captureAnchor])

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    lastScrollTopRef.current = el.scrollTop

    const pauseAutoScroll = () => {
      isUserScrolledUpRef.current = true
    }

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY < 0) pauseAutoScroll()
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "PageUp" || event.key === "ArrowUp" || event.key === "Home") {
        pauseAutoScroll()
      }
    }

    const handleScroll = () => {
      const currentTop = el.scrollTop
      const prevTop = lastScrollTopRef.current
      lastScrollTopRef.current = currentTop

      if (currentTop <= startThreshold) {
        triggerStartLoad()
      }

      const distanceFromBottom = el.scrollHeight - currentTop - el.clientHeight
      if (currentTop < prevTop && distanceFromBottom > bottomThreshold) {
        isUserScrolledUpRef.current = true
        return
      }
      if (
        isUserScrolledUpRef.current &&
        currentTop > prevTop &&
        distanceFromBottom <= bottomThreshold
      ) {
        isUserScrolledUpRef.current = false
      }
    }

    el.addEventListener("wheel", handleWheel, { passive: true })
    el.addEventListener("touchmove", pauseAutoScroll, { passive: true })
    el.addEventListener("keydown", handleKeyDown)
    el.addEventListener("scroll", handleScroll, { passive: true })
    return () => {
      el.removeEventListener("wheel", handleWheel)
      el.removeEventListener("touchmove", pauseAutoScroll)
      el.removeEventListener("keydown", handleKeyDown)
      el.removeEventListener("scroll", handleScroll)
    }
  }, [bottomThreshold, startThreshold, triggerStartLoad])

  useLayoutEffect(() => {
    const anchor = pendingAnchorRef.current
    if (!anchor) return
    const el = scrollRef.current
    if (!el) return

    const index = rows.findIndex((row, i) => getRowKey(row, i) === anchor.key)
    requestAnimationFrame(() => {
      const latest = scrollRef.current
      if (!latest) return
      if (index >= 0) {
        virtualizer.scrollToIndex(index, { align: "start" })
        requestAnimationFrame(() => {
          const next = scrollRef.current
          if (!next) return
          next.scrollTop = Math.max(0, next.scrollTop - anchor.offset)
        })
      } else {
        latest.scrollTop = Math.max(0, anchor.scrollTop + latest.scrollHeight - anchor.scrollHeight)
      }
      pendingAnchorRef.current = null
      startLoadPendingRef.current = false
    })
  }, [getRowKey, rows, virtualizer])

  useLayoutEffect(() => {
    if (resetKey === null) return
    isUserScrolledUpRef.current = false
    scrollToBottom("auto")
  }, [resetKey, scrollToBottom])

  const lastFollowKeyRef = useRef<RowKey | null>(followKey)
  useLayoutEffect(() => {
    if (followKey === null || followKey === lastFollowKeyRef.current) return
    lastFollowKeyRef.current = followKey
    if (!isUserScrolledUpRef.current) {
      scrollToBottom("auto")
    }
  }, [followKey, scrollToBottom])

  const wasFollowingOutputRef = useRef(false)
  useEffect(() => {
    if (!followOutput) {
      if (wasFollowingOutputRef.current && !isUserScrolledUpRef.current) {
        scrollToBottom("auto")
      }
      wasFollowingOutputRef.current = false
      return
    }

    if (!wasFollowingOutputRef.current) {
      isUserScrolledUpRef.current = false
      wasFollowingOutputRef.current = true
    }

    const tick = () => {
      const el = scrollRef.current
      if (el && !isUserScrolledUpRef.current) {
        const target = el.scrollHeight - el.clientHeight
        if (target - el.scrollTop > 1) {
          el.scrollTop = target
        }
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
  }, [followOutput, scrollToBottom])

  return {
    scrollRef,
    virtualizer,
    virtualItems: virtualizer.getVirtualItems(),
    totalSize: Math.max(virtualizer.getTotalSize(), virtualizer.scrollRect?.height ?? 0),
    scrollToBottom,
  }
}
