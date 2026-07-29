export type DeviceGroup = "desktop" | "tablet" | "mobile"

export interface DeviceViewportPreset {
  id: string
  group: DeviceGroup
  label: string
  width: number
  height: number | null
}

/**
 * CSS viewport sizes (logical pixels), deliberately independent from DPR.
 * Keep the legacy desktop/tablet/mobile ids so existing per-artifact choices
 * in localStorage continue to resolve after the richer picker ships.
 */
export const DEVICE_VIEWPORT_PRESETS = [
  { id: "desktop", group: "desktop", label: "Desktop", width: 1440, height: null },
  { id: "tablet", group: "tablet", label: "iPad Air", width: 820, height: 1180 },
  { id: "ipad-mini", group: "tablet", label: 'iPad Mini 8.3"', width: 744, height: 1133 },
  { id: "ipad-pro-11", group: "tablet", label: 'iPad Pro 11"', width: 834, height: 1210 },
  { id: "ipad-pro-13", group: "tablet", label: 'iPad Pro 13"', width: 1032, height: 1376 },
  { id: "surface-pro-9", group: "tablet", label: "Surface Pro 9", width: 960, height: 1440 },
  { id: "mobile", group: "mobile", label: "iPhone 14", width: 390, height: 844 },
  { id: "iphone-se", group: "mobile", label: "iPhone SE", width: 375, height: 667 },
  { id: "iphone-15-pro", group: "mobile", label: "iPhone 15 Pro", width: 393, height: 852 },
  {
    id: "iphone-15-pro-max",
    group: "mobile",
    label: "iPhone 15 Pro Max",
    width: 430,
    height: 932,
  },
  { id: "pixel-7", group: "mobile", label: "Pixel 7", width: 412, height: 915 },
  { id: "galaxy-s23", group: "mobile", label: "Galaxy S23", width: 360, height: 780 },
] as const satisfies readonly DeviceViewportPreset[]

export type DeviceViewportPresetId = (typeof DEVICE_VIEWPORT_PRESETS)[number]["id"]
export type PreviewDevice = "auto" | "custom" | DeviceViewportPresetId

export interface ViewportSize {
  width: number
  height: number
}

export interface ViewportOffset {
  x: number
  y: number
}

export type ViewportResizeAxis = "width" | "height" | "both"

export const DEFAULT_CUSTOM_VIEWPORT: ViewportSize = { width: 390, height: 844 }
export const CUSTOM_VIEWPORT_LIMITS = {
  minWidth: 240,
  maxWidth: 2560,
  minHeight: 320,
  maxHeight: 2560,
} as const

export function findDeviceViewportPreset(value: string): DeviceViewportPreset | null {
  return DEVICE_VIEWPORT_PRESETS.find((preset) => preset.id === value) ?? null
}

export function isPreviewDevice(value: string | null): value is PreviewDevice {
  return value === "auto" || value === "custom" || findDeviceViewportPreset(value ?? "") != null
}

export function clampCustomViewport(size: ViewportSize): ViewportSize {
  const finiteWidth = Number.isFinite(size.width)
    ? Math.round(size.width)
    : DEFAULT_CUSTOM_VIEWPORT.width
  const finiteHeight = Number.isFinite(size.height)
    ? Math.round(size.height)
    : DEFAULT_CUSTOM_VIEWPORT.height
  return {
    width: Math.min(
      CUSTOM_VIEWPORT_LIMITS.maxWidth,
      Math.max(CUSTOM_VIEWPORT_LIMITS.minWidth, finiteWidth),
    ),
    height: Math.min(
      CUSTOM_VIEWPORT_LIMITS.maxHeight,
      Math.max(CUSTOM_VIEWPORT_LIMITS.minHeight, finiteHeight),
    ),
  }
}

/** Convert pointer movement in rendered pixels back to logical viewport pixels. */
export function resizeCustomViewport(
  start: ViewportSize,
  deltaX: number,
  deltaY: number,
  renderedScale: number,
  axis: ViewportResizeAxis,
): ViewportSize {
  const safeScale = Number.isFinite(renderedScale) && renderedScale > 0 ? renderedScale : 1
  return clampCustomViewport({
    width: axis === "height" ? start.width : start.width + deltaX / safeScale,
    height: axis === "width" ? start.height : start.height + deltaY / safeScale,
  })
}

/**
 * The custom frame is centered by its flex parent. Moving its center by half of
 * the rendered size delta keeps the opposite (left/top) edge fixed while the
 * right/bottom resize handles follow the pointer one-for-one.
 */
export function offsetAnchoredCustomViewport(
  startSize: ViewportSize,
  nextSize: ViewportSize,
  renderedScale: number,
  axis: ViewportResizeAxis,
  startOffset: ViewportOffset,
): ViewportOffset {
  const safeScale = Number.isFinite(renderedScale) && renderedScale > 0 ? renderedScale : 1
  return {
    x:
      startOffset.x +
      (axis === "height" ? 0 : ((nextSize.width - startSize.width) * safeScale) / 2),
    y:
      startOffset.y +
      (axis === "width" ? 0 : ((nextSize.height - startSize.height) * safeScale) / 2),
  }
}
