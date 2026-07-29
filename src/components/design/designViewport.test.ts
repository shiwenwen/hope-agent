import { describe, expect, it } from "vitest"
import {
  CUSTOM_VIEWPORT_LIMITS,
  DEVICE_VIEWPORT_PRESETS,
  clampCustomViewport,
  findDeviceViewportPreset,
  isPreviewDevice,
  offsetAnchoredCustomViewport,
  resizeCustomViewport,
} from "@/components/design/designViewport"

describe("design viewport presets", () => {
  it("keeps the legacy ids and exposes mainstream phone and tablet choices", () => {
    expect(findDeviceViewportPreset("desktop")?.width).toBe(1440)
    expect(findDeviceViewportPreset("tablet")).toMatchObject({ width: 820, height: 1180 })
    expect(findDeviceViewportPreset("mobile")).toMatchObject({ width: 390, height: 844 })
    expect(findDeviceViewportPreset("ipad-mini")).toMatchObject({ width: 744, height: 1133 })
    expect(findDeviceViewportPreset("ipad-pro-11")).toMatchObject({ width: 834, height: 1210 })
    expect(findDeviceViewportPreset("ipad-pro-13")).toMatchObject({ width: 1032, height: 1376 })
    expect(findDeviceViewportPreset("surface-pro-9")).toMatchObject({
      width: 960,
      height: 1440,
    })
    expect(
      DEVICE_VIEWPORT_PRESETS.filter((preset) => preset.group === "tablet").length,
    ).toBeGreaterThan(3)
    expect(
      DEVICE_VIEWPORT_PRESETS.filter((preset) => preset.group === "mobile").length,
    ).toBeGreaterThan(4)
    expect(isPreviewDevice("custom")).toBe(true)
    expect(isPreviewDevice("unknown-device")).toBe(false)
  })

  it("clamps invalid and out-of-range custom dimensions", () => {
    expect(clampCustomViewport({ width: 10, height: 9999 })).toEqual({
      width: CUSTOM_VIEWPORT_LIMITS.minWidth,
      height: CUSTOM_VIEWPORT_LIMITS.maxHeight,
    })
    expect(clampCustomViewport({ width: Number.NaN, height: Number.POSITIVE_INFINITY })).toEqual({
      width: 390,
      height: 844,
    })
  })

  it("converts rendered drag distance back to logical pixels", () => {
    expect(resizeCustomViewport({ width: 390, height: 844 }, 40, 60, 0.5, "both")).toEqual({
      width: 470,
      height: 964,
    })
    expect(resizeCustomViewport({ width: 390, height: 844 }, 40, 60, 0.5, "width")).toEqual({
      width: 470,
      height: 844,
    })
    expect(resizeCustomViewport({ width: 390, height: 844 }, 40, 60, 0.5, "height")).toEqual({
      width: 390,
      height: 964,
    })
  })

  it("offsets a centered frame so its left and top edges stay anchored", () => {
    const start = { width: 390, height: 844 }
    const next = resizeCustomViewport(start, 40, 60, 0.5, "both")
    expect(offsetAnchoredCustomViewport(start, next, 0.5, "both", { x: 0, y: 0 })).toEqual({
      x: 20,
      y: 30,
    })
    expect(offsetAnchoredCustomViewport(start, next, 0.5, "width", { x: 8, y: 12 })).toEqual({
      x: 28,
      y: 12,
    })
  })
})
