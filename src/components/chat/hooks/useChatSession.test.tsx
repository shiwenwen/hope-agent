// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"
import type { AvailableModel, ChatRuntimeDefaults } from "@/types/chat"
import { DEFAULT_AGENT_ID } from "@/types/tools"
import { useChatSession } from "./useChatSession"

const mocks = vi.hoisted(() => ({
  transport: { call: vi.fn(), listen: vi.fn(() => vi.fn()) },
  t: (key: string) => key,
}))

vi.mock("@/lib/transport-provider", () => ({ getTransport: () => mocks.transport }))
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn(), warn: vi.fn() } }))
vi.mock("@/lib/notifications", () => ({ notify: vi.fn() }))
vi.mock("react-i18next", async (importOriginal) => ({
  ...(await importOriginal<typeof import("react-i18next")>()),
  useTranslation: () => ({ t: mocks.t }),
}))

const enabledModel = { providerId: "enabled-provider", modelId: "enabled-model" }
const disabledModel = { providerId: "disabled-provider", modelId: "disabled-model" }
const runtimeDefaults: ChatRuntimeDefaults = {
  model: enabledModel,
  preferredModel: null,
  preferredModelAvailable: true,
  temperature: null,
  reasoningEffort: "medium",
}

function setup() {
  const setActiveModel = vi.fn()
  const applyModelForDisplay = vi.fn()
  const options = {
    availableModels: [
      {
        ...enabledModel,
        providerName: "Enabled provider",
        apiType: "openai_chat",
        modelName: "Enabled model",
        inputTypes: ["text"],
        contextWindow: 128_000,
        maxTokens: 8_192,
        reasoning: false,
      } satisfies AvailableModel,
    ],
    setActiveModel,
    applyModelForDisplay,
    globalActiveModelRef: { current: disabledModel },
    activeSessionReadable: false,
    activeSessionReadableRef: { current: false },
  }
  return { ...renderHook(() => useChatSession(options)), setActiveModel, applyModelForDisplay }
}

describe("new chat model selection", () => {
  beforeEach(() => {
    mocks.transport.call.mockReset()
    mocks.transport.call.mockImplementation(async (command: string) => {
      if (command === "list_sessions_cmd") return [[], 0]
      if (command === "regular_unread_total_cmd") return 0
      if (command === "list_agents") return [{ id: DEFAULT_AGENT_ID, name: "Main" }]
      if (command === "get_default_agent_id") return DEFAULT_AGENT_ID
      if (command === "get_agent_config") return { model: { primary: null } }
      if (command === "get_chat_runtime_defaults") return runtimeDefaults
      return null
    })
  })

  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  test("repeated new chats use the resolved enabled model instead of the disabled global default", async () => {
    const { result, setActiveModel, applyModelForDisplay } = setup()
    await waitFor(() => expect(result.current.agents).toHaveLength(1))

    for (let i = 0; i < 2; i++) {
      await act(async () => result.current.handleNewChat(DEFAULT_AGENT_ID))
    }

    expect(applyModelForDisplay.mock.calls).toEqual([
      ["enabled-provider::enabled-model"],
      ["enabled-provider::enabled-model"],
    ])
    expect(setActiveModel).not.toHaveBeenCalledWith(disabledModel)
    expect(mocks.transport.call).toHaveBeenCalledWith("get_chat_runtime_defaults", {
      agentId: DEFAULT_AGENT_ID,
    })
  })

  test("clears the previous selection when no model is available", async () => {
    const { result, setActiveModel, applyModelForDisplay } = setup()
    await waitFor(() => expect(result.current.agents).toHaveLength(1))
    const original = mocks.transport.call.getMockImplementation()!
    mocks.transport.call.mockImplementation(async (command: string) =>
      command === "get_chat_runtime_defaults"
        ? { ...runtimeDefaults, model: null }
        : original(command),
    )

    await act(async () => result.current.handleNewChat(DEFAULT_AGENT_ID))

    expect(setActiveModel).toHaveBeenLastCalledWith(null)
    expect(applyModelForDisplay).not.toHaveBeenCalled()
  })

  test("ignores a delayed model result from a previous new chat", async () => {
    const { result, applyModelForDisplay } = setup()
    await waitFor(() => expect(result.current.agents).toHaveLength(1))
    let resolvePrevious!: (defaults: ChatRuntimeDefaults) => void
    const previous = new Promise<ChatRuntimeDefaults>((resolve) => {
      resolvePrevious = resolve
    })
    const original = mocks.transport.call.getMockImplementation()!
    mocks.transport.call.mockImplementation(
      async (command: string, args?: { agentId?: string }) => {
        if (command === "get_chat_runtime_defaults" && args?.agentId === "previous-agent") {
          return previous
        }
        return original(command)
      },
    )

    let previousChat!: Promise<void>
    await act(async () => {
      previousChat = result.current.handleNewChat("previous-agent")
    })
    await act(async () => result.current.handleNewChat(DEFAULT_AGENT_ID))
    await act(async () => {
      resolvePrevious({ ...runtimeDefaults, model: disabledModel })
      await previousChat
    })

    expect(applyModelForDisplay.mock.calls).toEqual([["enabled-provider::enabled-model"]])
  })
})
