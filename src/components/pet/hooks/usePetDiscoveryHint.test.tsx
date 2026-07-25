// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import {
  PET_DISCOVERY_DELAY_MS,
  PET_DISCOVERY_STORAGE_KEY,
  usePetDiscoveryHint,
} from "./usePetDiscoveryHint"

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

beforeEach(() => {
  window.localStorage.clear()
  vi.useFakeTimers()
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe("usePetDiscoveryHint", () => {
  test("opens once Pet config is ready without stealing the initial render", () => {
    const { result, rerender } = renderHook(
      ({ ready }) => usePetDiscoveryHint({ supported: true, ready, enabled: false }),
      { initialProps: { ready: false } },
    )

    expect(result.current.open).toBe(false)
    rerender({ ready: true })
    act(() => vi.advanceTimersByTime(PET_DISCOVERY_DELAY_MS))
    expect(result.current.open).toBe(true)
  })

  test("snoozes an outside dismissal for this mount without marking it discovered", () => {
    const { result, unmount } = renderHook(() =>
      usePetDiscoveryHint({ supported: true, ready: true, enabled: false }),
    )
    act(() => vi.advanceTimersByTime(PET_DISCOVERY_DELAY_MS))
    act(() => result.current.handleOpenChange(false))
    expect(result.current.open).toBe(false)
    expect(window.localStorage.getItem(PET_DISCOVERY_STORAGE_KEY)).toBeNull()

    unmount()
    const next = renderHook(() =>
      usePetDiscoveryHint({ supported: true, ready: true, enabled: false }),
    )
    act(() => vi.advanceTimersByTime(PET_DISCOVERY_DELAY_MS))
    expect(next.result.current.open).toBe(true)
  })

  test("never returns after explicit dismissal or prior use", () => {
    const dismissed = renderHook(() =>
      usePetDiscoveryHint({ supported: true, ready: true, enabled: false }),
    )
    act(() => dismissed.result.current.markDiscovered())
    expect(window.localStorage.getItem(PET_DISCOVERY_STORAGE_KEY)).toBe("seen")
    dismissed.unmount()

    window.localStorage.clear()
    const alreadyEnabled = renderHook(() =>
      usePetDiscoveryHint({ supported: true, ready: true, enabled: true }),
    )
    act(() => vi.advanceTimersByTime(0))
    expect(alreadyEnabled.result.current.open).toBe(false)
    expect(window.localStorage.getItem(PET_DISCOVERY_STORAGE_KEY)).toBe("seen")
  })
})
