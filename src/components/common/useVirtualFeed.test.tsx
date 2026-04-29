// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { useVirtualFeed } from "./useVirtualFeed"

const virtualizerMock = vi.hoisted(() => ({
  scrollToIndex: vi.fn(),
  measureElement: vi.fn(),
  latestInstance: undefined as
    | {
        shouldAdjustScrollPositionOnItemSizeChange?: (
          item: { start: number },
          delta: number,
          instance: { scrollOffset: number | null },
        ) => boolean
      }
    | undefined,
}))

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: vi.fn((options: { count: number; getItemKey: (index: number) => string | number }) => {
    const instance = {
      scrollToIndex: virtualizerMock.scrollToIndex,
      measureElement: virtualizerMock.measureElement,
      getVirtualItems: () =>
        Array.from({ length: options.count }, (_, index) => ({
          index,
          key: options.getItemKey(index),
          start: index * 40,
        })),
      getTotalSize: () => options.count * 40,
      scrollRect: { height: 300 },
      shouldAdjustScrollPositionOnItemSizeChange: undefined,
    }
    virtualizerMock.latestInstance = instance
    return instance
  }),
}))

interface Row {
  id: string
}

const rows: Row[] = [{ id: "a" }, { id: "b" }, { id: "c" }]

let rafCallbacks: FrameRequestCallback[] = []

function flushRaf() {
  const callbacks = rafCallbacks
  rafCallbacks = []
  callbacks.forEach((callback) => callback(performance.now()))
}

function setScrollMetrics(el: HTMLElement, scrollTop: number) {
  Object.defineProperty(el, "scrollHeight", {
    value: 1000,
    configurable: true,
  })
  Object.defineProperty(el, "clientHeight", {
    value: 300,
    configurable: true,
  })
  el.scrollTop = scrollTop
}

function FeedHarness({
  followKey,
  forceFollowKey = null,
  followOutput = false,
  resetKey = "session-a",
}: {
  followKey: string
  forceFollowKey?: string | null
  followOutput?: boolean
  resetKey?: string
}) {
  const feed = useVirtualFeed({
    rows,
    getRowKey: (row) => row.id,
    estimateSize: () => 40,
    followKey,
    forceFollowKey,
    followOutput,
    resetKey,
    bottomThreshold: 80,
  })

  return (
    <div>
      <div ref={feed.scrollRef} data-testid="scroller" />
      <span data-testid="paused">{String(feed.isAutoFollowPaused)}</span>
      <span data-testid="unseen">{String(feed.hasUnseenOutput)}</span>
      <button type="button" onClick={() => feed.resumeAutoFollow("auto")}>
        jump
      </button>
    </div>
  )
}

describe("useVirtualFeed auto-follow", () => {
  beforeEach(() => {
    rafCallbacks = []
    virtualizerMock.scrollToIndex.mockClear()
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      rafCallbacks.push(callback)
      return rafCallbacks.length
    })
    vi.stubGlobal("cancelAnimationFrame", vi.fn())
  })

  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  test("does not scroll to the latest row after the user detaches during streaming", () => {
    const { rerender } = render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)

    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    expect(screen.getByTestId("paused").textContent).toBe("true")

    virtualizerMock.scrollToIndex.mockClear()
    rerender(<FeedHarness followKey="b" followOutput />)

    expect(screen.getByTestId("paused").textContent).toBe("true")
    expect(screen.getByTestId("unseen").textContent).toBe("true")
    expect(virtualizerMock.scrollToIndex).not.toHaveBeenCalled()
  })

  test("cancels an already queued bottom scroll when the user detaches before the animation frame", () => {
    const { rerender } = render(<FeedHarness followKey="a" followOutput />)

    flushRaf()
    flushRaf()
    rafCallbacks = []
    virtualizerMock.scrollToIndex.mockClear()

    rerender(<FeedHarness followKey="b" followOutput />)
    const scroller = screen.getByTestId("scroller")
    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)
    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    expect(screen.getByTestId("paused").textContent).toBe("true")

    flushRaf()

    expect(virtualizerMock.scrollToIndex).not.toHaveBeenCalled()
  })

  test("does not let virtualizer size corrections move the viewport while detached", () => {
    render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)
    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    const shouldAdjust = virtualizerMock.latestInstance?.shouldAdjustScrollPositionOnItemSizeChange
    expect(shouldAdjust?.({ start: 0 }, 40, { scrollOffset: 500 })).toBe(false)
  })

  test("does not detach when a touch gesture moves toward the bottom", () => {
    render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    fireEvent.touchStart(scroller, { touches: [{ clientY: 200 }] })
    fireEvent.touchMove(scroller, { touches: [{ clientY: 120 }] })

    expect(screen.getByTestId("paused").textContent).toBe("false")
    expect(screen.getByTestId("unseen").textContent).toBe("false")
  })

  test("detaches when a touch gesture moves toward older messages", () => {
    render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    fireEvent.touchStart(scroller, { touches: [{ clientY: 120 }] })
    fireEvent.touchMove(scroller, { touches: [{ clientY: 200 }] })

    expect(screen.getByTestId("paused").textContent).toBe("true")
    expect(screen.getByTestId("unseen").textContent).toBe("true")
  })

  test("resumeAutoFollow scrolls to bottom and allows following again", () => {
    render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)
    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    fireEvent.click(screen.getByRole("button", { name: "jump" }))
    flushRaf()
    flushRaf()

    expect(screen.getByTestId("paused").textContent).toBe("false")
    expect(screen.getByTestId("unseen").textContent).toBe("false")
    expect(virtualizerMock.scrollToIndex).toHaveBeenCalledWith(2, {
      align: "end",
      behavior: "auto",
    })
    expect(scroller.scrollTop).toBe(700)
  })

  test("forceFollowKey resumes following for explicit new-message jumps", () => {
    const { rerender } = render(<FeedHarness followKey="a" followOutput />)
    const scroller = screen.getByTestId("scroller")

    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)
    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    virtualizerMock.scrollToIndex.mockClear()
    rerender(<FeedHarness followKey="b" forceFollowKey="b" followOutput />)
    flushRaf()
    flushRaf()

    expect(screen.getByTestId("paused").textContent).toBe("false")
    expect(screen.getByTestId("unseen").textContent).toBe("false")
    expect(virtualizerMock.scrollToIndex).toHaveBeenCalledWith(2, {
      align: "end",
      behavior: "auto",
    })
  })

  test("resetKey restores following when the feed switches context", () => {
    const { rerender } = render(<FeedHarness followKey="a" followOutput resetKey="session-a" />)
    const scroller = screen.getByTestId("scroller")

    setScrollMetrics(scroller, 700)
    fireEvent.scroll(scroller)
    setScrollMetrics(scroller, 500)
    fireEvent.scroll(scroller)

    expect(screen.getByTestId("paused").textContent).toBe("true")

    virtualizerMock.scrollToIndex.mockClear()
    rerender(<FeedHarness followKey="a" followOutput resetKey="session-b" />)
    flushRaf()
    flushRaf()

    expect(screen.getByTestId("paused").textContent).toBe("false")
    expect(screen.getByTestId("unseen").textContent).toBe("false")
    expect(virtualizerMock.scrollToIndex).toHaveBeenCalledWith(2, {
      align: "end",
      behavior: "auto",
    })
  })
})
