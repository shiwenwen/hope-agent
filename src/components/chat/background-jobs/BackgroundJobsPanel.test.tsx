// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { BackgroundJobSnapshot } from "@/types/background-jobs"
import BackgroundJobsPanel from "./BackgroundJobsPanel"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => (typeof fallback === "string" ? fallback : key),
  }),
}))

vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
  Tooltip: ({ children }: { children: ReactNode }) => children,
  TooltipTrigger: ({ children }: { children: ReactNode }) => children,
  TooltipContent: ({ children }: { children: ReactNode }) => children,
  TooltipProvider: ({ children }: { children: ReactNode }) => children,
}))

vi.mock("@/components/ui/animated-presence", () => ({
  AnimatedCollapse: ({ open, children }: { open: boolean; children: ReactNode }) =>
    open ? <>{children}</> : null,
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: vi.fn().mockResolvedValue(null),
    listen: vi.fn(() => () => {}),
  }),
}))

vi.mock("./useLocalModelJobsMirror", () => ({
  useLocalModelJobsMirror: () => [],
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function job(patch: Partial<BackgroundJobSnapshot>): BackgroundJobSnapshot {
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
  jobs: BackgroundJobSnapshot[],
  props?: {
    overrides?: Record<string, boolean>
    onJobExpandedChange?: (jobId: string, expanded: boolean) => void
    integrated?: boolean
  },
) {
  return render(
    <BackgroundJobsPanel
      jobs={jobs}
      jobExpansionOverrides={props?.overrides}
      onJobExpandedChange={props?.onJobExpandedChange}
      onClose={() => {}}
      integrated={props?.integrated}
    />,
  )
}

describe("BackgroundJobsPanel", () => {
  test("renders active jobs expanded and reports manual collapse", () => {
    const onJobExpandedChange = vi.fn()
    renderPanel([job({ jobId: "job-1" })], { onJobExpandedChange })

    expect(screen.getByText("running output")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "收起任务" }))

    expect(onJobExpandedChange).toHaveBeenCalledWith("job-1", false)
  })

  test("honors a remembered collapsed override", () => {
    renderPanel([job({ jobId: "job-1" })], { overrides: { "job-1": false } })

    expect(screen.queryByText("running output")).toBeNull()
    expect(screen.getByRole("button", { name: "展开任务" })).toBeTruthy()
  })

  test("collapses completed jobs by default but keeps failed jobs expanded", () => {
    renderPanel([
      job({
        jobId: "done",
        status: "completed",
        outputTail: null,
        resultPreview: "success preview",
      }),
      job({
        jobId: "failed",
        status: "failed",
        outputTail: null,
        error: "failure details",
      }),
    ])

    expect(screen.queryByText("success preview")).toBeNull()
    expect(screen.getByText("failure details")).toBeTruthy()
  })

  test("offers no expand toggle when a row has nothing to expand", () => {
    renderPanel([
      // Subagent projections carry no content by design — no toggle, ever.
      job({
        jobId: "sub",
        kind: "subagent",
        status: "completed",
        tool: "subagent:researcher",
        label: "",
        outputTail: null,
        subagentRunId: "run-1",
      }),
      job({
        jobId: "done",
        status: "completed",
        outputTail: null,
        resultPreview: "success preview",
      }),
    ])

    // Only the tool job (collapsed by default) keeps a toggle.
    expect(screen.getAllByRole("button", { name: "展开任务" })).toHaveLength(1)
    expect(screen.queryByRole("button", { name: "收起任务" })).toBeNull()
  })

  test("does not repeat the frame title when hosted by workbench tabs", () => {
    renderPanel([], { integrated: true })

    expect(screen.queryByText("后台任务")).toBeNull()
    expect(screen.getByText("本会话")).toBeTruthy()
  })
})
