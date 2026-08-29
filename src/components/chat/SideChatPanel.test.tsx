// @vitest-environment jsdom

import { act, cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import SideChatPanel from "./SideChatPanel"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

const componentCapture = vi.hoisted(() => ({
  onCommandAction: undefined as ((result: unknown) => unknown) | undefined,
  onSwitchModel: undefined as ((providerId: string, modelId: string) => unknown) | undefined,
}))

vi.mock("@/components/chat/ChatInput", () => ({
  default: (props: { onCommandAction?: (result: unknown) => unknown }) => {
    componentCapture.onCommandAction = props.onCommandAction
    return <div data-testid="chat-input" />
  },
}))

vi.mock("@/components/chat/MessageList", () => ({
  default: (props: { onSwitchModel?: (providerId: string, modelId: string) => unknown }) => {
    componentCapture.onSwitchModel = props.onSwitchModel
    return <div data-testid="message-list" />
  },
}))

vi.mock("@/components/chat/ApprovalDialog", () => ({
  default: () => null,
}))

vi.mock("./hooks/useEmbeddedChatReadReceipt", () => ({
  useEmbeddedChatReadReceipt: vi.fn(),
}))

vi.mock("./hooks/useChatStreamReattach", () => ({
  useChatStreamReattach: vi.fn(),
}))

const sessionShape = {
  messages: [],
  setMessages: vi.fn(),
  currentSessionId: "side-1",
  setCurrentSessionId: vi.fn(),
  currentSessionIdRef: { current: "side-1" },
  currentAgentId: "main",
  agentName: "Main",
  agents: [],
  loading: false,
  setLoading: vi.fn(),
  loadingSessionIds: new Set<string>(),
  setLoadingSessionIds: vi.fn(),
  loadingSessionsRef: { current: new Set<string>() },
  sessionCacheRef: { current: new Map() },
  sessions: [],
  hasMore: false,
  loadingMore: false,
  handleLoadMore: vi.fn(),
  availableModels: [],
  activeModel: null,
  unavailableModelPreference: null,
  reasoningEffort: "medium",
  sessionTemperature: null,
  handleModelChange: vi.fn(),
  handleEffortChange: vi.fn(),
  resetEffort: vi.fn(),
  handleTemperatureChange: vi.fn(),
  reloadSessions: vi.fn(),
  reloadMessages: vi.fn(),
  updateSessionMessages: vi.fn(),
}

vi.mock("./useQuickChatSession", () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  useQuickChatSession: () => sessionShape as any,
}))

const streamShape = {
  input: "",
  setInput: vi.fn(),
  typedMentions: [],
  setInputWithMention: vi.fn(),
  handleSend: vi.fn(),
  attachedFiles: [],
  setAttachedFiles: vi.fn(),
  maxChatAttachmentBytes: 1024,
  pendingMessageQuotes: [],
  setPendingMessageQuotes: vi.fn(),
  pendingMessage: null,
  setPendingMessage: vi.fn(),
  pendingSends: [],
  editPendingSend: vi.fn(),
  discardPendingSend: vi.fn(),
  sendPendingSend: vi.fn(),
  forceInsertPendingSend: vi.fn(),
  cancelForceInsertPendingSend: vi.fn(),
  handleStop: vi.fn(),
  approvalRequests: [],
  handleApprovalResponse: vi.fn(),
  permissionMode: "default",
  setPermissionModeByUser: vi.fn(),
  sandboxMode: "off",
  setSandboxModeByUser: vi.fn(),
  showCodexAuthExpired: false,
  setShowCodexAuthExpired: vi.fn(),
  handleTurnStarted: vi.fn(),
  handleTurnEnded: vi.fn(),
}

vi.mock("./useChatStream", () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  useChatStream: () => streamShape as any,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  componentCapture.onCommandAction = undefined
  componentCapture.onSwitchModel = undefined
})

describe("SideChatPanel slash actions", () => {
  test("renders the no-argument /model picker and lets it switch models", async () => {
    render(
      <SideChatPanel
        sessionId="side-1"
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
      />,
    )

    await act(async () => {
      await componentCapture.onCommandAction?.({
        content: "",
        action: {
          type: "showModelPicker",
          models: [
            {
              providerId: "provider-1",
              providerName: "Provider",
              modelId: "model-1",
              modelName: "Model",
            },
          ],
          activeProviderId: "provider-1",
          activeModelId: "model-old",
        },
      })
    })

    const appendPicker = sessionShape.setMessages.mock.calls.at(-1)?.[0]
    expect(appendPicker).toBeTypeOf("function")
    expect(appendPicker([])).toMatchObject([
      {
        role: "event",
        content: "",
        modelPickerData: {
          models: [
            {
              providerId: "provider-1",
              modelId: "model-1",
            },
          ],
          activeProviderId: "provider-1",
          activeModelId: "model-old",
        },
      },
    ])
    expect(sessionShape.reloadMessages).not.toHaveBeenCalled()

    await act(async () => {
      await componentCapture.onSwitchModel?.("provider-1", "model-1")
    })
    expect(sessionShape.handleModelChange).toHaveBeenCalledWith("provider-1::model-1")
  })
})
