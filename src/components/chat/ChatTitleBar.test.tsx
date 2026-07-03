// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"

import ChatTitleBar from "./ChatTitleBar"
import type { SessionMeta } from "@/types/chat"

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => (typeof fallback === "string" ? fallback : key),
  }),
}))

vi.mock("@/lib/appMeta", () => ({
  useAppVersion: () => "0.0.0-test",
}))

vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
}))

vi.mock("@/components/chat/export/ExportSessionDialog", () => ({
  ExportSessionDialog: () => null,
}))

vi.mock("./AgentSwitcher", () => ({
  default: ({ agentName }: { agentName: string }) => <div>{agentName}</div>,
}))

function sessionMeta(patch: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id: "s1",
    title: "New Chat",
    agentId: "ha-main",
    createdAt: "2026-07-01T00:00:00.000Z",
    updatedAt: "2026-07-01T00:00:00.000Z",
    messageCount: 0,
    unreadCount: 0,
    channelUnreadCount: 0,
    pendingInteractionCount: 0,
    hasError: false,
    isCron: false,
    incognito: false,
    ...patch,
  }
}

function renderTitleBar(props: Partial<React.ComponentProps<typeof ChatTitleBar>> = {}) {
  const sessions = props.sessions ?? [sessionMeta()]
  return render(
    <ChatTitleBar
      agentName="Hope"
      currentAgentId="ha-main"
      currentSessionId="s1"
      sessions={sessions}
      messages={[]}
      activeModel={null}
      availableModels={[]}
      reasoningEffort="medium"
      loading={false}
      compacting={false}
      {...props}
    />,
  )
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("ChatTitleBar working directory affordances", () => {
  test("shows a visible workflow workspace entry in the title bar", () => {
    const onToggleWorkspacePanel = vi.fn()
    renderTitleBar({
      onToggleWorkspacePanel,
    })

    expect(screen.getByText("Workflow")).toBeTruthy()

    const codingButton = screen.getByRole("button", { name: "Open Workflow workspace" })
    fireEvent.click(codingButton)

    expect(onToggleWorkspacePanel).toHaveBeenCalledTimes(1)
  })

  test("badges the workflow entry when workflow runs need attention", () => {
    renderTitleBar({
      onToggleWorkspacePanel: vi.fn(),
      workspaceWorkflowStatus: {
        activeCount: 2,
        attentionCount: 1,
        runningCount: 1,
      },
    })

    expect(screen.getByRole("button", { name: /need attention/ })).toBeTruthy()
    expect(screen.getByText("1")).toBeTruthy()
  })

  test("shows file controls for an empty selected session with a working directory", () => {
    const onToggleFilesPanel = vi.fn()
    renderTitleBar({
      sessions: [sessionMeta({ workingDir: "/Users/me/repo" })],
      effectiveWorkingDir: "/Users/me/repo",
      workingDirSource: "session",
      onToggleFilesPanel,
    })

    expect(screen.getByText("repo")).toBeTruthy()

    const filesButton = screen.getByRole("button", { name: "Show files" })
    fireEvent.click(filesButton)

    expect(onToggleFilesPanel).toHaveBeenCalledTimes(1)
  })

  test("does not show file controls before a working directory exists", () => {
    renderTitleBar({
      sessions: [sessionMeta()],
      effectiveWorkingDir: null,
      onToggleFilesPanel: undefined,
    })

    expect(screen.queryByRole("button", { name: "Show files" })).toBeNull()
  })
})
