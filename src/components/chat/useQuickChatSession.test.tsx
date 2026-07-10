// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import type { AvailableModel } from "@/types/chat"
import { useQuickChatSession } from "./useQuickChatSession"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => vi.fn()),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function callsFor(command: string) {
  return transportMock.call.mock.calls.filter(([calledCommand]) => calledCommand === command)
}

function createLocalStorage(): Storage {
  const values = new Map<string, string>()
  return {
    get length() {
      return values.size
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  }
}

describe("useQuickChatSession", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", createLocalStorage())
    transportMock.call.mockReset()
    transportMock.listen.mockReset()
    transportMock.listen.mockImplementation(() => vi.fn())
  })

  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    vi.clearAllMocks()
  })

  test("keeps the public session setter state and ref in sync immediately", () => {
    const { result } = renderHook(() => useQuickChatSession(false))

    act(() => {
      result.current.setCurrentSessionId("session-a")
      expect(result.current.currentSessionIdRef.current).toBe("session-a")
    })
    expect(result.current.currentSessionId).toBe("session-a")

    act(() => {
      result.current.setCurrentSessionId((previous) => `${previous}-next`)
      expect(result.current.currentSessionIdRef.current).toBe("session-a-next")
    })
    expect(result.current.currentSessionId).toBe("session-a-next")
  })

  test("does not pin the previous session while switching agents", async () => {
    const modelLoad = deferred<AvailableModel[]>()
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_available_models") return modelLoad.promise
      if (command === "get_active_model") return Promise.resolve(null)
      if (command === "get_current_settings") {
        return Promise.resolve({ reasoning_effort: "medium" })
      }
      if (command === "get_agent_config") {
        return Promise.resolve({ model: { primary: null, reasoningEffort: null } })
      }
      return Promise.resolve(undefined)
    })
    const { result } = renderHook(() => useQuickChatSession(false))

    act(() => {
      result.current.setCurrentSessionId("session-a")
    })
    expect(result.current.currentSessionIdRef.current).toBe("session-a")

    let switchAgent = Promise.resolve()
    await act(async () => {
      switchAgent = result.current.handleSwitchAgent("agent-b")
      await Promise.resolve()
    })

    await act(async () => {
      await result.current.handleModelChange("provider-b::model-b")
    })

    expect(callsFor("set_active_model")).toEqual([
      ["set_active_model", { providerId: "provider-b", modelId: "model-b" }],
    ])
    expect(callsFor("set_session_model")).toHaveLength(0)

    await act(async () => {
      modelLoad.resolve([])
      await switchAgent
    })
  })
})
