// @vitest-environment jsdom

import type { ComponentProps } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { WorkspaceEnvironmentState } from "./useWorkspaceEnvironment"
import type { WorkspaceEnvironmentSnapshot } from "@/lib/transport"
import type { BackgroundJobSnapshot } from "@/types/background-jobs"
import type { Message } from "@/types/chat"
import WorkspacePanel from "./WorkspacePanel"

const envMock = vi.hoisted(() => ({
  state: {
    snapshot: null,
    loading: false,
    error: null,
  } as WorkspaceEnvironmentState,
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn((name: string) => Promise.resolve(name === "get_background_job" ? null : [])),
  listen: vi.fn(() => () => {}),
}))

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallback?: string, values?: Record<string, unknown>) => {
      let text = typeof fallback === "string" ? fallback : key
      if (values) {
        for (const [k, v] of Object.entries(values)) {
          text = text.replace(`{{${k}}}`, String(v))
        }
      }
      return text
    },
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    supportsLocalFileOps: () => true,
    // KnowledgeSection (useSessionKnowledge) fetches attachments + subscribes to
    // knowledge:changed — stub both so the panel mounts in tests.
    call: transportMock.call,
    listen: transportMock.listen,
  }),
}))

vi.mock("@/hooks/useDangerousModeStatus", () => ({
  useDangerousModeStatus: () => ({ active: false, cliFlag: false, configFlag: false }),
}))

vi.mock("./useWorkspaceEnvironment", () => ({
  useWorkspaceEnvironment: () => envMock.state,
}))

vi.mock("./useWorkspaceArtifacts", () => ({
  useWorkspaceArtifacts: () => ({
    files: [],
    sources: [],
    browser: [],
    filesTruncated: false,
    sourcesTruncated: false,
    browserTruncated: false,
  }),
}))

afterEach(() => {
  cleanup()
  envMock.state = { snapshot: null, loading: false, error: null }
  transportMock.call.mockReset()
  transportMock.call.mockImplementation((name: string) =>
    Promise.resolve(name === "get_background_job" ? null : []),
  )
  transportMock.listen.mockClear()
})

function backgroundJob(patch: Partial<BackgroundJobSnapshot> = {}): BackgroundJobSnapshot {
  return {
    jobId: "job-1",
    kind: "tool",
    status: "running",
    tool: "exec",
    label: "cargo test",
    origin: "chat",
    sessionId: "s1",
    createdAt: 1,
    completedAt: null,
    error: null,
    resultPreview: null,
    resultPath: null,
    childCount: null,
    childrenTerminal: null,
    childrenCompleted: null,
    childrenFailed: null,
    subagentRunId: null,
    outputTail: "running output",
    ...patch,
  }
}

function renderPanel(
  snapshot: WorkspaceEnvironmentSnapshot | null,
  props: Partial<ComponentProps<typeof WorkspacePanel>> = {},
) {
  envMock.state = { snapshot, loading: false, error: null }
  return render(
    <TooltipProvider>
      <WorkspacePanel
        taskSnapshot={null}
        messages={[]}
        onOpenDiff={() => {}}
        onClose={() => {}}
        sessionId="s1"
        sessionMeta={{
          id: "s1",
          agentId: "ha-main",
          createdAt: "2026-01-01T00:00:00Z",
          updatedAt: "2026-01-01T00:00:00Z",
          messageCount: 0,
          unreadCount: 0,
          channelUnreadCount: 0,
          hasError: false,
          pendingInteractionCount: 0,
          isCron: false,
          incognito: false,
          channelInfo: {
            channelId: "telegram",
            accountId: "acc",
            chatId: "chat-1",
            chatType: "dm",
            senderName: "Ada",
          },
        }}
        project={{
          id: "p1",
          name: "my-project",
          createdAt: 0,
          updatedAt: 0,
          sortOrder: 0,
          archived: false,
          sessionCount: 1,
          unreadCount: 0,
          memoryCount: 0,
        }}
        effectiveWorkingDir={snapshot?.workingDir.path ?? null}
        workingDirSource="project"
        permissionMode="default"
        planState="review"
        activeModel={{ providerId: "openai", modelId: "gpt-test" }}
        {...props}
      />
    </TooltipProvider>,
  )
}

