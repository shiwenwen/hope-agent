// @vitest-environment jsdom

import { describe, expect, test } from "vitest"

import { isInvertedClickPair, type PointerClickSample } from "./inverted-click-recovery"

const pointerUp: PointerClickSample = {
  button: 0,
  buttons: 0,
  clientX: 747.2,
  clientY: 353.4,
  detail: 0,
  isPrimary: true,
  pointerId: 1,
  pointerType: "mouse",
  timeStamp: 22868,
}

const pointerDown: PointerClickSample = {
  ...pointerUp,
  detail: 1,
}

describe("isInvertedClickPair", () => {
  test("matches the captured macOS WKWebView failure signature", () => {
    expect(isInvertedClickPair(pointerUp, pointerDown, 0.04)).toBe(true)
    expect(isInvertedClickPair({ ...pointerUp, detail: 1 }, pointerDown, 0.04)).toBe(true)
  })

  test("rejects ordinary, stale, moved and non-mouse sequences", () => {
    expect(isInvertedClickPair({ ...pointerUp, detail: 2 }, pointerDown, 0.04)).toBe(false)
    expect(isInvertedClickPair(pointerUp, pointerDown, 41)).toBe(false)
    expect(
      isInvertedClickPair(pointerUp, { ...pointerDown, clientX: pointerDown.clientX + 3 }, 0.04),
    ).toBe(false)
    expect(
      isInvertedClickPair(pointerUp, { ...pointerDown, pointerType: "touch" }, 0.04),
    ).toBe(false)
    expect(
      isInvertedClickPair(pointerUp, { ...pointerDown, timeStamp: pointerDown.timeStamp + 2 }, 0.04),
    ).toBe(false)
  })
})
