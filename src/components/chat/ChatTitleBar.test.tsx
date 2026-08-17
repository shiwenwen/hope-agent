// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { Eye, FolderOpen, Globe, Layers, LayoutDashboard } from "lucide-react"

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
        "chat.rightPanel.expand": "Expand workbench",
        "chat.rightPanel.collapse": "Collapse workbench",
        "chat.controlPanel.floatWindow": "Float window",
        "common.close": "Close",
        "workspace.panelTitle": "Workspace",
        "fileBrowser.panelTitle": "Files",
        "backgroundJobs.panelTitle": "Background Tasks",
        "filePreview.panelTitle": "Preview",
        "fileBrowser.maximize": "Maximize",
        "fileBrowser.minimize": "Restore",
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

function titleBar(props: Partial<React.ComponentProps<typeof ChatTitleBar>> = {}) {
  return (
    <ChatTitleBar
      agentName="Hope"
      currentAgentId="ha-main"
      currentSessionId="s1"
      sessions={props.sessions ?? [sessionMeta()]}
      messages={[]}
      activeModel={null}
      availableModels={[]}
      reasoningEffort="medium"
      loading={false}
      compacting={false}
      {...props}
    />
  )
}

function renderTitleBar(props: Partial<React.ComponentProps<typeof ChatTitleBar>> = {}) {
  return render(titleBar(props))
}

function statusToggle(): HTMLElement {
  return screen.getByRole("button", { name: "chat.sessionStatus" })
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  transportMock.call.mockImplementation(() => Promise.resolve("full"))
  transportMock.listen.mockImplementation(() => () => {})
})

describe("ChatTitleBar workbench", () => {
  test("renders open tabs in the single title row and dispatches their ids", () => {
    const onSelectWorkbenchTab = vi.fn()
    const tabs = [
      {
        id: "workspace",
        panelId: "workspace" as const,
        labelKey: "workspace.panelTitle",
        icon: LayoutDashboard,
        open: true,
      },
      {
        id: "background-jobs",
        panelId: "background-jobs" as const,
        labelKey: "backgroundJobs.panelTitle",
        icon: Layers,
        open: true,
      },
    ]
    renderTitleBar({
      workbenchWidth: 720,
      workbenchTabs: tabs,
      workbenchLaunchItems: tabs,
      activeWorkbenchTabId: "workspace",
      onSelectWorkbenchTab,
      onCollapseWorkbench: vi.fn(),
    })

    expect(screen.getByRole("tablist", { name: "Right panel dock" })).toBeTruthy()
    fireEvent.click(screen.getByRole("tab", { name: /Background Tasks/ }))
    expect(onSelectWorkbenchTab).toHaveBeenCalledWith("background-jobs")
  })

  test("uses a neutral selected state and localizes the badge label", () => {
    renderTitleBar({
      workbenchWidth: 720,
      activeWorkbenchTabId: "workspace",
      onCollapseWorkbench: vi.fn(),
      workbenchTabs: [
        {
          id: "workspace",
          panelId: "workspace",
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

    const workspaceTab = screen.getByRole("tab", { name: /Workspace/ })
    expect(workspaceTab.className.split(" ")).toContain("text-foreground")
    expect(workspaceTab.getAttribute("aria-selected")).toBe("true")
    expect(screen.getByLabelText("1 workflows need attention")).toBeTruthy()
  })

  test("keeps multiple file preview tabs in the same workbench", () => {
    const onSelectWorkbenchTab = vi.fn()
    renderTitleBar({
      workbenchWidth: 720,
      activeWorkbenchTabId: "workspace",
      onSelectWorkbenchTab,
      onCollapseWorkbench: vi.fn(),
      workbenchTabs: [
        {
          id: "workspace",
          panelId: "workspace",
          labelKey: "workspace.panelTitle",
          icon: LayoutDashboard,
          open: true,
        },
        {
          id: "preview:readme",
          panelId: "preview",
          label: "README.md",
          icon: Eye,
          open: true,
        },
      ],
    })

    fireEvent.click(screen.getByRole("tab", { name: /README.md/ }))
    expect(onSelectWorkbenchTab).toHaveBeenCalledWith("preview:readme")
  })

  test("keeps a workbench reopen entry while the surface is collapsed", () => {
    const onExpandWorkbench = vi.fn()
    renderTitleBar({
      workbenchCollapsed: true,
      workbenchTabs: [
        {
          id: "files",
          panelId: "files",
          labelKey: "fileBrowser.panelTitle",
          icon: FolderOpen,
          open: true,
        },
      ],
      workbenchLaunchItems: [
        {
          id: "files",
          panelId: "files",
          labelKey: "fileBrowser.panelTitle",
          icon: FolderOpen,
          open: true,
        },
      ],
      onExpandWorkbench,
    })

    fireEvent.click(screen.getByRole("button", { name: "Expand workbench" }))
    expect(onExpandWorkbench).toHaveBeenCalledOnce()
  })

  test("owns maximize and floating-window controls at the workbench level", () => {
    const onToggleWorkbenchTabWindow = vi.fn()
    const onToggleWorkbenchMaximize = vi.fn()
    renderTitleBar({
      workbenchWidth: 720,
      activeWorkbenchTabId: "browser",
      onCollapseWorkbench: vi.fn(),
      onToggleWorkbenchTabWindow,
      onToggleWorkbenchMaximize,
      workbenchTabs: [
        {
          id: "browser",
          panelId: "browser",
          label: "Browser",
          icon: Globe,
          open: true,
          windowMode: "docked",
        },
      ],
    })

    fireEvent.click(screen.getByRole("button", { name: "Float window" }))
    expect(onToggleWorkbenchTabWindow).toHaveBeenCalledWith("browser")

    fireEvent.click(screen.getByRole("button", { name: "Maximize" }))
    expect(onToggleWorkbenchMaximize).toHaveBeenCalledOnce()
  })

  test("pins the session-status card until its own button is clicked again", () => {
    renderTitleBar({})

    expect(statusToggle().getAttribute("aria-pressed")).toBe("false")
    fireEvent.click(statusToggle())
    expect(statusToggle().getAttribute("aria-pressed")).toBe("true")

    fireEvent.mouseDown(document.body)
    expect(statusToggle().getAttribute("aria-pressed")).toBe("true")

    fireEvent.click(statusToggle())
    expect(statusToggle().getAttribute("aria-pressed")).toBe("false")
  })

  test("re-shows a still-pinned status card once the layout has room again", () => {
    const { rerender } = renderTitleBar({})
    fireEvent.click(statusToggle())

    rerender(titleBar({ suppressStatus: true }))
    expect(statusToggle().getAttribute("aria-pressed")).toBe("false")

    rerender(titleBar({ suppressStatus: false }))
    expect(statusToggle().getAttribute("aria-pressed")).toBe("true")
  })

  test("never clips the title row, so its drop-down surfaces stay visible", () => {
    const { container } = renderTitleBar({})

    const row = container.firstElementChild as HTMLElement
    expect(row.className).toContain("h-10")
    expect(row.className.split(" ")).not.toContain("overflow-hidden")
    const wrapper = row.firstElementChild as HTMLElement
    expect(wrapper.className.split(" ")).not.toContain("overflow-hidden")
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
