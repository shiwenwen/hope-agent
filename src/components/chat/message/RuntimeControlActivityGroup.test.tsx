// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import {
  SubagentRunsContext,
  type SubagentRunsView,
} from "@/components/chat/subagent/useSubagentRuns"
import type { SubagentRun, ToolCall } from "@/types/chat"

import RuntimeControlActivityGroup from "./RuntimeControlActivityGroup"

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      if (key.startsWith("executionStatus.runtimeControl.action.agent.close_")) {
        return `close ${values?.count} ${values?.count === 1 ? "agent" : "agents"}`
      }
      if (key === "executionStatus.runtimeControl.action.agent.closeAll") {
        return "close all agents"
      }
      if (key.startsWith("executionStatus.runtimeControl.action.process.close_")) {
        return `close ${values?.count} ${values?.count === 1 ? "process" : "processes"}`
      }
      if (key.endsWith(".action.team.resume_one")) return "resume 1 team"
      if (key.endsWith(".action.team.resume_other")) return `resume ${values?.count} teams`
      if (key === "executionStatus.runtimeControl.state.accepted") {
        return `requested ${values?.action}`
      }
      if (key === "executionStatus.runtimeControl.state.completed") {
        return `completed ${values?.action}`
      }
      if (key === "executionStatus.runtimeControl.state.failed") {
        return `failed ${values?.action}`
      }
      if (key === "executionStatus.runtimeControl.state.partial") {
        return `partial ${values?.action}`
      }
      if (key === "executionStatus.runtimeControl.state.noAgentsToClose") {
        return "no agents need closing"
      }
      if (key === "executionStatus.runtimeControl.state.noResumeNeeded") {
        return "team already complete · no resume needed"
      }
      if (key.startsWith("executionStatus.runtimeControl.badge.refused_")) {
        return `${values?.count} refused`
      }
      if (key.startsWith("executionStatus.runtimeControl.aggregate.")) {
        const kind = key
          .split(".")
          .at(-1)
          ?.replace(/_(one|other)$/, "")
        return `${kind} ${values?.count}`
      }
      if (key === "executionStatus.runtimeControl.failureDetails") return "Resume failures"
      if (key === "executionStatus.tool.group.failedCount") return `${values?.count} failed`
      if (key === "tools.elapsed") return String(values?.time)
      return key
    },
  }),
}))

vi.mock("./ToolCallBlock", () => ({
  default: ({ tool, labelOverride }: { tool: ToolCall; labelOverride: string }) => (
    <div data-testid="runtime-detail" data-label={labelOverride}>
      {tool.callId}:{tool.result}
    </div>
  ),
}))

vi.mock("./ToolMediaPreview", () => ({ default: () => null }))

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

function killTool(extra: Partial<ToolCall> = {}): ToolCall {
  return {
    callId: "kill-1",
    name: "subagent",
    arguments: JSON.stringify({ action: "kill", run_id: "r1" }),
    result: "signal accepted",
    ...extra,
  }
}

function teamResumeTool(callId: string, result: Record<string, unknown>): ToolCall {
  return {
    callId,
    name: "team",
    arguments: JSON.stringify({ action: "resume", team_id: "team-1" }),
    result: JSON.stringify(result),
  }
}

function run(status: SubagentRun["status"]): SubagentRun {
  return {
    runId: "r1",
    threadId: "thread-1",
    parentSessionId: "parent",
    parentAgentId: "ha-main",
    childAgentId: "researcher",
    childSessionId: "child-1",
    task: "research",
    status,
    depth: 1,
    startedAt: "2026-08-05T00:00:00.000Z",
    triggerKind: "spawn",
    leaseEpoch: 1,
    deliveryKind: "parent",
    ownerKind: "parent_session",
    ownerId: "parent",
  }
}

