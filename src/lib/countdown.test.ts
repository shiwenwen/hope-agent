// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { formatRemaining, useCountdownRemainingSec } from "./countdown"

describe("formatRemaining", () => {
  it("formats seconds, minutes and hours compactly", () => {
    expect(formatRemaining(0)).toBe("0s")
    expect(formatRemaining(-5)).toBe("0s")
    expect(formatRemaining(45)).toBe("45s")
    expect(formatRemaining(200)).toBe("3m 20s")
    expect(formatRemaining(3900)).toBe("1h 5m")
  })
})

describe("useCountdownRemainingSec", () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    cleanup()
    vi.useRealTimers()
  })

  it("returns null without a deadline and whole seconds with one", () => {
    const { result: none } = renderHook(() => useCountdownRemainingSec(null))
    expect(none.current).toBeNull()

    const deadline = Date.now() + 10_500
    const { result } = renderHook(() => useCountdownRemainingSec(deadline))
    expect(result.current).toBe(11)
  })

  it("ticks down once per second on the shared ticker and clamps at 0", () => {
    const deadline = Date.now() + 2_000
    const { result } = renderHook(() => useCountdownRemainingSec(deadline))
    expect(result.current).toBe(2)
    act(() => {
      vi.advanceTimersByTime(1_000)
    })
    expect(result.current).toBe(1)
    act(() => {
      vi.advanceTimersByTime(5_000)
    })
    // Past the deadline the snapshot stays clamped at 0, never negative.
    expect(result.current).toBe(0)
  })

  it("stops the shared interval once the last subscriber unmounts", () => {
    const spy = vi.spyOn(window, "clearInterval")
    const a = renderHook(() => useCountdownRemainingSec(Date.now() + 5_000))
    const b = renderHook(() => useCountdownRemainingSec(Date.now() + 5_000))
    const before = spy.mock.calls.length
    a.unmount()
    expect(spy.mock.calls.length).toBe(before)
    b.unmount()
    expect(spy.mock.calls.length).toBe(before + 1)
    spy.mockRestore()
  })
})
