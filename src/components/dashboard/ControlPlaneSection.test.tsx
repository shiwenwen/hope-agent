// @vitest-environment jsdom

import type { ReactNode } from "react"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import type { ControlPlaneDashboard } from "./types"
import ControlPlaneSection from "./ControlPlaneSection"

const { transportCall } = vi.hoisted(() => ({
  transportCall: vi
    .fn()
    .mockResolvedValue([{ id: "project-1", name: "Archived project", archived: true }]),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: transportCall }),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}))

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  LineChart: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  Line: () => null,
  CartesianGrid: () => null,
  XAxis: () => null,
  YAxis: () => null,
  Tooltip: () => null,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const ratio = (numerator: number, denominator: number, rate: number | null) => ({
  numerator,
  denominator,
  rate,
})

const duration = { p50Secs: null, sampleCount: 0, eligibleCount: 1 }

function fixture(): ControlPlaneDashboard {
  return {
    summary: {
      goalAcceptance: ratio(0, 0, null),
      workflowCompletion: ratio(1, 2, 0.5),
      loopStrongProgress: ratio(1, 3, 1 / 3),
      taskCohortCompletion: ratio(1, 1, 1),
      attentionCount: 1,
    },
    goals: {
      acceptance: ratio(0, 0, null),
      requiredCriteria: ratio(0, 0, null),
      auditedGoalCount: 0,
      acceptedDuration: duration,
      currentStates: [],
      closureOutcomes: [],
      domains: [],
      trend: [],
    },
    workflows: {
      completion: ratio(1, 2, 0.5),
      opFailure: ratio(1, 4, 0.25),
      duration,
      goalBinding: ratio(1, 2, 0.5),
      approvalTrigger: ratio(1, 2, 0.5),
      currentStates: [],
      kinds: [],
      origins: [],
      trend: [],
    },
    loops: {
      strongProgress: ratio(1, 3, 1 / 3),
      noProgress: ratio(1, 3, 1 / 3),
      duration,
      currentBlockedSchedules: 1,
      progressStates: [],
      triggerKinds: [],
      strategies: [],
      currentStates: [],
      trend: [],
    },
    tasks: {
      cohortCompletion: ratio(1, 1, 1),
      currentBacklog: 2,
      duration,
      currentStates: [],
      trend: [],
    },
    plans: {
      cohortCompletion: ratio(1, 1, 1),
      activeNow: 1,
      duration,
      currentStates: [],
      byAgent: [],
      byProject: [],
      trend: [],
    },
    attention: {
      total: 1,
      items: [
        {
          kind: "goal",
          id: "goal-1",
          sessionId: "session-1",
          title: "Fix goal",
          status: "blocked",
          reason: "needs_strict_evidence",
          severity: "critical",
          updatedAt: "2026-07-15T00:00:00Z",
        },
      ],
    },
  }
}

describe("ControlPlaneSection", () => {
  it("renders an explicit empty state", () => {
    render(
      <ControlPlaneSection
        data={null}
        loading={false}
        projectId={null}
        onProjectChange={() => {}}
      />,
    )
    expect(screen.getByText("dashboard.controlPlane.empty")).toBeTruthy()
  })

  it("uses an em dash for a zero-denominator metric and opens attention items", async () => {
    const onOpenAttention = vi.fn()
    render(
      <ControlPlaneSection
        data={fixture()}
        loading={false}
        projectId={null}
        onProjectChange={() => {}}
        onOpenAttention={onOpenAttention}
      />,
    )
    await waitFor(() =>
      expect(transportCall).toHaveBeenCalledWith("list_projects_cmd", {
        includeArchived: true,
      }),
    )
    expect(screen.getAllByText("—").length).toBeGreaterThan(0)
    expect(screen.getByText(/dashboard\.controlPlane\.attentionKinds\.goal/)).toBeTruthy()
    expect(screen.getByText(/dashboard\.controlPlane\.states\.blocked/)).toBeTruthy()
    expect(
      screen.getByText(/dashboard\.controlPlane\.attentionReasons\.needs_strict_evidence/),
    ).toBeTruthy()
    fireEvent.click(screen.getByText("Fix goal"))
    expect(onOpenAttention).toHaveBeenCalledWith(expect.objectContaining({ id: "goal-1" }))
  })

  it("keeps Plan history as a separate destination", async () => {
    const onOpenPlanHistory = vi.fn()
    render(
      <ControlPlaneSection
        data={fixture()}
        loading={false}
        projectId={null}
        onProjectChange={() => {}}
        onOpenPlanHistory={onOpenPlanHistory}
        initialSection="progress"
      />,
    )
    fireEvent.click(await screen.findByText("dashboard.controlPlane.openPlanHistory"))
    expect(onOpenPlanHistory).toHaveBeenCalledTimes(1)
  })
})
