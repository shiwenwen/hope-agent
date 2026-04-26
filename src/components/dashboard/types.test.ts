import { describe, expect, test, vi } from "vitest"

import { chartName, chartNumber, formatDelta, formatNumber } from "./types"

describe("dashboard format helpers", () => {
  test("formats compact numbers and deltas", () => {
    expect(formatNumber(999)).toBe("999")
    expect(formatNumber(1_250)).toBe("1.3K")
    expect(formatNumber(1_250_000)).toBe("1.3M")
    expect(formatDelta(null)).toBe("")
    expect(formatDelta(12.345)).toBe("+12.3%")
    expect(formatDelta(-4.56)).toBe("-4.6%")
    expect(formatDelta(1_200)).toBe("+1.2K%")
  })

  test("normalizes recharts formatter values defensively", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {})

    expect(chartNumber(42)).toBe(42)
    expect(chartNumber("42.5")).toBe(42.5)
    expect(chartNumber("nope")).toBe(0)
    expect(chartNumber([7, 8])).toBe(7)
    expect(chartNumber([])).toBe(0)
    expect(chartNumber({ value: 1 })).toBe(0)

    expect(chartName("tokens")).toBe("tokens")
    expect(chartName(123)).toBe("123")
    expect(chartName({})).toBe("")

    warn.mockRestore()
  })
})
