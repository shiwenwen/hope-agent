// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"

import type { Transport } from "@/lib/transport"
import { setTransport } from "@/lib/transport-provider"
import {
  listenEnhancedFocusIndicators,
  loadEnhancedFocusIndicators,
  saveEnhancedFocusIndicators,
} from "./focus-indicator-preference"

function mockTransport(initialEnabled: boolean) {
  const listeners = new Map<string, Set<(payload: unknown) => void>>()
  let enabled = initialEnabled

  const call = vi.fn((command: string, args?: Record<string, unknown>) => {
    if (command === "get_enhanced_focus_indicators") return Promise.resolve(enabled)
    if (command === "set_enhanced_focus_indicators") {
      enabled = Boolean(args?.enabled)
      return Promise.resolve(null)
    }
    return Promise.reject(new Error(`Unexpected command: ${command}`))
  })

  const transport = {
    call,
    listen: vi.fn((name: string, handler: (payload: unknown) => void) => {
      const handlers = listeners.get(name) ?? new Set()
      handlers.add(handler)
      listeners.set(name, handlers)
      return () => handlers.delete(handler)
    }),
  } as unknown as Transport

  return {
    call,
    emit(name: string, payload: unknown) {
      for (const handler of listeners.get(name) ?? []) handler(payload)
    },
    setStoredEnabled(next: boolean) {
      enabled = next
    },
    transport,
  }
}

afterEach(() => {
  vi.useRealTimers()
  delete document.documentElement.dataset.focusIndicators
})

describe("enhanced focus indicator preference", () => {
  test("loads and saves the shared preference", async () => {
    const mock = mockTransport(false)
    setTransport(mock.transport)

    await expect(loadEnhancedFocusIndicators()).resolves.toBe(false)
    expect(document.documentElement.dataset.focusIndicators).toBe("auto")

    await saveEnhancedFocusIndicators(true)
    expect(document.documentElement.dataset.focusIndicators).toBe("enhanced")
    expect(mock.call).toHaveBeenLastCalledWith("set_enhanced_focus_indicators", {
      enabled: true,
    })
  })

  test("reloads when another window emits the focus config category", async () => {
    const mock = mockTransport(false)
    setTransport(mock.transport)
    const onChange = vi.fn()
    const off = listenEnhancedFocusIndicators(onChange)

    mock.setStoredEnabled(true)
    mock.emit("config:changed", { category: "focus_indicator" })
    await vi.waitFor(() => expect(onChange).toHaveBeenCalledWith(true))

    expect(document.documentElement.dataset.focusIndicators).toBe("enhanced")
    off()
  })

  test("falls back instead of blocking rendering when the backend stalls", async () => {
    vi.useFakeTimers()
    setTransport({
      call: vi.fn(() => new Promise(() => {})),
    } as unknown as Transport)

    const loading = loadEnhancedFocusIndicators()
    await vi.runAllTimersAsync()

    await expect(loading).resolves.toBe(false)
    expect(document.documentElement.dataset.focusIndicators).toBe("auto")
  })
})