function runsView(rows: SubagentRun[]): SubagentRunsView {
  return {
    sessionId: "parent",
    runs: rows,
    byId: new Map(rows.map((row) => [row.runId, row])),
    byChildSessionId: new Map(rows.map((row) => [row.childSessionId, row])),
    runningCount: rows.filter((row) => row.status === "running").length,
    loaded: true,
    refetch: vi.fn(),
  }
}

describe("RuntimeControlActivityGroup", () => {
  test("shows an accepted close request without a live terminal state", () => {
    render(<RuntimeControlActivityGroup tools={[killTool()]} />)

    screen.getByRole("button", { name: /requested close 1 agent\b/i })
  })

  test("uses the plural action label for multiple targets", () => {
    render(
      <RuntimeControlActivityGroup
        tools={[
          killTool(),
          killTool({
            callId: "kill-2",
            arguments: JSON.stringify({ action: "kill", run_id: "r2" }),
          }),
        ]}
      />,
    )

    screen.getByRole("button", { name: /requested close 2 agents\b/i })
  })

  test("upgrades the same request after SubagentRunsContext confirms killed", () => {
    render(
      <SubagentRunsContext.Provider value={runsView([run("killed")])}>
        <RuntimeControlActivityGroup tools={[killTool()]} />
      </SubagentRunsContext.Provider>,
    )

    screen.getByRole("button", { name: /completed close 1 agent\b/i })
  })

  test("keeps the original tool/result available in an automatically opened failed group", () => {
    render(
      <RuntimeControlActivityGroup
        tools={[killTool({ result: "Tool error: denied", isError: true })]}
      />,
    )

    expect(screen.getByTestId("runtime-detail").textContent).toBe("kill-1:Tool error: denied")
    expect(screen.getByTestId("runtime-detail").getAttribute("data-label")).toMatch(
      /failed close 1 agent\b/i,
    )
  })

  test("opens when a request becomes problematic and still allows a manual collapse", () => {
    const { rerender } = render(<RuntimeControlActivityGroup tools={[killTool()]} />)
    expect(screen.getByRole("button").getAttribute("aria-expanded")).toBe("false")

    rerender(
      <RuntimeControlActivityGroup
        tools={[killTool({ result: "Tool error: denied", isError: true })]}
      />,
    )
    const header = screen.getByRole("button")
    expect(header.getAttribute("aria-expanded")).toBe("true")

    fireEvent.click(header)
    expect(header.getAttribute("aria-expanded")).toBe("false")
  })

  test("describes a no-target kill-all aggregate without inventing a target count", () => {
    const { container } = render(
      <RuntimeControlActivityGroup
        tools={[
          {
            callId: "kill-all-1",
            name: "subagent",
            arguments: JSON.stringify({ action: "kill_all" }),
            result: JSON.stringify({
              disposition: "no_targets",
              terminal: true,
              requested_count: 0,
              terminal_count: 0,
              pending_count: 0,
              refused_count: 0,
            }),
          },
        ]}
      />,
    )

    const header = screen.getByRole("button", { name: /no agents need closing/i })
    expect(header.textContent).not.toContain("1 target")
    expect(container.querySelector("[data-runtime-control-aggregate]")).toBeNull()
  })

  test("renders runtime processes as process closes rather than job cancellations", () => {
    render(
      <RuntimeControlActivityGroup
        tools={[
          {
            callId: "process-1",
            name: "runtime_cancel",
            arguments: JSON.stringify({ kind: "process", id: "pid-1" }),
            result: JSON.stringify({
              disposition: "requested",
              status: "killed",
              finalStatus: "killed",
            }),
          },
        ]}
      />,
    )

    screen.getByRole("button", { name: /completed close 1 process\b/i })
  })

  test("opens a mixed kill-all result and exposes every aggregate count", () => {
    const { container } = render(
      <RuntimeControlActivityGroup
        tools={[
          {
            callId: "kill-all-mixed",
            name: "subagent",
            arguments: JSON.stringify({ action: "kill_all" }),
            result: JSON.stringify({
              disposition: "requested",
              terminal: false,
              requested_count: 2,
              terminal_count: 1,
              pending_count: 1,
              refused_count: 1,
              runs: [],
            }),
          },
        ]}
      />,
    )

    expect(screen.getByRole("button").getAttribute("aria-expanded")).toBe("true")
    expect(container.querySelector("[data-runtime-control-aggregate]")).not.toBeNull()
    screen.getByText("1 refused")
    screen.getByText("requested 2")
    screen.getByText("terminal 1")
    screen.getByText("pending 1")
    screen.getByText("refused 1")
  })

  test("starts the 100ms clock only for a running call with a start timestamp", () => {
    vi.useFakeTimers()
    const intervalSpy = vi.spyOn(window, "setInterval")
    const { rerender } = render(
      <RuntimeControlActivityGroup tools={[killTool({ result: undefined })]} />,
    )

    expect(intervalSpy).not.toHaveBeenCalled()
    rerender(
      <RuntimeControlActivityGroup
        tools={[killTool({ result: undefined, startedAtMs: Date.now() })]}
      />,
    )
    expect(intervalSpy).toHaveBeenCalledWith(expect.any(Function), 100)
  })

  test("announces an accepted request becoming completed through a polite status region", () => {
    const requestedKill = killTool({
      result: JSON.stringify({
        disposition: "requested",
        requested: true,
        terminal: false,
        status: "running",
        final_status: null,
      }),
    })
    const { rerender } = render(
      <SubagentRunsContext.Provider value={runsView([])}>
        <RuntimeControlActivityGroup tools={[requestedKill]} />
      </SubagentRunsContext.Provider>,
    )

    expect(screen.getByRole("status")).toHaveTextContent(/requested close 1 agent\b/i)
    expect(screen.getByRole("status").getAttribute("aria-live")).toBe("polite")

    rerender(
      <SubagentRunsContext.Provider value={runsView([run("completed")])}>
        <RuntimeControlActivityGroup tools={[requestedKill]} />
      </SubagentRunsContext.Provider>,
    )
    expect(screen.getByRole("status")).toHaveTextContent(/completed close 1 agent\b/i)
  })

  test("prioritizes a partial Team resume, opens failures, and preserves raw results", () => {
    const completed = teamResumeTool("resume-full", {
      status: "resumed",
      teamStatus: "active",
      disposition: "resumed",
      resumedMemberCount: 2,
      failedMemberCount: 0,
      failures: [],
    })
    const partial = teamResumeTool("resume-partial", {
      status: "partially_resumed",
      teamStatus: "active",
      disposition: "partial",
      resumedMemberCount: 1,
      failedMemberCount: 1,
      failures: [
        {
          name: "reviewer",
          reason: "old_attempt_active",
          oldAttemptStatus: "running",
        },
      ],
    })

    render(<RuntimeControlActivityGroup tools={[completed, partial]} />)

    const header = screen.getByRole("button")
    expect(header.getAttribute("aria-expanded")).toBe("true")
    const status = screen.getByRole("status")
    expect(status).toHaveTextContent("partial resume 1 team")
    expect(status.className).toContain("text-amber")
    screen.getByText("resumed 1")
    screen.getByText("failed 1")
    screen.getByText("Resume failures")
    screen.getByText("reviewer · old_attempt_active · running")
    expect(
      screen
        .getAllByTestId("runtime-detail")
        .some((node) => node.textContent?.includes("resume-partial")),
    ).toBe(true)
  })

  test("shows a no-op Team resume as neutral completion without a success check", () => {
    const { container } = render(
      <RuntimeControlActivityGroup
        tools={[
          teamResumeTool("resume-no-op", {
            status: "already_complete",
            teamStatus: "paused",
            disposition: "no_op",
            resumedMemberCount: 0,
            failedMemberCount: 0,
            failures: [],
          }),
        ]}
      />,
    )

    expect(screen.getByRole("status")).toHaveTextContent("team already complete · no resume needed")
    expect(container.querySelector(".text-green-500\\/80")).toBeNull()
  })
})
