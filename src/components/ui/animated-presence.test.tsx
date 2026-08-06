// @vitest-environment jsdom

import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

import { AnimatedCollapse } from "./animated-presence"

beforeEach(() => {
  vi.useFakeTimers()
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    callback(0)
    return 0
  })
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe("AnimatedCollapse", () => {
  test("clips visible overflow until the opening height transition settles", () => {
    const { rerender } = render(
      <AnimatedCollapse open={false} overflow="visible-when-open" unmountOnExit={false}>
        <div data-testid="collapse-content">content</div>
      </AnimatedCollapse>,
    )

    rerender(
      <AnimatedCollapse open overflow="visible-when-open" unmountOnExit={false}>
        <div data-testid="collapse-content">content</div>
      </AnimatedCollapse>,
    )

    const outer = screen.getByTestId("collapse-content").parentElement?.parentElement
    expect(outer?.classList.contains("overflow-hidden")).toBe(true)
    expect(outer?.classList.contains("overflow-visible")).toBe(false)

    act(() => vi.runAllTimers())
    expect(outer?.classList.contains("overflow-visible")).toBe(true)

    rerender(
      <AnimatedCollapse open={false} overflow="visible-when-open" unmountOnExit={false}>
        <div data-testid="collapse-content">content</div>
      </AnimatedCollapse>,
    )
    expect(outer?.classList.contains("overflow-hidden")).toBe(true)
  })
})
