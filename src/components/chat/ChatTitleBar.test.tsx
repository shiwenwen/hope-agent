// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { Eye, FolderOpen, Layers, LayoutDashboard } from "lucide-react"

import ChatTitleBar from "./ChatTitleBar"
import type { SessionMeta } from "@/types/chat"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(() => Promise.resolve("full")),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, options?: string | Record<string, unknown>) => {
      const translations: Record<string, string> = {
        "chat.rightPanel.dock": "Right panel dock",
        "chat.rightPanel.openPanel": "Open {{panel}}",
        "chat.rightPanel.switchToPanel": "Switch to {{panel}}",
        "chat.rightPanel.collapsePanel": "Collapse {{panel}}",
        "chat.rightPanel.expandPanel": "Expand {{panel}}",
        "chat.rightPanel.workflowAttentionCount": "{{count}} workflows need attention",
        "workspace.panelTitle": "Workspace",
        "fileBrowser.panelTitle": "Files",
        "backgroundJobs.panelTitle": "Background Tasks",
        "filePreview.panelTitle": "Preview",
      }
      const template = translations[key] ?? (typeof options === "string" ? options : key)
      if (!options || typeof options === "string") return template
      return template.replace(/{{(\w+)}}/g, (_, name: string) => String(options[name] ?? ""))
    },
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
  transportMock.call.mockImplementation(() => Promise.resolve("full"))
  transportMock.listen.mockImplementation(() => () => {})
})

describe("ChatTitleBar right-panel dock", () => {
  test("renders a single icon entry for each panel and dispatches its id", () => {
    const onRightPanelAction = vi.fn()
    renderTitleBar({
      onRightPanelAction,
      rightPanels: [
        {
          id: "workspace",
          labelKey: "workspace.panelTitle",
          icon: LayoutDashboard,
          open: false,
        },
        {
          id: "background-jobs",
          labelKey: "backgroundJobs.panelTitle",
          icon: Layers,
          open: false,
        },
      ],
    })

    expect(screen.getByRole("toolbar", { name: "Right panel dock" })).toBeTruthy()
    expect(screen.getAllByRole("button", { name: "Open Workspace" })).toHaveLength(1)
    fireEvent.click(screen.getByRole("button", { name: "Open Workspace" }))
    expect(onRightPanelAction).toHaveBeenCalledWith("workspace")
  })

  test("uses a neutral selected state and localizes the badge label", () => {
    renderTitleBar({
      activeRightPanelId: "workspace",
      rightPanels: [
        {
          id: "workspace",
          labelKey: "workspace.panelTitle",
          icon: LayoutDashboard,
          open: true,
          badge: {
            count: 1,
            labelKey: "chat.rightPanel.workflowAttentionCount",
            tone: "attention",
          },
        },
      ],
    })

    const workspaceButton = screen.getByRole("button", { name: "Collapse Workspace" })
    expect(workspaceButton.className.split(" ")).toContain("text-foreground")
    expect(workspaceButton.dataset.panelState).toBe("active")
    expect(screen.getByLabelText("1 workflows need attention")).toBeTruthy()
  })

  test("keeps open transient panels in the same dock", () => {
    const onRightPanelAction = vi.fn()
    renderTitleBar({
      activeRightPanelId: "workspace",
      onRightPanelAction,
      rightPanels: [
        {
          id: "workspace",
          labelKey: "workspace.panelTitle",
          icon: LayoutDashboard,
          open: true,
        },
        { id: "preview", labelKey: "filePreview.panelTitle", icon: Eye, open: true },
      ],
    })

    const previewButton = screen.getByRole("button", { name: "Switch to Preview" })
    fireEvent.click(previewButton)
    expect(onRightPanelAction).toHaveBeenCalledWith("preview")
  })

  test("labels the active icon as expand when the rail is collapsed", () => {
    renderTitleBar({
      activeRightPanelId: "files",
      rightPanelCollapsed: true,
      rightPanels: [
        { id: "files", labelKey: "fileBrowser.panelTitle", icon: FolderOpen, open: true },
      ],
    })

    expect(screen.getByRole("button", { name: "Expand Files" })).toBeTruthy()
  })

  test("still shows the localized working-directory chip", () => {
    renderTitleBar({
      sessions: [sessionMeta({ workingDir: "/Users/me/repo" })],
      effectiveWorkingDir: "/Users/me/repo",
      workingDirSource: "session",
    })

    expect(screen.getByText("repo")).toBeTruthy()
  })
})
