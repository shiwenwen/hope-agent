// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import type { ReactElement } from "react"

import { TooltipProvider } from "@/components/ui/tooltip"
import QuickChatDialog from "./QuickChatDialog"

function renderWithProviders(ui: ReactElement) {
  return render(<TooltipProvider>{ui}</TooltipProvider>)
}

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

// Heavy children — replaced with stubs so the only behavior under test is
// the dialog's "View full chat" button.
vi.mock("@/components/chat/MessageList", () => ({
  default: () => <div data-testid="message-list" />,
}))
vi.mock("@/components/chat/ApprovalDialog", () => ({
  default: () => null,
}))
vi.mock("@/components/chat/ChatInput", () => ({
  default: () => <div data-testid="chat-input" />,
}))

const sessionShape = {
  messages: [{ role: "user", content: "hi", timestamp: "2026-04-26T00:00:00.000Z" }],
  setMessages: vi.fn(),
  currentSessionId: "session-123",
  setCurrentSessionId: vi.fn(),
  currentSessionIdRef: { current: "session-123" },
  currentAgentId: "default",
  agentName: "Main",
  agents: [],
  loading: false,
  setLoading: vi.fn(),
  loadingSessionIds: new Set<string>(),
  setLoadingSessionIds: vi.fn(),
  sessionCacheRef: { current: new Map() },
  loadingSessionsRef: { current: new Set() },
  hasMore: false,
  loadingMore: false,
  handleLoadMore: vi.fn(),
  availableModels: [],
  activeModel: null,
  reasoningEffort: "medium",
  setReasoningEffort: vi.fn(),
  handleNewChat: vi.fn(),
  handleSwitchAgent: vi.fn(),
  handleModelChange: vi.fn(),
  handleEffortChange: vi.fn(),
  reloadSessions: vi.fn(),
  updateSessionMessages: vi.fn(),
  initSession: vi.fn(),
  sessions: [],
}

vi.mock("./useQuickChatSession", () => ({
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  useQuickChatSession: () => sessionShape as any,
}))

vi.mock("./useChatStream", () => ({
  useChatStream: () => ({
    input: "",
    setInput: vi.fn(),
    handleSend: vi.fn(),
    attachedFiles: [],
    setAttachedFiles: vi.fn(),
    pendingMessage: null,
    setPendingMessage: vi.fn(),
    handleStop: vi.fn(),
    approvalRequests: [],
    handleApprovalResponse: vi.fn(),
    permissionMode: "default",
    setPermissionMode: vi.fn(),
  }),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  // Reset session shape mutations between tests
  sessionShape.messages = [
    { role: "user", content: "hi", timestamp: "2026-04-26T00:00:00.000Z" },
  ]
  sessionShape.currentSessionId = "session-123"
})

describe("QuickChatDialog 'View full chat' button", () => {
  test("renders and triggers navigate + close when session has messages", () => {
    const onOpenChange = vi.fn()
    const onNavigateToSession = vi.fn()
    renderWithProviders(
      <QuickChatDialog
        open
        onOpenChange={onOpenChange}
        onNavigateToSession={onNavigateToSession}
      />,
    )

    const button = screen.getByLabelText("quickChat.viewFullChat")
    expect(button).toBeTruthy()

    fireEvent.click(button)
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(onNavigateToSession).toHaveBeenCalledWith("session-123")
  })

  test("hidden when no current session", () => {
    sessionShape.currentSessionId = null as unknown as string
    renderWithProviders(
      <QuickChatDialog
        open
        onOpenChange={vi.fn()}
        onNavigateToSession={vi.fn()}
      />,
    )
    expect(screen.queryByLabelText("quickChat.viewFullChat")).toBeNull()
  })

  test("hidden when messages are empty", () => {
    sessionShape.messages = []
    renderWithProviders(
      <QuickChatDialog
        open
        onOpenChange={vi.fn()}
        onNavigateToSession={vi.fn()}
      />,
    )
    expect(screen.queryByLabelText("quickChat.viewFullChat")).toBeNull()
  })

  test("hidden when onNavigateToSession is not provided", () => {
    renderWithProviders(<QuickChatDialog open onOpenChange={vi.fn()} />)
    expect(screen.queryByLabelText("quickChat.viewFullChat")).toBeNull()
  })
})
