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
  onContinue: undefined as (() => unknown) | undefined,
  autonomyPaused: undefined as boolean | undefined,
  workingDir: undefined as string | null | undefined,
  pendingQuestionGroup: undefined as unknown,
  onQuestionSubmitted: undefined as (() => unknown) | undefined,
  streamOptions: undefined as Record<string, unknown> | undefined,
}))

const askUserCapture = vi.hoisted(() => ({
  currentSessionId: undefined as string | null | undefined,
  pendingQuestionGroup: {
    requestId: "request-side-1",
    sessionId: "side-1",
    questions: [],
  },
  setPendingQuestionGroup: vi.fn(),
}))

vi.mock("@/components/chat/ChatInput", () => ({
  default: (props: {
    onCommandAction?: (result: unknown) => unknown
    onContinue?: () => unknown
    autonomyPaused?: boolean
    workingDir?: string | null
  }) => {
    componentCapture.onCommandAction = props.onCommandAction
    componentCapture.onContinue = props.onContinue
    componentCapture.autonomyPaused = props.autonomyPaused
    componentCapture.workingDir = props.workingDir
    return <div data-testid="chat-input" />
  },
}))

vi.mock("@/components/chat/MessageList", () => ({
  default: (props: {
    onSwitchModel?: (providerId: string, modelId: string) => unknown
    pendingQuestionGroup?: unknown
    onQuestionSubmitted?: () => unknown
  }) => {
    componentCapture.onSwitchModel = props.onSwitchModel
    componentCapture.pendingQuestionGroup = props.pendingQuestionGroup
    componentCapture.onQuestionSubmitted = props.onQuestionSubmitted
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

vi.mock("./ask-user/useAskUserPending", () => ({
  useAskUserPending: (currentSessionId: string | null) => {
    askUserCapture.currentSessionId = currentSessionId
    return {
      pendingQuestionGroup: askUserCapture.pendingQuestionGroup,
      setPendingQuestionGroup: askUserCapture.setPendingQuestionGroup,
    }
  },
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
  sessions: [{ id: "side-1", autonomyPaused: false }],
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
  stopPendingSessions: new Set<string>(),
  handleContinue: vi.fn(),
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
  useChatStream: (options: Record<string, unknown>) => {
    componentCapture.streamOptions = options
    return streamShape
  },
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  componentCapture.onCommandAction = undefined
  componentCapture.onSwitchModel = undefined
  componentCapture.onContinue = undefined
  componentCapture.autonomyPaused = undefined
  componentCapture.workingDir = undefined
  componentCapture.pendingQuestionGroup = undefined
  componentCapture.onQuestionSubmitted = undefined
  componentCapture.streamOptions = undefined
  askUserCapture.currentSessionId = undefined
  sessionShape.sessions = [{ id: "side-1", autonomyPaused: false }]
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

  test("wires durable Continue and the effective workspace into the side composer", async () => {
    sessionShape.sessions = [{ id: "side-1", autonomyPaused: true }]

    render(
      <SideChatPanel
        sessionId="side-1"
        workingDir="/project/inherited-root"
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
      />,
    )

    expect(componentCapture.autonomyPaused).toBe(true)
    expect(componentCapture.workingDir).toBe("/project/inherited-root")
    expect(componentCapture.streamOptions?.mentionWorkingDir).toBe("/project/inherited-root")
    expect(componentCapture.streamOptions?.parentInjectionDeltasViaChatStream).toBe(true)

    await act(async () => {
      await componentCapture.onContinue?.()
    })
    expect(streamShape.handleContinue).toHaveBeenCalledTimes(1)
  })

  test("renders and clears structured questions for the side session", () => {
    render(
      <SideChatPanel
        sessionId="side-1"
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
      />,
    )

    expect(askUserCapture.currentSessionId).toBe("side-1")
    expect(componentCapture.pendingQuestionGroup).toBe(askUserCapture.pendingQuestionGroup)

    componentCapture.onQuestionSubmitted?.()
    expect(askUserCapture.setPendingQuestionGroup).toHaveBeenCalledWith(null)
  })
})
