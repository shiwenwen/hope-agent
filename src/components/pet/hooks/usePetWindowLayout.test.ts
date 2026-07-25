import { describe, expect, test } from "vitest"
import { chooseOverlayPlacement, selectOverlayMeasurement } from "./usePetWindowLayout"

describe("chooseOverlayPlacement", () => {
  test("keeps the current orientation when both sides fit", () => {
    expect(
      chooseOverlayPlacement(
        380,
        560,
        { left: 900, right: 900, above: 900, below: 900 },
        { horizontal: "right", vertical: "below" },
      ),
    ).toEqual({ horizontal: "right", vertical: "below" })
  })

  test.each([
    ["top-left", { left: 60, right: 900, above: 116, below: 900 }, "right", "below"],
    ["top-right", { left: 900, right: 60, above: 116, below: 900 }, "left", "below"],
    ["bottom-left", { left: 60, right: 900, above: 900, below: 12 }, "right", "above"],
    ["bottom-right", { left: 900, right: 60, above: 900, below: 12 }, "left", "above"],
  ] as const)("opens inward at the %s corner", (_label, space, horizontal, vertical) => {
    expect(
      chooseOverlayPlacement(380, 560, space, { horizontal: "left", vertical: "above" }),
    ).toEqual({ horizontal, vertical })
  })

  test("chooses the smaller total overflow when neither orientation fits", () => {
    expect(
      chooseOverlayPlacement(
        400,
        640,
        { left: 150, right: 210, above: 260, below: 180 },
        { horizontal: "left", vertical: "above" },
      ),
    ).toEqual({ horizontal: "right", vertical: "below" })
  })
})

describe("selectOverlayMeasurement", () => {
  const hidden = { width: 420, height: 180 }
  const live = { width: 420, height: 260 }

  test("uses the hidden twin before the overlay is committed", () => {
    expect(selectOverlayMeasurement(hidden, live, false)).toEqual(hidden)
  })

  test("uses the visible card after commit so Pet pagination can resize the window", () => {
    expect(selectOverlayMeasurement(hidden, live, true)).toEqual(live)
  })

  test("falls back to the hidden twin until the visible observer reports", () => {
    expect(selectOverlayMeasurement(hidden, null, true)).toEqual(hidden)
  })
})
