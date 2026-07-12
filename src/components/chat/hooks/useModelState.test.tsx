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

describe("useModelState", () => {
  beforeEach(() => {
    transportMock.call.mockReset()
    transportMock.call.mockResolvedValue(undefined)
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  test("updates a draft locally without mutating global or Agent defaults", async () => {
    const { result } = renderHook(() => useModelState())

    await act(async () => result.current.handleModelChange("provider-a::model-a"))

    expect(callsFor("set_active_model")).toHaveLength(0)
    expect(callsFor("patch_agent_model_defaults")).toHaveLength(0)
    expect(result.current.activeModel).toEqual({
      providerId: "provider-a",
      modelId: "model-a",
    })
  })

  test("pins only the selected existing session", async () => {
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange(
        "provider-b::model-b",
        "session-1",
        "agent-1",
      )
    })

    expect(callsFor("set_active_model")).toHaveLength(0)
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

  test("optionally patches the Agent primary without changing global", async () => {
    transportMock.call.mockImplementation(() => Promise.resolve(undefined))
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange("provider-c::model-c", "session-2", "agent-2", {
        applyToAgentDefault: true,
      })
    })

    expect(callsFor("set_active_model")).toHaveLength(0)
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
    expect(callsFor("patch_agent_model_defaults")).toEqual([
      [
        "patch_agent_model_defaults",
        {
          id: "agent-2",
          patch: { primaryModel: { providerId: "provider-c", modelId: "model-c" } },
        },
      ],
    ])
    expect(result.current.activeModel).toEqual({
      providerId: "provider-c",
      modelId: "model-c",
    })
  })

  test("does not pin a session when no session id exists", async () => {
    const { result } = renderHook(() => useModelState())

    await act(async () => {
      await result.current.handleModelChange("provider-d::model-d", null)
    })

    expect(callsFor("set_active_model")).toHaveLength(0)
    expect(callsFor("set_session_model")).toHaveLength(0)
  })
})
