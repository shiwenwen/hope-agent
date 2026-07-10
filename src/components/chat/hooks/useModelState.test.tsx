// @vitest-environment jsdom

import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { useModelState } from "./useModelState"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: vi.fn() },
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

function callsFor(command: string) {
  return transportMock.call.mock.calls.filter(([calledCommand]) => calledCommand === command)
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

describe("useModelState", () => {
  beforeEach(() => {
    transportMock.call.mockReset()
    transportMock.call.mockResolvedValue(undefined)
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  test("updates the draft UI and local global model before persistence completes", async () => {
    const persistence = deferred<void>()
    transportMock.call.mockReturnValueOnce(persistence.promise)
    const { result } = renderHook(() => useModelState())
    let modelChange = Promise.resolve()

    act(() => {
      modelChange = result.current.handleModelChange("provider-a::model-a")
    })

    expect(callsFor("set_active_model")).toEqual([
      ["set_active_model", { providerId: "provider-a", modelId: "model-a" }],
    ])
    expect(result.current.activeModel).toEqual({
      providerId: "provider-a",
      modelId: "model-a",
    })
    expect(result.current.globalActiveModelRef.current).toEqual({
      providerId: "provider-a",
      modelId: "model-a",
    })

    await act(async () => {
      persistence.resolve()
      await modelChange
    })
  })

  test("persists an existing session selection globally and pins that session exactly once", async () => {
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange(
        "provider-b::model-b",
        "session-1",
        "agent-1",
      )
    })

    expect(callsFor("set_active_model")).toEqual([
      ["set_active_model", { providerId: "provider-b", modelId: "model-b" }],
    ])
    expect(callsFor("set_session_model")).toEqual([
      [
        "set_session_model",
        {
          sessionId: "session-1",
          providerId: "provider-b",
          modelId: "model-b",
        },
      ],
    ])
  })

  test("still pins the session and retains the UI selection when global persistence fails", async () => {
    transportMock.call.mockImplementation((command: string) => {
      if (command === "set_active_model") {
        return Promise.reject(new Error("global persistence failed"))
      }
      return Promise.resolve(undefined)
    })
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange("provider-c::model-c", "session-2")
    })

    expect(callsFor("set_active_model")).toHaveLength(1)
    expect(callsFor("set_session_model")).toEqual([
      [
        "set_session_model",
        {
          sessionId: "session-2",
          providerId: "provider-c",
          modelId: "model-c",
        },
      ],
    ])
    expect(result.current.activeModel).toEqual({
      providerId: "provider-c",
      modelId: "model-c",
    })
    expect(result.current.globalActiveModelRef.current).toEqual({
      providerId: "provider-c",
      modelId: "model-c",
    })
  })

  test("does not pin a session when no session id exists", async () => {
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange("provider-d::model-d", null)
    })

    expect(callsFor("set_active_model")).toHaveLength(1)
    expect(callsFor("set_session_model")).toHaveLength(0)
  })
})
