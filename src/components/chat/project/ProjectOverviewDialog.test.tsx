// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"

import ProjectOverviewDialog from "./ProjectOverviewDialog"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn((event: string, handler: (payload: unknown) => void) => {
    void event
    void handler
    return () => {}
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("./ProjectIcon", () => ({ default: () => <div data-testid="project-icon" /> }))
vi.mock("./ProjectInstructionsEditor", () => ({
  default: ({ readOnly }: { readOnly?: boolean }) => (
    <div>{readOnly ? "instructions-read-only" : "instructions-editable"}</div>
  ),
}))
vi.mock("./ProjectMemorySection", () => ({
  ProjectMemorySection: ({ readOnly }: { readOnly?: boolean }) => (
    <div>{readOnly ? "auto-memory-read-only" : "auto-memory-editable"}</div>
  ),
}))
vi.mock("./file-browser/FileBrowserView", () => ({
  FileBrowserView: ({ editable }: { editable?: boolean }) => (
    <div>{editable ? "file-browser-editable" : "file-browser-read-only"}</div>
  ),
}))

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    i18n: { language: "zh-CN" },
    t: (key: string, options?: Record<string, unknown>) => {
      const messages: Record<string, string> = {
        "project.overview.totalSessions": "项目会话",
        "project.overview.autoMemoryTopics": "自动记忆主题",
        "project.overview.activeClaims": "已整理记忆",
        "project.overview.agentsLines": "AGENTS.md",
        "project.overview.recentSessions": "最近会话",
        "project.overview.messageCount": `${options?.count ?? 0} 条消息`,
        "project.overview.agentsStats": `${options?.lines ?? 0} 行 · ${options?.size ?? ""}`,
        "project.overview.autoMemoryStats": `${options?.count ?? 0} 个主题`,
        "project.overview.unavailable": "暂不可用",
        "project.tabOverview": "概览",
        "project.tabFiles": "文件",
        "project.tabInstructions": "指令",
        "project.tabAutoMemory": "自动记忆",
        "project.newChatInProject": "新建会话",
      }
      return messages[key] ?? key
    },
  }),
}))

const project = {
  id: "project-1",
  name: "Hope Agent",
  description: "测试项目",
  createdAt: 1,
  updatedAt: 1,
  sortOrder: 0,
  archived: false,
  sessionCount: 99,
  unreadCount: 0,
}

const overview = {
  sessionCount: 1,
  autoMemoryTopicCount: 3,
  activeClaimCount: 4,
  instructions: {
    path: "/tmp/project/AGENTS.md",
    lineCount: 12,
    sizeBytes: 1024,
    empty: false,
  },
  recentSessions: [
    {
      id: "session-1",
      title: "继续实现概览",
      agentId: "ha-main",
      createdAt: "2026-07-15T08:00:00Z",
      updatedAt: "2026-07-15T09:00:00Z",
      messageCount: 8,
      unreadCount: 2,
      channelUnreadCount: 0,
      hasError: false,
      pendingInteractionCount: 1,
      isCron: false,
      incognito: false,
    },
  ],
}

function renderOverview(overrides: Record<string, unknown> = {}) {
  const props = {
    open: true,
    project,
    onOpenChange: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    onArchive: vi.fn(),
    onNewSessionInProject: vi.fn(),
    onOpenSession: vi.fn(),
    onOpenStructuredMemory: vi.fn(),
    ...overrides,
  }
  render(
    <TooltipProvider>
      <ProjectOverviewDialog {...props} />
    </TooltipProvider>,
  )
  return props
}

afterEach(() => {
  cleanup()
  transportMock.call.mockReset()
  transportMock.listen.mockClear()
})

describe("ProjectOverviewDialog", () => {
  it("renders actionable metrics and opens recent sessions", async () => {
    transportMock.call.mockResolvedValue(overview)
    const props = renderOverview()

    expect(await screen.findByText("继续实现概览")).toBeTruthy()
    expect(screen.getByText("自动记忆主题")).toBeTruthy()
    expect(screen.getByText("已整理记忆")).toBeTruthy()
    expect(screen.getByText("12")).toBeTruthy()

    fireEvent.click(screen.getByText("继续实现概览"))
    expect(props.onOpenSession).toHaveBeenCalledWith("session-1")
    expect(props.onOpenChange).toHaveBeenCalledWith(false)

    fireEvent.click(screen.getByText("已整理记忆"))
    expect(props.onOpenStructuredMemory).toHaveBeenCalledWith("project-1")
  })

  it("keeps other metrics usable when one source is unavailable", async () => {
    transportMock.call.mockResolvedValue({
      ...overview,
      autoMemoryTopicCount: null,
      instructions: null,
    })
    renderOverview()

    await waitFor(() => expect(screen.getAllByText("—").length).toBeGreaterThan(0))
    fireEvent.click(screen.getByText("自动记忆主题"))
    expect(await screen.findByText("auto-memory-editable")).toBeTruthy()
  })

  it("keeps archived project context read-only", async () => {
    transportMock.call.mockResolvedValue(overview)
    renderOverview({ project: { ...project, archived: true } })

    expect(await screen.findByText("继续实现概览")).toBeTruthy()
    expect(screen.queryByText("新建会话")).toBeNull()
    expect(screen.getByText("instructions-read-only")).toBeTruthy()

    fireEvent.mouseDown(screen.getByRole("tab", { name: "文件" }), { button: 0 })
    expect(await screen.findByText("file-browser-read-only")).toBeTruthy()

    fireEvent.mouseDown(screen.getByRole("tab", { name: "自动记忆" }), { button: 0 })
    expect(await screen.findByText("auto-memory-read-only")).toBeTruthy()
  })

  it("does not show unavailable states while the overview is loading", () => {
    transportMock.call.mockReturnValue(new Promise(() => {}))
    renderOverview()

    expect(screen.queryByText("暂不可用")).toBeNull()
  })

  it("filters project events and coalesces matching reloads", async () => {
    transportMock.call.mockResolvedValue(overview)
    renderOverview()
    expect(await screen.findByText("继续实现概览")).toBeTruthy()
    transportMock.call.mockClear()

    const fsListener = transportMock.listen.mock.calls.find(
      ([event]) => event === "project:fs_changed",
    )?.[1] as ((payload: unknown) => void) | undefined
    expect(fsListener).toBeTypeOf("function")

    fsListener?.({ scope: "project", scopeId: "project-2" })
    await new Promise((resolve) => window.setTimeout(resolve, 160))
    expect(transportMock.call).not.toHaveBeenCalled()

    fsListener?.({ scope: "project", scopeId: "project-1" })
    fsListener?.({ scope: "project", scopeId: "project-1" })
    await waitFor(() => expect(transportMock.call).toHaveBeenCalledTimes(1))
  })
})