describe("WorkspacePanel environment section", () => {
  it("renders the no-working-dir state", () => {
    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    expect(screen.getByText("环境")).toBeTruthy()
    expect(screen.getByText("无工作目录")).toBeTruthy()
    expect(screen.getAllByText("未设置").length).toBeGreaterThan(0)
  })

  it("renders project, channel, branch, and dirty git status", () => {
    renderPanel({
      workingDir: { path: "/repo", source: "project", exists: true, name: "repo" },
      git: {
        root: "/repo",
        branch: "main",
        detached: false,
        head: "abc123",
        worktrees: [{ path: "/repo", branch: "main", isCurrent: true }],
        status: {
          changedFiles: 2,
          stagedFiles: 1,
          unstagedFiles: 1,
          untrackedFiles: 0,
          conflictedFiles: 0,
          linesAdded: 12,
          linesRemoved: 3,
          clean: false,
        },
        sync: {
          upstream: "origin/main",
          remote: "https://example.com/repo.git",
          ahead: 0,
          behind: 0,
          state: "upToDate",
        },
        lastCommit: { hash: "abc123", subject: "Add workspace env" },
      },
    })

    expect(screen.getByText("有变更")).toBeTruthy()
    expect(screen.getByText("my-project")).toBeTruthy()
    expect(screen.getByText("telegram")).toBeTruthy()
    expect(screen.getByText("main")).toBeTruthy()
    expect(screen.getByText("2 个文件")).toBeTruthy()
    expect(screen.getByText("Add workspace env")).toBeTruthy()
  })

  it("reuses expandable background job controls in the workspace section", () => {
    const onBackgroundJobExpandedChange = vi.fn()
    renderPanel(null, {
      backgroundJobs: [backgroundJob()],
      onBackgroundJobExpandedChange,
    })

    expect(screen.getByText("running output")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "收起任务" }))

    expect(onBackgroundJobExpandedChange).toHaveBeenCalledWith("job-1", false)
  })

  it("renders cross-turn memory diagnostics in the workspace section", () => {
    const messages: Message[] = [
      {
        role: "assistant",
        content: "带记忆的回答",
        usedMemoryRefs: [
          {
            kind: "memory",
            id: "m1",
            origin: "active_memory",
            role: "selected",
            preview: "private preference",
          },
        ],
        retrievalPlanner: {
          status: "partial",
          totalRefs: 1,
          layers: [
            {
              layer: "active_memory",
              status: "used",
              refCount: 1,
              selectedCount: 1,
            },
            {
              layer: "knowledge",
              status: "skipped",
              refCount: 0,
              skippedReason: "side_query_error",
            },
          ],
        },
      },
    ]

    renderPanel(null, { messages })

    expect(screen.getByText("记忆诊断")).toBeTruthy()
    expect(screen.getByText("Partially degraded")).toBeTruthy()
    expect(screen.getByText("来源对比")).toBeTruthy()
    expect(screen.getByText("最近轮次")).toBeTruthy()
    expect(screen.getAllByText("Active recall").length).toBeGreaterThan(0)
    expect(screen.getByText("#1")).toBeTruthy()
    expect(screen.getByText(/recall failed/)).toBeTruthy()
  })

  it("shows knowledge attachment load failures instead of the empty attached state", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_session_kbs_cmd") {
        return Promise.reject(
          new Error(
            "Authorization: Bearer bearer-secret token=query-secret api_key=sk-live-secret",
          ),
        )
      }
      return Promise.resolve(name === "get_background_job" ? null : [])
    })

    renderPanel(null)

    expect(await screen.findByText("无法读取已挂载知识空间")).toBeTruthy()
    expect(await screen.findByText(/Authorization: Bearer \[redacted\]/)).toBeTruthy()
    expect(screen.queryByText("未挂载知识空间")).toBeNull()
    expect(screen.queryByText(/bearer-secret|query-secret|sk-live-secret/)).toBeNull()
  })
})
