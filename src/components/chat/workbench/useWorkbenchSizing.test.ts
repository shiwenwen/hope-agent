// @vitest-environment jsdom

import { act, renderHook } from "@testing-library/react"
import { beforeEach, describe, expect, it } from "vitest"
import {
  automaticWorkbenchWidth,
  manualWorkbenchWidth,
  nextWorkbenchLayoutMode,
  useWorkbenchSizing,
  workbenchCollapseThreshold,
  CHAT_INITIAL_RESERVE,
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

describe("workbench sizing", () => {
  beforeEach(() => window.localStorage.clear())

  it("uses the available space while preserving the automatic chat reserve", () => {
    expect(automaticWorkbenchWidth(1920)).toBe(1280)
    expect(automaticWorkbenchWidth(1440)).toBe(880)
    expect(automaticWorkbenchWidth(980)).toBe(420)
  })

  it("allows a wider manual split but preserves the hard chat minimum", () => {
    expect(manualWorkbenchWidth(1440, 1200)).toBe(1080)
    expect(manualWorkbenchWidth(1440, 760)).toBe(760)
    expect(manualWorkbenchWidth(1440, 200)).toBe(420)
  })

  it("uses hysteresis when moving between docked and stage layouts", () => {
    expect(nextWorkbenchLayoutMode("docked", true, 779)).toBe("stage")
    expect(nextWorkbenchLayoutMode("stage", true, 840)).toBe("stage")
    expect(nextWorkbenchLayoutMode("stage", true, 860)).toBe("docked")
    expect(nextWorkbenchLayoutMode("stage", false, 600)).toBe("docked")
  })

  it("hands the environment card's lane to the chat reserve", () => {
    expect(automaticWorkbenchWidth(1600)).toBeGreaterThan(automaticWorkbenchWidth(1600, 908))
    expect(1600 - automaticWorkbenchWidth(1600, 908)).toBeGreaterThanOrEqual(908)
  })

  it("collapses only once both columns are past their ideal minimum", () => {
    expect(workbenchCollapseThreshold()).toBe(CHAT_INITIAL_RESERVE + WORKBENCH_MIN)
    expect(workbenchCollapseThreshold(908) - workbenchCollapseThreshold()).toBe(348)
  })

  it("persists a manual width only when the resize is committed", () => {
    const { result } = renderHook(() => useWorkbenchSizing(false))

    act(() => result.current.setManualWidth(600))
    expect(window.localStorage.getItem("hope.chat.workbench.widthMode")).toBeNull()
    expect(window.localStorage.getItem("hope.chat.workbench.manualWidth")).toBeNull()

    act(() => result.current.commitManualWidth(600))
    expect(window.localStorage.getItem("hope.chat.workbench.widthMode")).toBe("manual")
    expect(window.localStorage.getItem("hope.chat.workbench.manualWidth")).toBe("600")
  })
})
