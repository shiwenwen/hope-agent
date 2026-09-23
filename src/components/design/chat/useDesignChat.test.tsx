// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import { logger } from "@/lib/logger"
import { useDesignChat } from "./useDesignChat"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => vi.fn()),
}))

vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transportMock }))
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn() } }))

describe("useDesignChat model routing", () => {
  beforeEach(() => {
    transportMock.call.mockReset()
    transportMock.listen.mockReset()
    transportMock.listen.mockImplementation(() => vi.fn())
    transportMock.call.mockImplementation((command: string) => {
      if (command === "list_agents") return Promise.resolve([])
      if (command === "get_available_models") {
        return Promise.resolve([{ providerId: "p", modelId: "chosen" }])
      }
      if (command === "get_chat_runtime_defaults") {
        return Promise.resolve({
          model: { providerId: "p", modelId: "chosen" },
          reasoningEffort: "medium",
        })
      }
      return Promise.resolve(undefined)
    })
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  test("pins a selection on an existing design session", async () => {
    const { result } = renderHook(() => useDesignChat(null, false))
    act(() => result.current.setCurrentSessionId("design-session"))

    await act(async () => {
      await result.current.handleModelChange("p::chosen")
    })

    expect(transportMock.call).toHaveBeenCalledWith("set_session_model", {
      sessionId: "design-session",
      providerId: "p",
      modelId: "chosen",
    })
    expect(result.current.activeModel).toEqual({ providerId: "p", modelId: "chosen" })
    expect(result.current.draftModelOverrideRef.current).toBeNull()
  })

  test("blocks sends while an existing session model is being saved", async () => {
    let finishSave!: () => void
    transportMock.call.mockImplementation((command: string) => {
      if (command === "set_session_model") {
        return new Promise<void>((resolve) => {
          finishSave = resolve
        })
      }
      return Promise.resolve(undefined)
    })
    const { result } = renderHook(() => useDesignChat(null, false))
    act(() => result.current.setCurrentSessionId("design-session"))

    let change!: Promise<void>
    act(() => {
      change = result.current.handleModelChange("p::chosen")
    })
    expect(result.current.modelSaving).toBe(true)
    expect(result.current.modelSavePendingRef.current).toBe(true)

    await act(async () => {
      finishSave()
      await change
    })
    expect(result.current.modelSaving).toBe(false)
    expect(result.current.modelSavePendingRef.current).toBe(false)
  })

  test("keeps the known project model before discovery completes", async () => {
    let finishDiscovery!: (models: { providerId: string; modelId: string }[]) => void
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_available_models") {
        return new Promise((resolve) => {
          finishDiscovery = resolve
        })
      }
      if (command === "get_chat_runtime_defaults") {
        return Promise.resolve({ model: null, reasoningEffort: "medium" })
      }
      return Promise.resolve([])
    })
    const selected = { providerId: "p", modelId: "chosen" }
    const { result } = renderHook(() => useDesignChat("project", true, selected))

    expect(result.current.draftModelOverrideRef.current).toEqual(selected)
    await act(async () => finishDiscovery([{ providerId: "p", modelId: "chosen" }]))
    expect(result.current.draftModelOverrideRef.current).toEqual(selected)
  })

  test("keeps the known project model when discovery fails", async () => {
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_available_models") return Promise.reject(new Error("offline"))
      if (command === "get_chat_runtime_defaults") {
        return Promise.resolve({ model: null, reasoningEffort: "medium" })
      }
      return Promise.resolve([])
    })
    const selected = { providerId: "p", modelId: "chosen" }
    const { result } = renderHook(() => useDesignChat("project", true, selected))

    await waitFor(() =>
      expect(logger.error).toHaveBeenCalledWith(
        "ui",
        "DesignChat::loadModels",
        "Failed to load models",
        expect.any(Error),
      ),
    )
    expect(result.current.draftModelOverrideRef.current).toEqual(selected)
  })

  test("passes the project model into a new design session", async () => {
    const selected = { providerId: "p", modelId: "chosen" }
    const { result } = renderHook(() => useDesignChat(null, true, selected))

    await waitFor(() => expect(result.current.availableModels).toHaveLength(1))

    expect(result.current.activeModel).toEqual(selected)
    expect(result.current.draftModelOverrideRef.current).toEqual(selected)
  })
})
