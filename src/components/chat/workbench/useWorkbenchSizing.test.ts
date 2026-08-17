// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react"
import { beforeEach, describe, expect, it } from "vitest"
import {
  nextWorkbenchLayoutMode,
  resolveWorkbenchLayout,
  useWorkbenchSizing,
  CHAT_IDEAL_MIN,
  WORKBENCH_IDEAL_MIN,
  WORKBENCH_MIN,
} from "./useWorkbenchSizing"

const storageValues = new Map<string, string>()
const memoryStorage: Storage = {
  get length() {
    return storageValues.size
  },
  clear: () => storageValues.clear(),
  getItem: (key) => storageValues.get(key) ?? null,
  key: (index) => [...storageValues.keys()][index] ?? null,
  removeItem: (key) => storageValues.delete(key),
  setItem: (key, value) => storageValues.set(key, String(value)),
}

Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: memoryStorage,
})

function setViewport(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width })
}

const CARD_LANE = 348

/** Both column widths for an even auto split. */
function split(available: number, chatIdeal = CHAT_IDEAL_MIN) {
  const { width, collapse } = resolveWorkbenchLayout({ available, ratio: 0.5, chatIdeal })
  return { workbench: width, chat: available - width, collapse }
}

describe("workbench sizing", () => {
  beforeEach(() => {
    window.localStorage.clear()
    setViewport(1024)
  })

  it("shrinks both columns together while each is above its ideal minimum", () => {
    expect(split(1600)).toMatchObject({ workbench: 800, chat: 800 })
    expect(split(1200)).toMatchObject({ workbench: 600, chat: 600 })
    // Exactly at both ideals — the last width where nothing has had to give.
    expect(split(CHAT_IDEAL_MIN + WORKBENCH_IDEAL_MIN)).toMatchObject({
      workbench: WORKBENCH_IDEAL_MIN,
      chat: CHAT_IDEAL_MIN,
    })
  })

  it("makes the workbench give once the chat is at its ideal minimum", () => {
    expect(split(1000)).toMatchObject({ workbench: 440, chat: CHAT_IDEAL_MIN })
    expect(split(CHAT_IDEAL_MIN + WORKBENCH_MIN)).toMatchObject({
      workbench: WORKBENCH_MIN,
      chat: CHAT_IDEAL_MIN,
    })
  })

  it("asks to collapse once even a minimum workbench no longer fits", () => {
    expect(split(CHAT_IDEAL_MIN + WORKBENCH_MIN).collapse).toBe(false)
    expect(split(CHAT_IDEAL_MIN + WORKBENCH_MIN - 1).collapse).toBe(true)
  })

  it("keeps a dragged split proportional instead of pinning its pixel width", () => {
    const wide = resolveWorkbenchLayout({ available: 1600, ratio: 0.65, chatIdeal: CHAT_IDEAL_MIN })
    const narrow = resolveWorkbenchLayout({
      available: 1200,
      ratio: 0.65,
      chatIdeal: CHAT_IDEAL_MIN,
    })
    expect(wide.width).toBe(1040)
    expect(narrow.width).toBeLessThan(wide.width)
    expect(1200 - narrow.width).toBeGreaterThanOrEqual(CHAT_IDEAL_MIN)
  })

  it("hands the card's lane to the chat, and takes it back when it stops fitting", () => {
    setViewport(1600)
    const withCard = renderHook(() => useWorkbenchSizing(true, CARD_LANE, true))
    expect(withCard.result.current.cardFits).toBe(true)
    expect(1600 - withCard.result.current.width).toBe(CHAT_IDEAL_MIN + CARD_LANE)

    // One pixel under "both ideals plus the lane" the card is the first to go,
    // which hands its space straight back to both columns.
    setViewport(CHAT_IDEAL_MIN + CARD_LANE + WORKBENCH_IDEAL_MIN - 1)
    const dropped = renderHook(() => useWorkbenchSizing(true, CARD_LANE, true))
    expect(dropped.result.current.cardFits).toBe(false)
    expect(dropped.result.current.width).toBeGreaterThan(withCard.result.current.width)
  })

  it("uses hysteresis when moving between docked and stage layouts", () => {
    expect(nextWorkbenchLayoutMode("docked", true, 779)).toBe("stage")
    expect(nextWorkbenchLayoutMode("stage", true, 840)).toBe("stage")
    expect(nextWorkbenchLayoutMode("stage", true, 860)).toBe("docked")
    expect(nextWorkbenchLayoutMode("stage", false, 600)).toBe("docked")
  })

  it("persists a dragged split as a ratio, and only once committed", () => {
    setViewport(1600)
    const { result } = renderHook(() => useWorkbenchSizing(true))

    act(() => result.current.setManualWidth(800))
    expect(window.localStorage.getItem("hope.chat.workbench.widthMode")).toBeNull()

    act(() => result.current.commitManualWidth(800))
    expect(window.localStorage.getItem("hope.chat.workbench.widthMode")).toBe("manual")
    expect(window.localStorage.getItem("hope.chat.workbench.manualRatio")).toBe("0.5000")

    // A narrower window re-derives from that ratio rather than replaying 800px.
    setViewport(1200)
    const narrow = renderHook(() => useWorkbenchSizing(true))
    expect(narrow.result.current.width).toBe(600)
  })
})
