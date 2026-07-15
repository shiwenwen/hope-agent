// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { useReadableSurface } from "./useReadableSurface"

let focusListener: ((focused: boolean) => void) | null = null

vi.mock("@/lib/notifications", () => ({
  isAppWindowFocused: () => true,
  subscribeAppWindowFocus: (listener: (focused: boolean) => void) => {
    focusListener = listener
    listener(true)
    return () => {
      focusListener = null
    }
  },
}))

function setDocumentVisibility(value: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", { configurable: true, value })
  document.dispatchEvent(new Event("visibilitychange"))
}

afterEach(() => {
  cleanup()
  focusListener = null
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" })
})

describe("useReadableSurface", () => {
  it("requires the selected app view, a visible document, and window focus", () => {
    const { result, rerender } = renderHook(
      ({ selected }: { selected: boolean }) => useReadableSurface(selected),
      { initialProps: { selected: true } },
    )

    expect(result.current).toBe(true)

    act(() => focusListener?.(false))
    expect(result.current).toBe(false)

    act(() => focusListener?.(true))
    expect(result.current).toBe(true)

    act(() => setDocumentVisibility("hidden"))
    expect(result.current).toBe(false)

    act(() => setDocumentVisibility("visible"))
    rerender({ selected: false })
    expect(result.current).toBe(false)
  })
})
