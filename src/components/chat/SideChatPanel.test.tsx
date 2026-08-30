// @vitest-environment jsdom

import { StrictMode } from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import SideChatPanel from "./SideChatPanel"
import {
  resolveSideChatQuoteOwner,
  resolveSideChatSurfaceSessionId,
} from "./sideChatSurface"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

const componentCapture = vi.hoisted(() => ({
  onCommandAction: undefined as ((result: unknown) => unknown) | undefined,
  onSwitchModel: undefined as ((providerId: string, modelId: string) => unknown) | undefined,
  onContinue: undefined as (() => unknown) | undefined,
  onResume: undefined as ((message: string) => unknown) | undefined,
  autonomyPaused: undefined as boolean | undefined,
  workingDir: undefined as string | null | undefined,
  enableGoalAndPlanModes: undefined as boolean | undefined,
  enableWorkflowMode: undefined as boolean | undefined,
  pendingQuestionGroup: undefined as unknown,
  onQuestionSubmitted: undefined as (() => unknown) | undefined,
  onOpenDiff: undefined as ((metadata: unknown) => unknown) | undefined,
  onOpenSubagentRun: undefined as ((target: unknown) => unknown) | undefined,
  onViewChildSession: undefined as ((sessionId: string) => unknown) | undefined,
  subagentRunsSnapshot: undefined as unknown,
  pendingQuotes: undefined as unknown,
  onRemoveQuote: undefined as ((index: number) => unknown) | undefined,
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

const readReceiptMock = vi.hoisted(() => vi.fn())

vi.mock("@/components/chat/ChatInput", () => ({
  default: (props: {
    onCommandAction?: (result: unknown) => unknown
    onContinue?: () => unknown
    autonomyPaused?: boolean
    workingDir?: string | null
    enableGoalAndPlanModes?: boolean
    enableWorkflowMode?: boolean
    pendingQuotes?: unknown
    onRemoveQuote?: (index: number) => unknown
  }) => {
    componentCapture.onCommandAction = props.onCommandAction
    componentCapture.onContinue = props.onContinue
    componentCapture.autonomyPaused = props.autonomyPaused
    componentCapture.workingDir = props.workingDir
    componentCapture.enableGoalAndPlanModes = props.enableGoalAndPlanModes
    componentCapture.enableWorkflowMode = props.enableWorkflowMode
    componentCapture.pendingQuotes = props.pendingQuotes
    componentCapture.onRemoveQuote = props.onRemoveQuote
    return <div data-testid="chat-input" />
  },
}))

vi.mock("@/components/chat/MessageList", () => ({
  default: (props: {
    onResume?: (message: string) => unknown
    onSwitchModel?: (providerId: string, modelId: string) => unknown
    pendingQuestionGroup?: unknown
    onQuestionSubmitted?: () => unknown
    onOpenDiff?: (metadata: unknown) => unknown
    onOpenSubagentRun?: (target: unknown) => unknown
    onViewChildSession?: (sessionId: string) => unknown
    subagentRunsSnapshot?: unknown
  }) => {
    componentCapture.onResume = props.onResume
    componentCapture.onSwitchModel = props.onSwitchModel
    componentCapture.pendingQuestionGroup = props.pendingQuestionGroup
    componentCapture.onQuestionSubmitted = props.onQuestionSubmitted
    componentCapture.onOpenDiff = props.onOpenDiff
    componentCapture.onOpenSubagentRun = props.onOpenSubagentRun
    componentCapture.onViewChildSession = props.onViewChildSession
    componentCapture.subagentRunsSnapshot = props.subagentRunsSnapshot
    return <div data-testid="message-list" />
  },
}))

vi.mock("@/components/chat/ApprovalDialog", () => ({
  default: () => null,
}))

vi.mock("./hooks/useEmbeddedChatReadReceipt", () => ({
  useEmbeddedChatReadReceipt: readReceiptMock,
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
  pendingQuotes: [],
  setPendingQuotes: vi.fn(),
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
  componentCapture.onResume = undefined
  componentCapture.autonomyPaused = undefined
  componentCapture.workingDir = undefined
  componentCapture.enableGoalAndPlanModes = undefined
  componentCapture.enableWorkflowMode = undefined
  componentCapture.pendingQuestionGroup = undefined
  componentCapture.onQuestionSubmitted = undefined
  componentCapture.onOpenDiff = undefined
  componentCapture.onOpenSubagentRun = undefined
  componentCapture.onViewChildSession = undefined
  componentCapture.subagentRunsSnapshot = undefined
  componentCapture.pendingQuotes = undefined
  componentCapture.onRemoveQuote = undefined
  componentCapture.streamOptions = undefined
  askUserCapture.currentSessionId = undefined
  sessionShape.sessions = [{ id: "side-1", autonomyPaused: false }]
})

describe("SideChatPanel slash actions", () => {
  test("continues a round-limited side turn through its own send handler", async () => {
    const onActivity = vi.fn()
    render(
      <SideChatPanel
        sessionId="side-1"
        isViewVisible
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
        onActivity={onActivity}
      />,
    )
    expect(componentCapture.onResume).toBeTypeOf("function")
    await act(async () => {
      componentCapture.onResume?.("Continue the remaining work")
    })
    expect(streamShape.handleSend).toHaveBeenCalledWith("Continue the remaining work")
    expect(onActivity).toHaveBeenCalledOnce()
  })

  test("consumes a seeded prompt exactly once after StrictMode effect replay", async () => {
    const quote = {
      messageId: 42,
      role: "assistant" as const,
      content: "quoted answer",
    }

    render(
      <StrictMode>
        <SideChatPanel
          sessionId="side-1"
          isViewVisible
          seed={{ nonce: 1, prompt: "follow up", quote }}
          onClose={vi.fn()}
          onDeleted={vi.fn()}
          onPreviewFile={vi.fn()}
        />
      </StrictMode>,
    )

    await act(async () => {
      await Promise.resolve()
    })

    expect(streamShape.handleSend).toHaveBeenCalledOnce()
    expect(streamShape.handleSend).toHaveBeenCalledWith("follow up")
    expect(streamShape.setPendingMessageQuotes).toHaveBeenCalledOnce()
    const appendQuote = streamShape.setPendingMessageQuotes.mock.calls[0]?.[0]
    expect(appendQuote([])).toEqual([quote])
  })

  test("renders the no-argument /model picker and lets it switch models", async () => {
    render(
      <SideChatPanel
        sessionId="side-1"
        isViewVisible
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
        isViewVisible
        workingDir="/project/inherited-root"
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
      />,
    )

    expect(componentCapture.autonomyPaused).toBe(true)
    expect(componentCapture.workingDir).toBe("/project/inherited-root")
    expect(componentCapture.enableGoalAndPlanModes).toBe(false)
    expect(componentCapture.enableWorkflowMode).toBe(false)
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
        isViewVisible
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

  test("gates read receipts on the parent Conversations view", () => {
    const props = {
      sessionId: "side-1",
      onClose: vi.fn(),
      onDeleted: vi.fn(),
      onPreviewFile: vi.fn(),
    }
    const { rerender } = render(<SideChatPanel {...props} isViewVisible={false} />)

    expect(readReceiptMock).toHaveBeenLastCalledWith(false, true, "side-1", sessionShape.messages)

    rerender(<SideChatPanel {...props} isViewVisible />)
    expect(readReceiptMock).toHaveBeenLastCalledWith(true, true, "side-1", sessionShape.messages)
  })

  test("routes shared file quotes into the side composer", () => {
    const onFileQuoteHandlerChange = vi.fn()
    render(
      <SideChatPanel
        sessionId="side-1"
        isViewVisible
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
        onFileQuoteHandlerChange={onFileQuoteHandlerChange}
      />,
    )
    const handler = onFileQuoteHandlerChange.mock.calls.find(([value]) => value)?.[0]
    const quote = {
      path: "artifact:canvas-side",
      name: "Side canvas",
      startLine: 0,
      endLine: 0,
      content: "selected side content",
      revealable: false,
    }

    act(() => handler(quote))

    const appendQuote = streamShape.setPendingQuotes.mock.calls.at(-1)?.[0]
    expect(appendQuote([])).toEqual([quote])
    expect(componentCapture.pendingQuotes).toBe(streamShape.pendingQuotes)
    act(() => componentCapture.onRemoveQuote?.(0))
    const removeQuote = streamShape.setPendingQuotes.mock.calls.at(-1)?.[0]
    expect(removeQuote([quote])).toEqual([])
  })

  test("routes shared surfaces to the open side session only", () => {
    expect(resolveSideChatSurfaceSessionId("main-1", "side-1", true)).toBe("side-1")
    expect(resolveSideChatSurfaceSessionId("main-1", "side-1", false)).toBe("main-1")
    expect(resolveSideChatSurfaceSessionId("main-1", null, true)).toBe("main-1")
    expect(resolveSideChatQuoteOwner("main-1", "main-1", "side-1", true)).toBe("main")
    expect(resolveSideChatQuoteOwner("side-1", "main-1", "side-1", true)).toBe("side")
    expect(resolveSideChatQuoteOwner("side-1", "main-1", "side-1", false)).toBeNull()
  })

  test("exposes side tool diffs through the shared workbench callback", () => {
    const onOpenDiff = vi.fn()
    render(
      <SideChatPanel
        sessionId="side-1"
        isViewVisible
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
        onOpenDiff={onOpenDiff}
      />,
    )
    const diff = {
      kind: "file_change",
      path: "src/side.ts",
      action: "modify",
      additions: 2,
      deletions: 1,
      diff: "@@ -1 +1 @@",
    }

    act(() => componentCapture.onOpenDiff?.(diff))

    expect(onOpenDiff).toHaveBeenCalledWith(diff)
  })

  test("routes side subagent chips into the side-scoped shared panel", () => {
    const onOpenSubagentRun = vi.fn()
    const onViewChildSession = vi.fn()
    const subagentRunsSnapshot = {
      sessionId: "side-1",
      runs: [],
      byId: new Map(),
      byChildSessionId: new Map(),
      runningCount: 0,
      loaded: true,
      refetch: vi.fn(),
    }
    render(
      <SideChatPanel
        sessionId="side-1"
        isViewVisible
        onClose={vi.fn()}
        onDeleted={vi.fn()}
        onPreviewFile={vi.fn()}
        onOpenSubagentRun={onOpenSubagentRun}
        onViewChildSession={onViewChildSession}
        subagentRunsSnapshot={subagentRunsSnapshot}
      />,
    )
    const target = { runId: "run-side-1", childSessionId: "child-side-1" }

    act(() => componentCapture.onOpenSubagentRun?.(target))
    act(() => componentCapture.onViewChildSession?.("child-side-1"))

    expect(onOpenSubagentRun).toHaveBeenCalledWith(target)
    expect(onViewChildSession).toHaveBeenCalledWith("child-side-1")
    expect(componentCapture.subagentRunsSnapshot).toBe(subagentRunsSnapshot)
  })
})
