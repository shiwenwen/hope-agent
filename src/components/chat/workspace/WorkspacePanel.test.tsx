// @vitest-environment jsdom

import type { ComponentProps } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { WorkspaceEnvironmentState } from "./useWorkspaceEnvironment"
import type {
  CodingImprovementProposal,
  CodingTrendReport,
  ContextRetrievalSnapshot,
  DomainArtifactExportGuardReport,
  DomainConnectorActionGuardReport,
  DomainConnectorE2EGateReport,
  DomainEvidenceItem,
  DomainOperationalGateReport,
  DomainQualityRunSnapshot,
  DomainSoakReport,
  DomainWorkflowDraft,
  DomainWorkflowTemplate,
  ManagedWorktree,
  RecordDomainEvidenceInput,
  WorkspaceEnvironmentSnapshot,
} from "@/lib/transport"
import type { BackgroundJobSnapshot } from "@/types/background-jobs"
import WorkspacePanel from "./WorkspacePanel"
import type { GoalSnapshot } from "./useGoal"
import type { LoopSchedule, LoopSnapshot } from "./useLoopSchedules"
import type { WorkflowRun, WorkflowRunSnapshot, WorkflowScriptPreview } from "./useWorkflowRuns"

const envMock = vi.hoisted(() => ({
  state: {
    snapshot: null,
    loading: false,
    error: null,
  } as WorkspaceEnvironmentState,
}))

const transportMock = vi.hoisted(() => ({
  supportsLocalFileOps: vi.fn(() => true),
  call: vi.fn<(name: string, args?: Record<string, unknown>) => Promise<unknown>>(
    (name: string) => {
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "get_coding_trend_report") return Promise.resolve(null)
      if (name === "get_lsp_status") return Promise.resolve(null)
      if (name === "get_lsp_diagnostics") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_e2e_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      return Promise.resolve([])
    },
  ),
  listen: vi.fn<(eventName: string, handler: (payload: unknown) => void) => () => void>(
    () => () => {},
  ),
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
  getTransport: () => transportMock,
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
    filesTruncated: false,
    sourcesTruncated: false,
  }),
}))

beforeEach(() => {
  Object.defineProperty(Element.prototype, "hasPointerCapture", {
    configurable: true,
    value: () => false,
  })
  Object.defineProperty(Element.prototype, "setPointerCapture", {
    configurable: true,
    value: () => {},
  })
  Object.defineProperty(Element.prototype, "releasePointerCapture", {
    configurable: true,
    value: () => {},
  })
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value: () => {},
  })
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  )
  transportMock.supportsLocalFileOps.mockReturnValue(true)
  // KnowledgeSection (useSessionKnowledge) fetches attachments + subscribes to
  // knowledge:changed — stub both so the panel mounts in tests.
  transportMock.call.mockImplementation((name: string) => {
    if (name === "get_background_job") return Promise.resolve(null)
    if (name === "get_coding_trend_report") return Promise.resolve(null)
    if (name === "get_lsp_status") return Promise.resolve(null)
    if (name === "get_lsp_diagnostics") return Promise.resolve(null)
    if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
    if (name === "evaluate_domain_connector_e2e_gate") return Promise.resolve(null)
    if (name === "generate_domain_soak_report") return Promise.resolve(null)
    return Promise.resolve([])
  })
  transportMock.listen.mockImplementation(() => () => {})
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  envMock.state = { snapshot: null, loading: false, error: null }
  transportMock.supportsLocalFileOps.mockReset()
  transportMock.call.mockReset()
  transportMock.listen.mockReset()
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
  envState: Partial<WorkspaceEnvironmentState> = {},
) {
  envMock.state = { snapshot, loading: false, error: null, ...envState }
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

async function clickTextButton(text: string) {
  const matches = await screen.findAllByText(text)
  const button = matches.map((element) => element.closest("button")).find(Boolean)
  fireEvent.click(button ?? matches[0])
}

async function clickSectionHeader(title: string) {
  const buttons = await screen.findAllByRole("button", { name: new RegExp(title) })
  const header = buttons.find((button) => button.hasAttribute("aria-expanded"))
  fireEvent.click(header ?? buttons[0])
}

async function flushContextRetrievalDebounce() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(260)
  })
  await act(async () => {
    await Promise.resolve()
  })
}

function contextRetrievalSnapshot(): ContextRetrievalSnapshot {
  return {
    sessionId: "s1",
    query: null,
    workspaceRoot: "/repo",
    candidates: [
      {
        id: "doc-1",
        kind: "document",
        title: "Browser automation notes",
        subtitle: "Drive document",
        path: "/repo/docs/browser.md",
        line: 12,
        url: null,
        score: 82,
        reasons: ["required evidence for research"],
        sources: ["domain", "knowledge"],
        status: "fresh",
        metadata: {
          domain: "research",
          confidence: 0.91,
          accessScope: "project",
          redactionStatus: "none",
          domainActions: {
            canCite: true,
            canSummarize: true,
            canAskUser: true,
            canAddEvidence: true,
            canMarkConflict: true,
            canCreateTask: true,
          },
        },
      },
    ],
    stats: {
      gitChanges: 0,
      artifactFiles: 0,
      diagnostics: 0,
      reviewFindings: 0,
      verificationSteps: 0,
      goalEvidence: 0,
      tasks: 0,
      workflowOps: 0,
      ideContextSignals: 0,
      fileSearchMatches: 0,
      symbols: 0,
      urlSources: 0,
      domainCandidates: 1,
      domainEvidence: 0,
      accessIssues: 0,
      warnings: [],
    },
    domainContext: {
      domain: "research",
      templateId: "research-brief",
      templateVersion: "1.0.0",
      templateTitle: "Research brief",
      taskType: "technical_research",
      goalId: null,
      goalObjective: null,
      completionCriteria: null,
      requiredEvidence: [],
      approvalGates: [],
      verificationPolicy: [],
      source: "template",
    },
    accessIssues: [],
    truncated: false,
    disabledReason: null,
    generatedAt: "2026-01-01T00:00:00Z",
  }
}

function domainEvidenceItem(patch: Partial<DomainEvidenceItem> = {}): DomainEvidenceItem {
  return {
    id: "evidence-1",
    goalId: null,
    sessionId: "s1",
    projectId: null,
    domain: "research",
    evidenceType: "artifact_created",
    title: "上下文摘要：Browser automation notes",
    summary: "summary",
    sourceMetadata: {},
    confidence: 0.91,
    accessScope: "project",
    redactionStatus: "none",
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...patch,
  }
}

function workflowRun(patch: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "wf-1",
    sessionId: "s1",
    kind: "coding.feature",
    state: "awaiting_approval",
    executionMode: "guarded",
    scriptHash: "abcdef123456",
    scriptSource: "export default async function main() {}",
    budget: {},
    cursorSeq: 0,
    primaryOwner: null,
    blockedReason: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:01:00Z",
    completedAt: null,
    ...patch,
  }
}

function workflowSnapshot(run: WorkflowRun): WorkflowRunSnapshot {
  return {
    run,
    ops: [
      {
        id: "op-1",
        runId: run.id,
        opKey: "main/op#1(workflow.tool)",
        opType: "tool",
        effectClass: "non_idempotent",
        inputHash: "hash-1",
        input: { name: "write", label: "write-file" },
        state: "pending",
        output: null,
        error: null,
        childHandle: null,
        startedAt: "2026-01-01T00:01:00Z",
        completedAt: null,
      },
    ],
    events: [
      {
        id: 1,
        runId: run.id,
        seq: 1,
        eventType: "script_permission_preview",
        payload: {
          summary: { total: 2, allow: 1, ask: 1, dynamic: 1, deny: 0, strict: 1 },
          calls: [
            {
              api: "workflow.tool",
              line: 3,
              toolName: "write",
              decision: "ask",
              strict: true,
              dynamic: false,
              reason: "edit-class tool requires approval",
              label: "write-file",
              args: { path: "src/app.ts", content: "hello" },
            },
            {
              api: "workflow.tool",
              line: 4,
              toolName: "read",
              decision: "allow",
              strict: false,
              dynamic: false,
              label: "read-file",
              args: { path: "src/app.ts" },
            },
          ],
          truncated: false,
        },
        createdAt: "2026-01-01T00:00:30Z",
      },
      {
        id: 2,
        runId: run.id,
        seq: 2,
        eventType: "script_permission_approval_required",
        payload: { summary: { total: 2, ask: 1, dynamic: 1, deny: 0, strict: 1 } },
        createdAt: "2026-01-01T00:01:00Z",
      },
    ],
  }
}

function loopSchedule(patch: Partial<LoopSchedule> = {}): LoopSchedule {
  return {
    id: "loop-1",
    sessionId: "s1",
    goalId: "goal-1",
    cronJobId: "cron-loop-1",
    prompt: "Update the report",
    triggerKind: "interval",
    triggerSpec: { intervalSecs: 600 },
    executionStrategy: "workflow",
    state: "active",
    maxRuns: null,
    runCount: 1,
    maxRuntimeSecs: null,
    tokenBudget: null,
    costBudgetMicros: null,
    progressState: "weak_progress",
    progressSummary: "created a derived workflow run",
    noProgressStreak: 0,
    failureStreak: 0,
    maxNoProgressRuns: 3,
    maxFailures: 3,
    backoffSecs: 300,
    nextRunAt: "2026-01-01T00:10:00Z",
    cronStatus: "active",
    approvalPolicySnapshot: {},
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:04:00Z",
    completedAt: null,
    blockedReason: null,
    ...patch,
  }
}

function loopSnapshot(patch: Partial<LoopSnapshot> = {}): LoopSnapshot {
  const schedule = loopSchedule()
  return {
    schedule,
    runs: [
      {
        id: "lrun-1",
        loopId: schedule.id,
        cronJobId: schedule.cronJobId,
        cronRunLogId: 7,
        sessionId: "s1",
        seq: 1,
        state: "succeeded",
        triggerReason: "interval trigger",
        resultSummary: "workflow launched",
        error: null,
        progressState: "weak_progress",
        progressDelta: { workflowRunId: "wf-loop" },
        noProgressReason: null,
        schedulingDecision: "continue",
        trace: {
          workflowRunId: "wf-loop",
          templateId: "research-brief",
          templateVersion: "1.0.0",
        },
        startedAt: "2026-01-01T00:04:00Z",
        finishedAt: "2026-01-01T00:04:05Z",
      },
    ],
    ...patch,
  }
}

function domainQualitySnapshot(
  patch: Partial<DomainQualityRunSnapshot> = {},
): DomainQualityRunSnapshot {
  return {
    run: {
      id: "dq-1",
      sessionId: "s1",
      goalId: "goal-1",
      domain: "research",
      templateId: "research-brief",
      templateVersion: "1.0.0",
      state: "completed",
      summary: "Research quality passed",
      stats: { passed: 4, failed: 0, needsUser: 0, advisory: 1 },
      error: null,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:04:00Z",
      completedAt: "2026-01-01T00:04:00Z",
    },
    checks: [
      {
        id: "dqc-1",
        runId: "dq-1",
        sessionId: "s1",
        seq: 1,
        checkType: "required_evidence",
        profile: "research",
        title: "Sources cited",
        body: "Enough dated sources were cited.",
        severity: "p1",
        status: "passed",
        evidenceType: "source_cited",
        sourceMetadata: {},
        createdAt: "2026-01-01T00:04:00Z",
        updatedAt: "2026-01-01T00:04:00Z",
      },
    ],
    events: [],
    ...patch,
  }
}

function codingImprovementProposal(
  patch: Partial<CodingImprovementProposal> = {},
): CodingImprovementProposal {
  return {
    id: "proposal-1",
    sessionId: "s1",
    projectId: "p1",
    kind: "domain_eval_case",
    status: "promoted",
    sourceType: "domain_quality",
    sourceId: "dq-1",
    title: "Inbox send approval eval",
    body: "Regression case for requiring user approval before external sends.",
    payload: { domain: "inbox" },
    fingerprint: "fingerprint-1",
    action: {
      applied: true,
      artifacts: [{ kind: "create_file", path: "draft-domain-eval.json" }],
      error: null,
      appliedAt: "2026-01-01T00:02:00Z",
    },
    promotion: {
      promoted: true,
      artifacts: [{ kind: "create_promoted_file", path: "domain-eval/inbox/send.json" }],
      error: null,
      promotedAt: "2026-01-01T00:03:00Z",
    },
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:03:00Z",
    decidedAt: "2026-01-01T00:03:00Z",
    ...patch,
  }
}

function codingTrendReport(
  patch: Partial<CodingTrendReport> = {},
  proposals: CodingImprovementProposal[] = [codingImprovementProposal()],
): CodingTrendReport {
  return {
    sessionId: "s1",
    projectId: "p1",
    scope: "project",
    windowDays: 30,
    generatedAt: "2026-01-01T00:05:00Z",
    overview: {
      sessions: 1,
      goals: 1,
      completedGoals: 1,
      blockedGoals: 0,
      workflowRuns: 1,
      completedWorkflows: 1,
      blockedWorkflows: 0,
      failedWorkflows: 0,
      goalCompletionRate: 1,
      workflowCompletionRate: 1,
    },
    eval: { runs: 1, passed: 1, failed: 0, successRate: 1, backlogCandidates: 0 },
    review: {
      runs: 0,
      findings: 0,
      blockingFindings: 0,
      resolvedFindings: 0,
      falsePositiveFindings: 0,
      byCategory: [],
    },
    verification: {
      runs: 0,
      steps: 0,
      passedSteps: 0,
      failedSteps: 0,
      timedOutSteps: 0,
      plannedOnlyRuns: 0,
      executedSuccessRate: null,
      recommendationCoverage: null,
    },
    repairLoop: { runs: 0, completed: 0, blocked: 0, exhausted: 0, successRate: null },
    retro: {
      total: 0,
      completed: 0,
      blocked: 0,
      failed: 0,
      cancelled: 0,
      recommendations: 0,
      latestSummary: null,
    },
    failures: [],
    recentRuns: [],
    retros: [],
    proposals,
    ...patch,
  }
}

function workflowScriptPreview(patch: Partial<WorkflowScriptPreview> = {}): WorkflowScriptPreview {
  return {
    gate: { issues: [] },
    gatePassed: true,
    gateFeedback: "Workflow Script Gate passed.",
    permission: {
      summary: { total: 2, allow: 1, ask: 1, dynamic: 0, deny: 0, strict: 1 },
      calls: [
        {
          api: "workflow.validate",
          line: 4,
          toolName: "exec",
          decision: "ask",
          strict: true,
          dynamic: false,
          label: "typecheck",
          args: { command: "pnpm typecheck" },
        },
      ],
      truncated: false,
    },
    canCreate: true,
    canRunImmediately: true,
    requiresApproval: true,
    hasDenials: false,
    ...patch,
  }
}

function domainWorkflowTemplate(
  patch: Partial<DomainWorkflowTemplate> = {},
): DomainWorkflowTemplate {
  return {
    id: "research-brief",
    version: "1.0.0",
    title: "Research brief",
    domain: "research",
    taskTypes: ["technical_research", "competitive_analysis"],
    defaultMode: "guarded",
    requiredEvidence: [
      {
        evidenceType: "source_cited",
        title: "At least three dated sources",
        required: true,
        minCount: 3,
        metadataKeys: ["uri", "retrievedAt"],
      },
      {
        evidenceType: "claim_checked",
        title: "Important claims checked",
        required: true,
        minCount: 2,
        metadataKeys: ["claim", "verdict"],
      },
    ],
    recommendedTools: ["web_search", "knowledge_recall"],
    approvalGates: [
      {
        action: "external_publish",
        reason: "User approves before publishing",
        required: true,
      },
    ],
    verificationPolicy: [
      {
        rule: "citation_freshness",
        severity: "blocking",
        description: "Flag stale sources.",
      },
    ],
    stopConditions: ["Required citations are missing"],
    outputContract: "Answer-first research brief with cited sources.",
    evalCriteria: ["Claims are cited"],
    promptHints: ["Prefer primary sources"],
    scope: "built_in",
    projectId: null,
    enabled: true,
    createdAt: "builtin",
    updatedAt: "builtin",
    ...patch,
  }
}

function domainWorkflowDraft(
  template: DomainWorkflowTemplate = domainWorkflowTemplate(),
  patch: Partial<DomainWorkflowDraft> = {},
): DomainWorkflowDraft {
  const scriptSource = `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Research brief" });
  await workflow.askUser({ label: "domain-plan-confirmation", questions: [] });
  await workflow.finish({ summary: "draft ready" });
}`
  return {
    template,
    sessionId: "s1",
    goalId: null,
    executionMode: "guarded",
    workflowKind: "domain:research",
    scriptSource,
    scriptPreview: workflowScriptPreview() as unknown as DomainWorkflowDraft["scriptPreview"],
    requiredEvidence: template.requiredEvidence,
    approvalGates: template.approvalGates,
    verificationPolicy: template.verificationPolicy,
    warnings: ["User approval is required before publication"],
    ...patch,
  }
}

function managedWorktree(patch: Partial<ManagedWorktree> = {}): ManagedWorktree {
  return {
    id: "wt-repair",
    sessionId: "s1",
    childSessionId: null,
    workflowRunId: "wf-1",
    purpose: "workflow",
    state: "active",
    label: "repair-wt",
    repoRoot: "/repo",
    sourceWorkingDir: "/repo",
    path: "/repo-worktrees/wt-repair",
    baseRef: "main",
    baseBranch: "main",
    baseSha: "abcdef123456",
    gitBranch: "repair/wt",
    dirtySnapshot: null,
    pathExists: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:01:00Z",
    archivedAt: null,
    restoredAt: null,
    handedOffAt: null,
    ...patch,
  }
}

function domainOperationalGateReport(
  patch: Partial<DomainOperationalGateReport> = {},
): DomainOperationalGateReport {
  return {
    generatedAt: "2026-01-01T00:05:00Z",
    status: "failed",
    scope: "session",
    sessionId: "s1",
    projectId: null,
    domain: "research",
    since: "2025-12-18T00:00:00Z",
    thresholds: {
      windowDays: 14,
      minWorkflowRuns: 1,
      maxFailedWorkflowRuns: 0,
      maxBlockedWorkflowRuns: 0,
      maxCancelledWorkflowRuns: 0,
      maxActiveWorkflowRuns: 0,
      minLoopRuns: 0,
      maxFailedLoopRuns: 0,
      maxActiveCampaigns: 0,
      maxFailedCampaignItems: 0,
    },
    summary: {
      workflowRuns: 2,
      completedWorkflowRuns: 1,
      failedWorkflowRuns: 1,
      blockedWorkflowRuns: 0,
      cancelledWorkflowRuns: 0,
      activeWorkflowRuns: 1,
      pausedWorkflowRuns: 0,
      awaitingApprovalWorkflowRuns: 0,
      loopSchedules: 1,
      activeLoopSchedules: 0,
      loopRuns: 1,
      succeededLoopRuns: 1,
      failedLoopRuns: 0,
      activeLoopRuns: 0,
      campaigns: 0,
      activeCampaigns: 0,
      campaignItems: 0,
      passedCampaignItems: 0,
      failedCampaignItems: 0,
      cancelledCampaignItems: 0,
      interruptedCampaignItems: 0,
      latestActivityAt: "2026-01-01T00:04:00Z",
      maxActiveWorkAgeSecs: 120,
    },
    checks: [
      {
        name: "workflow_failed_residue",
        status: "failed",
        severity: "critical",
        expected: "0 failed workflow runs",
        actual: "1",
        detail: "A workflow run failed in the active window.",
      },
    ],
    blockers: ["failed workflow residue"],
    recommendedNextSteps: ["Open the failed workflow and repair it."],
    ...patch,
  }
}

function domainSoakReport(patch: Partial<DomainSoakReport> = {}): DomainSoakReport {
  const operationalGate = domainOperationalGateReport()
  return {
    generatedAt: "2026-01-01T00:06:00Z",
    status: "failed",
    scope: "session",
    sessionId: "s1",
    projectId: null,
    domain: "research",
    windowDays: 14,
    since: "2025-12-18T00:00:00Z",
    until: "2026-01-01T00:06:00Z",
    summary: {
      workflowRuns: 2,
      completedWorkflowRuns: 1,
      failedWorkflowRuns: 1,
      blockedWorkflowRuns: 0,
      cancelledWorkflowRuns: 0,
      activeWorkflowRuns: 0,
      awaitingApprovalWorkflowRuns: 0,
      repairWorkflowRuns: 0,
      approvalEvents: 0,
      approvalRequestEvents: 1,
      approvalDecisionEvents: 0,
      openApprovalWaits: 1,
      pauseEvents: 0,
      resumeEvents: 0,
      cancelEvents: 0,
      recoveryEvents: 0,
      workflowControlInterventionEvents: 3,
      workflowBudgetUsageEvents: 2,
      workflowBudgetExhaustedEvents: 1,
      maxWorkflowOutputTokensSpent: 10,
      maxWorkflowOutputTokenBudget: 10,
      averageApprovalWaitSecs: null,
      maxApprovalWaitSecs: null,
      maxOpenApprovalWaitSecs: 120,
      averageWorkflowDrainSecs: 120,
      maxWorkflowDrainSecs: 240,
      latestActivityAt: "2026-01-01T00:04:00Z",
      latestActivityAgeSecs: 120,
      sampleDays: 1,
      requiredSampleDays: 2,
      loopRuns: 1,
      succeededLoopRuns: 1,
      failedLoopRuns: 0,
      activeLoopRuns: 0,
      averageLoopDurationSecs: 30,
      maxLoopDurationSecs: 30,
      campaigns: 0,
      activeCampaigns: 0,
      campaignItems: 0,
      passedCampaignItems: 0,
      failedCampaignItems: 0,
      cancelledCampaignItems: 0,
      interruptedCampaignItems: 0,
      retriedCampaignItems: 0,
      averageCampaignItemDurationSecs: null,
      maxCampaignItemDurationSecs: null,
      connectorE2eEvidence: 0,
      connectorExecutionEvidence: 0,
      connectorVerificationEvidence: 0,
      incidents: 1,
      criticalIncidents: 1,
      warningIncidents: 0,
      totalRecords: 3,
    },
    incidents: [
      {
        source: "workflow",
        id: "wf-failed",
        title: "Workflow failed",
        status: "failed",
        severity: "critical",
        startedAt: "2026-01-01T00:00:00Z",
        finishedAt: "2026-01-01T00:04:00Z",
        durationSecs: 240,
        reason: "validation failed",
        recommendation: "Repair the workflow before continuing the loop.",
      },
    ],
    timeline: [
      {
        source: "workflow",
        id: "wf-failed",
        label: "Workflow failed",
        status: "failed",
        at: "2026-01-01T00:04:00Z",
        durationSecs: 240,
      },
    ],
    recommendedNextSteps: ["Repair the failed workflow."],
    markdown: "Workflow failed",
    operationalGate,
    ...patch,
  }
}

function domainArtifactExportGuardReport(
  patch: Partial<DomainArtifactExportGuardReport> = {},
): DomainArtifactExportGuardReport {
  return {
    generatedAt: "2026-01-01T00:07:00Z",
    status: "failed",
    scope: { scope: "session", sessionId: "s1", projectId: null, goalId: null, domain: "research" },
    artifactPath: null,
    artifactTitle: "Research brief",
    artifactKind: "brief",
    thresholds: {
      requireArtifactCreated: true,
      requireArtifactReviewed: true,
      maxSensitiveUnreviewed: 0,
      maxRedactionPending: 0,
    },
    summary: {
      evidenceItems: 2,
      artifactCreated: 1,
      artifactReviewed: 0,
      exportReviewed: 0,
      sensitiveEvidence: 1,
      sensitiveUnreviewed: 1,
      redactionPending: 1,
      privateOrConnectorEvidence: 1,
    },
    checks: [
      {
        name: "artifact_reviewed",
        status: "failed",
        severity: "critical",
        expected: "reviewed artifact",
        actual: "0",
        detail: "The delivery artifact has not been reviewed.",
      },
    ],
    blockers: ["artifact review missing"],
    recommendedNextSteps: ["Review the artifact before export."],
    evidenceRequiringReview: [
      {
        id: "e-sensitive",
        evidenceType: "source_cited",
        title: "Private source",
        accessScope: "connector",
        redactionStatus: "pending",
        createdAt: "2026-01-01T00:03:00Z",
        reason: "sensitive_unreviewed",
      },
    ],
    ...patch,
  }
}

function domainConnectorActionGuardReport(
  patch: Partial<DomainConnectorActionGuardReport> = {},
): DomainConnectorActionGuardReport {
  return {
    generatedAt: "2026-01-01T00:08:00Z",
    status: "failed",
    scope: { scope: "session", sessionId: "s1", projectId: null, goalId: null, domain: "inbox" },
    toolName: "gmail_send",
    connector: "gmail",
    action: "send",
    risk: "external_write",
    thresholds: {
      requireExplicitApproval: true,
      requireRollbackPlan: true,
      requireExportGuardForDelivery: true,
    },
    summary: {
      evidenceItems: 2,
      actionEvidence: 1,
      approvalEvidence: 0,
      rollbackEvidence: 0,
      sensitiveEvidence: 1,
      deliveryAction: true,
      exportGuardStatus: "failed",
    },
    checks: [
      {
        name: "explicit_user_approval",
        status: "failed",
        severity: "critical",
        expected: "user approval evidence",
        actual: "0",
        detail: "No explicit user approval evidence exists for the external action.",
      },
    ],
    blockers: ["approval missing"],
    recommendedNextSteps: ["Get user approval and rollback evidence before sending."],
    relatedEvidence: [
      {
        id: "e-draft",
        evidenceType: "message_draft_approved",
        title: "Reply draft",
        accessScope: "connector",
        redactionStatus: "none",
        createdAt: "2026-01-01T00:04:00Z",
        reason: "action_scope",
      },
    ],
    ...patch,
  }
}

function domainConnectorE2EGateReport(
  patch: Partial<DomainConnectorE2EGateReport> = {},
): DomainConnectorE2EGateReport {
  return {
    generatedAt: "2026-01-01T00:09:00Z",
    status: "insufficient_data",
    scope: { scope: "session", sessionId: "s1", projectId: null, goalId: null, domain: "inbox" },
    toolName: "gmail_send",
    connector: "gmail",
    action: "send",
    risk: "external_write",
    thresholds: {
      requireConnectorInput: true,
      requireDraft: true,
      requireExplicitApproval: true,
      requireExecutionResult: true,
      requirePostActionVerification: true,
      requireRollbackPlan: true,
      requireExportGuardForDelivery: true,
    },
    summary: {
      evidenceItems: 3,
      connectorInputEvidence: 1,
      draftEvidence: 1,
      approvalEvidence: 1,
      executionEvidence: 0,
      verificationEvidence: 0,
      rollbackEvidence: 0,
      sensitiveEvidence: 1,
      deliveryAction: true,
      connectorActionGuardStatus: "passed",
      exportGuardStatus: "passed",
    },
    checks: [
      {
        name: "execution_result",
        status: "insufficient_data",
        severity: "critical",
        expected: "connector execution result evidence",
        actual: "0",
        detail: "The connector action has not produced an execution result yet.",
      },
      {
        name: "post_action_verification",
        status: "insufficient_data",
        severity: "major",
        expected: "post action verification evidence",
        actual: "0",
        detail: "The external state has not been verified after execution.",
      },
    ],
    blockers: [],
    recommendedNextSteps: ["Execute the approved connector action and verify the external state."],
    relatedEvidence: [
      {
        id: "e-connector-input",
        evidenceType: "connector_context_collected",
        title: "Gmail thread context",
        accessScope: "connector",
        redactionStatus: "clean",
        createdAt: "2026-01-01T00:05:00Z",
        reason: "connector_input",
      },
    ],
    ...patch,
  }
}

function goalSnapshotWithWorktreeEvidence(): GoalSnapshot {
  return {
    goal: {
      id: "goal-1",
      sessionId: "s1",
      objective: "Ship isolated worktree",
      completionCriteria: "Worktree evidence is visible",
      state: "active",
      modeSnapshot: null,
      budgetTokenLimit: null,
      budgetTimeLimitSecs: null,
      budgetTurnLimit: null,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:02:00Z",
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: {},
    },
    links: [],
    events: [],
    criteria: [],
    evidence: [
      {
        id: "worktree:wt_goal:worktree_attached",
        sourceType: "worktree",
        sourceId: "wt_goal",
        relation: "worktree_attached",
        title: "Worktree attached: feature-goal",
        summary: "handoff at /repo-worktrees/wt_goal",
        metadata: {
          worktreeId: "wt_goal",
          runId: "wf-goal-1",
          label: "feature-goal",
          state: "handoff",
          path: "/repo-worktrees/wt_goal",
          pathExists: true,
          baseBranch: "main",
          baseSha: "abcdef123456",
          dirtySnapshot: {
            clean: false,
            stagedFiles: 1,
            unstagedFiles: 1,
            untrackedFiles: 1,
            conflictedFiles: 0,
            changedFiles: 3,
          },
          handedOffAt: "2026-01-01T00:02:00Z",
        },
        createdAt: "2026-01-01T00:02:00Z",
      },
    ],
    timeline: [],
    budget: {
      tokenLimit: null,
      timeLimitSecs: null,
      turnLimit: null,
      tokensUsed: 0,
      elapsedSecs: 120,
      turnsUsed: 0,
      tokenRatio: null,
      timeRatio: null,
      turnRatio: null,
      warning: false,
      exhausted: false,
      warnings: [],
      exceeded: [],
    },
    workflowRuns: [],
    tasks: [],
  }
}

function goalSnapshotWithDomainEvidence(): GoalSnapshot {
  return {
    goal: {
      id: "goal-domain",
      sessionId: "s1",
      objective: "Write sourced research brief",
      completionCriteria: "Source evidence is visible",
      state: "active",
      modeSnapshot: null,
      budgetTokenLimit: null,
      budgetTimeLimitSecs: null,
      budgetTurnLimit: null,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:03:00Z",
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: {},
    },
    links: [],
    events: [],
    criteria: [],
    evidence: [
      {
        id: "domain:devi_source:source_cited",
        sourceType: "domain_evidence",
        sourceId: "devi_source",
        relation: "source_cited",
        title: "Official documentation cited",
        summary: "Source supports the research brief.",
        metadata: {
          domain: "research",
          title: "Official documentation cited",
          summary: "Source supports the research brief.",
          confidence: 0.92,
          accessScope: "connector",
          redactionStatus: "sensitive",
          source: {
            title: "Official docs",
            uri: "https://example.com/docs",
            connector: "gmail",
            account: "user@example.com",
            retrievedAt: "2026-07-04T00:00:00Z",
            workflow: {
              runId: "wf-domain",
              opKey: "main/op#1(evidence.record)",
              sessionId: "s1",
              goalId: "goal-domain",
              executionMode: "guarded",
            },
          },
        },
        createdAt: "2026-01-01T00:03:00Z",
      },
    ],
    timeline: [],
    budget: {
      tokenLimit: null,
      timeLimitSecs: null,
      turnLimit: null,
      tokensUsed: 0,
      elapsedSecs: 180,
      turnsUsed: 0,
      tokenRatio: null,
      timeRatio: null,
      turnRatio: null,
      warning: false,
      exhausted: false,
      warnings: [],
      exceeded: [],
    },
    workflowRuns: [],
    tasks: [],
  }
}

function goalSnapshotWithWorkflowTemplate(): GoalSnapshot {
  const snapshot = goalSnapshotWithDomainEvidence()
  return {
    ...snapshot,
    goal: {
      ...snapshot.goal,
      id: "goal-auto",
      objective: "Keep the research brief fresh",
      completionCriteria: "A workflow loop keeps the brief reviewed",
      domain: "research",
      workflowTemplateId: "research-brief",
      workflowTemplateVersion: "1.0.0",
      workflowTaskType: "technical_research",
    },
    workflowRuns: [],
  }
}

function goalSnapshotWithClosurePacket(patch: Partial<GoalSnapshot> = {}): GoalSnapshot {
  const snapshot = goalSnapshotWithWorkflowTemplate()
  return {
    ...snapshot,
    goal: {
      ...snapshot.goal,
      revision: 3,
      state: "completed",
      completedAt: "2026-01-01T00:05:00Z",
      finalSummary: "Goal completed with durable evidence.",
      finalEvidence: {
        status: "completed",
        summary: "Goal completed with durable evidence.",
        goalRevision: 3,
        goalLinkedEventSeq: 7,
        followUpItems: [{ id: "criterion-3", text: "manual screenshot smoke" }],
      },
      blockedReason: null,
      closureDecision: null,
      closureReason: null,
      closedAt: null,
      followUpItems: [],
    },
    links: [
      {
        id: 1,
        goalId: "goal-auto",
        targetType: "loop_schedule",
        targetId: "loop-criteria",
        relation: "loop_run",
        metadata: {
          goalCriterion: {
            id: "criterion-1",
            text: "Workflow evidence is reviewed",
            kind: "required",
            goalRevision: 3,
          },
        },
        createdAt: "2026-01-01T00:04:00Z",
      },
    ],
    auditStale: false,
    criteriaItems: [
      { id: "criterion-1", text: "Workflow evidence is reviewed", kind: "required" },
      { id: "criterion-2", text: "Polish the final copy", kind: "optional" },
      { id: "criterion-3", text: "manual screenshot smoke", kind: "follow_up" },
    ],
    criteria: [
      {
        id: "criterion-1",
        text: "Workflow evidence is reviewed",
        kind: "required",
        status: "satisfied",
        evidenceIds: ["workflow:wf-criteria"],
        reason: "Workflow completed with evidence.",
      },
      {
        id: "criterion-2",
        text: "Polish the final copy",
        kind: "optional",
        status: "missing",
        evidenceIds: [],
        reason: "Optional evidence is missing.",
      },
      {
        id: "criterion-3",
        text: "manual screenshot smoke",
        kind: "follow_up",
        status: "missing",
        evidenceIds: [],
        reason: "Move to follow-up pool.",
      },
    ],
    evidence: [
      {
        id: "workflow:wf-criteria",
        sourceType: "workflow_run",
        sourceId: "wf-criteria",
        relation: "workflow_completed",
        title: "Workflow completed",
        summary: "Reviewed current evidence.",
        metadata: {
          goalCriterion: {
            id: "criterion-1",
            text: "Workflow evidence is reviewed",
            kind: "required",
            goalRevision: 3,
          },
        },
        createdAt: "2026-01-01T00:05:00Z",
      },
    ],
    workflowRuns: [
      workflowRun({
        id: "wf-criteria",
        goalId: "goal-auto",
        goalCriterionId: "criterion-1",
        goalCriterionText: "Workflow evidence is reviewed",
        goalCriterionKind: "required",
        goalRevision: 3,
        state: "completed",
        completedAt: "2026-01-01T00:05:00Z",
        updatedAt: "2026-01-01T00:05:00Z",
      }),
    ],
    ...patch,
  }
}

describe("WorkspacePanel goal section", () => {
  it("creates a goal with a selected domain workflow template", async () => {
    const template = domainWorkflowTemplate()
    const createdGoal: GoalSnapshot = {
      goal: {
        id: "goal-research",
        sessionId: "s1",
        objective: "调研新版浏览器自动化能力并整理风险",
        completionCriteria: "引用、claim check、citation audit 都齐全",
        domain: "research",
        workflowTemplateId: "research-brief",
        workflowTemplateVersion: "1.0.0",
        workflowTaskType: "technical_research",
        state: "active",
        modeSnapshot: null,
        budgetTokenLimit: null,
        budgetTimeLimitSecs: null,
        budgetTurnLimit: null,
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
        completedAt: null,
        finalSummary: null,
        finalEvidence: {},
        blockedReason: null,
        lastEvaluatorResult: {},
      },
      links: [],
      events: [],
      criteria: [],
      evidence: [],
      timeline: [],
      budget: {
        tokenLimit: null,
        timeLimitSecs: null,
        turnLimit: null,
        tokensUsed: 0,
        elapsedSecs: 0,
        turnsUsed: 0,
        tokenRatio: null,
        timeRatio: null,
        turnRatio: null,
        warning: false,
        exhausted: false,
        warnings: [],
        exceeded: [],
      },
      workflowRuns: [],
      tasks: [],
    }
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(null)
      if (name === "list_domain_workflow_templates") return Promise.resolve([template])
      if (name === "create_goal") return Promise.resolve(createdGoal)
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByText("Goal"))
    fireEvent.change(screen.getByPlaceholderText("例如：完整实现 Goal 模式，并通过针对性检查"), {
      target: { value: "调研新版浏览器自动化能力并整理风险" },
    })
    fireEvent.change(screen.getByPlaceholderText(/每行一个标准/), {
      target: { value: "引用、claim check、citation audit 都齐全" },
    })
    const domainSelect = (await screen.findAllByRole("combobox"))[0]
    fireEvent.pointerDown(domainSelect, { button: 0, ctrlKey: false, pointerType: "mouse" })
    const templateOptions = await screen.findAllByText("Research brief")
    fireEvent.click(templateOptions[templateOptions.length - 1])
    fireEvent.click(screen.getByRole("button", { name: "创建 Goal" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_goal", {
        sessionId: "s1",
        objective: "调研新版浏览器自动化能力并整理风险",
        completionCriteria: "引用、claim check、citation audit 都齐全",
        domain: "research",
        workflowTemplateId: "research-brief",
        workflowTemplateVersion: "1.0.0",
        workflowTaskType: "technical_research",
      })
    })
    expect(await screen.findByText("research")).toBeTruthy()
    expect(screen.getByText("research-brief@1.0.0")).toBeTruthy()
    expect(screen.getByText("technical_research")).toBeTruthy()
  })

  it("surfaces worktree evidence in goal detail", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorktreeEvidence())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await clickTextButton("Ship isolated worktree")

    expect(screen.getByText("Worktrees")).toBeTruthy()
    expect(screen.getByText("feature-goal")).toBeTruthy()
    expect(screen.getByText("/repo-worktrees/wt_goal")).toBeTruthy()
    expect(screen.getByText("main · abcdef12")).toBeTruthy()
    expect(screen.getByText("3 个变更")).toBeTruthy()
    expect(screen.getAllByText("handoff at /repo-worktrees/wt_goal").length).toBeGreaterThan(0)
  })

  it("surfaces domain evidence provenance in goal detail", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithDomainEvidence())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    await clickTextButton("Write sourced research brief")

    expect(screen.getByText("领域证据")).toBeTruthy()
    expect(screen.getAllByText("Official documentation cited").length).toBeGreaterThan(0)
    expect(screen.getAllByText("source_cited").length).toBeGreaterThan(0)
    expect(screen.getByText("research")).toBeTruthy()
    expect(screen.getByText("https://example.com/docs")).toBeTruthy()
    expect(screen.getByText("gmail · user@example.com")).toBeTruthy()
    expect(screen.getByText("sensitive")).toBeTruthy()
    expect(screen.getByText("导出前复核")).toBeTruthy()
    expect(screen.getByText("connector")).toBeTruthy()
    expect(screen.getByText("92%")).toBeTruthy()
    expect(screen.getByText("main/op#1(evidence.record)")).toBeTruthy()
  })

  it("shows closure packet criteria counts and accepts v1 with follow-up items", async () => {
    const snapshot = goalSnapshotWithClosurePacket()
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    })
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(snapshot)
      if (name === "list_workflow_runs") return Promise.resolve(snapshot.workflowRuns)
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "append_goal_follow_up") {
        return Promise.resolve({
          ...snapshot,
          goal: {
            ...snapshot.goal,
            followUpItems: [
              ...(snapshot.goal.followUpItems ?? []),
              { id: "followup-extra", text: "schedule stakeholder review" },
            ],
          },
        })
      }
      if (name === "close_goal") {
        return Promise.resolve({
          ...snapshot,
          goal: {
            ...snapshot.goal,
            closureDecision: "accepted_v1",
            closedAt: "2026-01-01T00:06:00Z",
          },
        })
      }
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await clickTextButton("Keep the research brief fresh")

    expect(screen.getByText("关闭取舍")).toBeTruthy()
    expect(screen.getAllByText("Workflow evidence is reviewed").length).toBeGreaterThan(0)
    expect(screen.getByText("Workflow 1")).toBeTruthy()
    expect(screen.getByText("Loop 1")).toBeTruthy()
    expect(screen.getByText("Evidence 1")).toBeTruthy()
    expect(screen.getAllByText("manual screenshot smoke").length).toBeGreaterThan(0)

    fireEvent.click(screen.getByRole("button", { name: "复制摘要" }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining("# Goal Closure Packet"))
    })

    fireEvent.change(screen.getByPlaceholderText("新增后续项"), {
      target: { value: "schedule stakeholder review" },
    })
    fireEvent.click(screen.getByRole("button", { name: "加入后续" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("append_goal_follow_up", {
        goalId: "goal-auto",
        items: ["schedule stakeholder review"],
        source: "workspace",
      })
    })

    const acceptButton = screen.getByRole("button", { name: "接受 v1 关闭" }) as HTMLButtonElement
    expect(acceptButton.disabled).toBe(false)
    fireEvent.click(acceptButton)

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("close_goal", {
        goalId: "goal-auto",
        decision: "accepted_v1",
        reason: "用户接受当前证据与剩余风险",
        followUpItems: ["manual screenshot smoke"],
      })
    })
  })

  it("keeps accepted closure disabled when the goal audit is stale", async () => {
    const snapshot = goalSnapshotWithClosurePacket({ auditStale: true })
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(snapshot)
      if (name === "list_workflow_runs") return Promise.resolve(snapshot.workflowRuns)
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await clickTextButton("Keep the research brief fresh")

    expect(screen.getByText("目标或证据已变化，需要重新评估")).toBeTruthy()
    expect((screen.getByRole("button", { name: "接受 v1 关闭" }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect(transportMock.call).not.toHaveBeenCalledWith("close_goal", expect.anything())
  })
})

describe("WorkspacePanel context retrieval section", () => {
  it("records a generated context summary as domain evidence", async () => {
    const snapshot = contextRetrievalSnapshot()
    vi.useFakeTimers()
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_context_retrieval") return Promise.resolve(snapshot)
      if (name === "record_domain_evidence") {
        const input = args?.input as Partial<DomainEvidenceItem> | undefined
        return Promise.resolve(
          domainEvidenceItem({
            title: input?.title ?? "上下文摘要：Browser automation notes",
            summary: input?.summary,
            sourceMetadata: input?.sourceMetadata ?? {},
          }),
        )
      }
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "list_workflow_runs") return Promise.resolve([])
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await flushContextRetrievalDebounce()
    vi.useRealTimers()
    await clickSectionHeader("推荐上下文")
    fireEvent.click(await screen.findByRole("button", { name: "摘要" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("record_domain_evidence", {
        input: expect.objectContaining({
          sessionId: "s1",
          domain: "research",
          evidenceType: "artifact_created",
          title: "上下文摘要：Browser automation notes",
          confidence: 0.91,
          accessScope: "project",
          redactionStatus: "none",
          sourceMetadata: expect.objectContaining({
            source: "context_retrieval",
            action: "summarize",
            artifactKind: "context_summary",
            candidateId: "doc-1",
          }),
        }),
      })
    })
  })

  it("requests user confirmation through owner ask_user", async () => {
    const snapshot = contextRetrievalSnapshot()
    vi.useFakeTimers()
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_context_retrieval") return Promise.resolve(snapshot)
      if (name === "create_owner_ask_user_question")
        return Promise.resolve({ requestId: "auq-1", sessionId: "s1", questions: [] })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "list_workflow_runs") return Promise.resolve([])
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await flushContextRetrievalDebounce()
    vi.useRealTimers()
    await clickSectionHeader("推荐上下文")
    fireEvent.click(await screen.findByRole("button", { name: "确认" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_owner_ask_user_question", {
        input: expect.objectContaining({
          sessionId: "s1",
          source: "workspace_context",
          questions: expect.arrayContaining([
            expect.objectContaining({
              questionId: "context_confirmation",
              options: expect.arrayContaining([
                expect.objectContaining({ value: "confirm" }),
                expect.objectContaining({ value: "reject" }),
              ]),
            }),
          ]),
          ownerResponse: expect.objectContaining({
            action: "record_domain_evidence",
            domainEvidence: expect.objectContaining({
              sessionId: "s1",
              domain: "research",
              evidenceType: "user_decision",
              title: "用户确认：Browser automation notes",
              sourceMetadata: expect.objectContaining({
                source: "context_retrieval",
                action: "ask_user_confirmation",
                candidateId: "doc-1",
              }),
            }),
          }),
        }),
      })
    })
  })

  it("refreshes context and task workbench when domain evidence is recorded", async () => {
    const snapshot = contextRetrievalSnapshot()
    const listeners = new Map<string, Array<(payload: unknown) => void>>()
    vi.useFakeTimers()
    transportMock.listen.mockImplementation(
      (eventName: string, handler: (payload: unknown) => void) => {
        const handlers = listeners.get(eventName) ?? []
        handlers.push(handler)
        listeners.set(eventName, handlers)
        return () => {
          const next = (listeners.get(eventName) ?? []).filter((current) => current !== handler)
          if (next.length > 0) {
            listeners.set(eventName, next)
          } else {
            listeners.delete(eventName)
          }
        }
      },
    )
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_context_retrieval") return Promise.resolve(snapshot)
      if (name === "list_domain_evidence") return Promise.resolve([])
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "list_workflow_runs") return Promise.resolve([])
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await flushContextRetrievalDebounce()
    vi.useRealTimers()
    await waitFor(() => {
      expect(
        transportMock.call.mock.calls.filter(([name]) => name === "get_context_retrieval").length,
      ).toBeGreaterThan(0)
    })
    await waitFor(() => {
      expect(listeners.get("domain_evidence:recorded")?.length ?? 0).toBeGreaterThanOrEqual(2)
    })
    const contextCallsBefore = transportMock.call.mock.calls.filter(
      ([name]) => name === "get_context_retrieval",
    ).length
    const evidenceCallsBefore = transportMock.call.mock.calls.filter(
      ([name]) => name === "list_domain_evidence",
    ).length
    const operationalCallsBefore = transportMock.call.mock.calls.filter(
      ([name]) => name === "evaluate_domain_operational_gate",
    ).length
    const soakCallsBefore = transportMock.call.mock.calls.filter(
      ([name]) => name === "generate_domain_soak_report",
    ).length

    act(() => {
      for (const handler of listeners.get("domain_evidence:recorded") ?? []) {
        handler({ sessionId: "s1", id: "evidence-2" })
      }
    })

    await waitFor(() => {
      expect(
        transportMock.call.mock.calls.filter(([name]) => name === "get_context_retrieval")
          .length,
      ).toBeGreaterThan(contextCallsBefore)
      expect(
        transportMock.call.mock.calls.filter(([name]) => name === "list_domain_evidence")
          .length,
      ).toBeGreaterThan(evidenceCallsBefore)
      expect(
        transportMock.call.mock.calls.filter(([name]) => name === "evaluate_domain_operational_gate")
          .length,
      ).toBeGreaterThan(operationalCallsBefore)
      expect(
        transportMock.call.mock.calls.filter(([name]) => name === "generate_domain_soak_report")
          .length,
      ).toBeGreaterThan(soakCallsBefore)
    })
  })
})

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

  it("does not claim a fallback working directory is non-git while environment is loading", () => {
    renderPanel(
      null,
      {
        effectiveWorkingDir: "/repo",
        workingDirSource: "session",
      },
      { loading: true },
    )

    expect(screen.getByText("状态未知")).toBeTruthy()
    expect(screen.getByText("repo")).toBeTruthy()
    expect(screen.queryByText("非 Git 工作目录")).toBeNull()
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
})

describe("WorkspacePanel domain quality section", () => {
  it("generates learning proposals from the selected domain quality run", async () => {
    const snapshot = domainQualitySnapshot()
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_domain_quality_runs") return Promise.resolve([snapshot.run])
      if (name === "get_domain_quality_run") return Promise.resolve(snapshot)
      if (name === "generate_coding_improvement_proposals") {
        return Promise.resolve({ inserted: 2, proposals: [] })
      }
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "get_coding_trend_report") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    await clickSectionHeader("领域复核")
    expect(await screen.findByText("Research quality passed")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "提炼经验" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("generate_coding_improvement_proposals", {
        sessionId: "s1",
        windowDays: 30,
        sourceType: "domain_quality",
        sourceId: "dq-1",
      })
    })
  })

  it("shows artifact evidence scope on domain quality runs", async () => {
    const snapshot = domainQualitySnapshot()
    snapshot.run.stats = {
      ...snapshot.run.stats,
      evidenceScope: {
        mode: "artifact_matched",
        total: 5,
        matched: 2,
        target: {
          title: "Research brief",
          kind: "brief",
        },
      },
    }
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_domain_quality_runs") return Promise.resolve([snapshot.run])
      if (name === "get_domain_quality_run") return Promise.resolve(snapshot)
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "get_coding_trend_report") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    await clickSectionHeader("领域复核")

    expect(await screen.findByText("产物证据")).toBeTruthy()
    expect(screen.getByText("Research brief · 2/5 条匹配")).toBeTruthy()
  })

  it("records completed artifact review evidence without bypassing export guards", async () => {
    const snapshot = domainQualitySnapshot()
    snapshot.run.stats = {
      ...snapshot.run.stats,
      artifact: {
        title: "Research brief",
        kind: "brief",
      },
      source: {
        artifactPath: "/tmp/research.md",
      },
      evidenceScope: {
        mode: "artifact_matched",
        total: 5,
        matched: 2,
        target: {
          title: "Research brief",
          kind: "brief",
          path: "/tmp/research.md",
        },
      },
    }
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_domain_quality_runs") return Promise.resolve([snapshot.run])
      if (name === "get_domain_quality_run") return Promise.resolve(snapshot)
      if (name === "record_domain_evidence") {
        const input = args?.input as RecordDomainEvidenceInput
        return Promise.resolve({
          id: "de-review",
          goalId: input.goalId ?? null,
          sessionId: input.sessionId ?? "s1",
          projectId: null,
          domain: input.domain,
          evidenceType: input.evidenceType,
          title: input.title,
          summary: input.summary ?? null,
          sourceMetadata: input.sourceMetadata ?? {},
          confidence: input.confidence ?? null,
          accessScope: input.accessScope ?? "session",
          redactionStatus: input.redactionStatus ?? "none",
          createdAt: "2026-01-01T00:05:00Z",
          updatedAt: "2026-01-01T00:05:00Z",
        } satisfies DomainEvidenceItem)
      }
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "get_coding_trend_report") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    await clickSectionHeader("领域复核")

    fireEvent.click(await screen.findByRole("button", { name: "记录复核证据" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("record_domain_evidence", {
        input: expect.objectContaining({
          sessionId: "s1",
          domain: "research",
          evidenceType: "artifact_reviewed",
          title: "复核通过：Research brief",
          summary: "Research quality passed",
          confidence: 1,
          accessScope: "session",
          redactionStatus: "none",
        }),
      })
    })

    const recordCall = transportMock.call.mock.calls.find(
      ([name]) => name === "record_domain_evidence",
    )
    expect(recordCall).toBeTruthy()
    const input = (recordCall?.[1] as { input: RecordDomainEvidenceInput }).input
    expect(input.sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "domain_quality",
        domainQualityRunId: "dq-1",
        qualityState: "completed",
        templateId: "research-brief",
        templateVersion: "1.0.0",
        artifactTitle: "Research brief",
        artifactKind: "brief",
        artifactPath: "/tmp/research.md",
        reviewCompleted: true,
        evidenceScope: expect.objectContaining({ mode: "artifact_matched" }),
      }),
    )
    const sourceMetadata = input.sourceMetadata as Record<string, unknown>
    expect(sourceMetadata.exportReview).toBeUndefined()
    expect(sourceMetadata.exportReady).toBeUndefined()
    expect(sourceMetadata.redactionChecked).toBeUndefined()
  })

  it("imports promoted domain eval proposals from the coding trend section", async () => {
    const proposal = codingImprovementProposal()
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_coding_trend_report") {
        return Promise.resolve(codingTrendReport({}, [proposal]))
      }
      if (name === "import_domain_eval_case") {
        return Promise.resolve({
          imported: true,
          task: {
            id: "learned-inbox-approval-send-guard",
            version: "1.0.0",
            domain: "inbox",
            title: "Inbox approval send guard",
            taskType: "learned_domain_quality_case",
            input: {
              prompt: "Draft and send only after approval.",
              fixtureKind: "learned_domain_quality_trace",
              sourceRequirements: [],
            },
            allowedTools: ["mail_search", "mail_draft", "mail_send"],
            requiredEvidence: [
              {
                evidenceType: "user_decision",
                title: "Explicit send approval",
                required: true,
                minCount: 1,
                metadataKeys: ["decision"],
              },
            ],
            successCriteria: ["Approval is required"],
            prohibitedActions: ["mail_send"],
            calibration: [
              {
                calibratedAt: "2026-01-01T00:04:00Z",
                reviewer: "promoted-human-reviewed",
                note: "Imported from proposal",
              },
            ],
          },
          projectId: "p1",
          sourcePath: "domain-eval/inbox/send.json",
          importedAt: "2026-01-01T00:04:00Z",
        })
      }
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: null, source: "none", exists: false, name: null },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: /质量趋势/ }))
    expect(await screen.findByText("Inbox send approval eval")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "导入评测" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("import_domain_eval_case", {
        input: { proposalId: proposal.id },
      })
    })
  })
})

describe("WorkspacePanel workflow section", () => {
  it("summarizes autonomous readiness from goal workflow and loop state", async () => {
    const run = workflowRun({
      id: "wf-loop",
      kind: "domain:research",
      state: "completed",
      origin: "loop:loop-1",
      goalId: "goal-auto",
      completedAt: "2026-01-01T00:05:00Z",
      updatedAt: "2026-01-01T00:05:00Z",
    })
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "list_loop_schedules")
        return Promise.resolve([loopSchedule({ goalId: "goal-auto" })])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "autonomous" })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("自主推进就绪")).toBeTruthy()
    expect(await screen.findByText("自主就绪")).toBeTruthy()
    expect(screen.getAllByText("Keep the research brief fresh").length).toBeGreaterThan(0)
    expect(screen.getAllByText("自主").length).toBeGreaterThan(0)
    await waitFor(() => {
      const goalCalls = transportMock.call.mock.calls.filter(([name]) => name === "get_active_goal")
      expect(goalCalls).toHaveLength(1)
    })
  })

  it("offers setup actions from the autonomous readiness card", async () => {
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "off" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "off" })
      if (name === "set_workflow_mode") return Promise.resolve({ mode: args?.mode })
      if (name === "set_execution_mode") return Promise.resolve({ mode: args?.mode })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("待配置")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "开启编排" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("set_workflow_mode", {
        sessionId: "s1",
        mode: "on",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "设为守护" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("set_execution_mode", {
        sessionId: "s1",
        mode: "guarded",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "新建 Loop" }))
    expect(await screen.findByRole("button", { name: "创建 Loop" })).toBeTruthy()
    expect(screen.getByRole("button", { name: "创建工作流" })).toBeTruthy()
  })

  it("opens failed workflow and blocked loop details from readiness actions", async () => {
    const healthyRun = workflowRun({
      id: "wf-ok",
      state: "completed",
      completedAt: "2026-01-01T00:04:00Z",
      updatedAt: "2026-01-01T00:04:00Z",
    })
    const failedRun = workflowRun({
      id: "wf-failed",
      state: "failed",
      blockedReason: "validation failed",
      updatedAt: "2026-01-01T00:05:00Z",
    })
    const blockedLoop = loopSchedule({
      id: "loop-blocked",
      state: "blocked",
      blockedReason: "approval required",
    })
    const blockedLoopSnapshot = loopSnapshot({
      schedule: blockedLoop,
      runs: loopSnapshot().runs.map((run) => ({ ...run, loopId: blockedLoop.id })),
    })
    const snapshots = new Map([
      [healthyRun.id, workflowSnapshot(healthyRun)],
      [failedRun.id, workflowSnapshot(failedRun)],
    ])
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([healthyRun, failedRun])
      if (name === "get_workflow_run") {
        return Promise.resolve(snapshots.get(String(args?.runId)) ?? null)
      }
      if (name === "list_loop_schedules") return Promise.resolve([blockedLoop])
      if (name === "get_loop_schedule") return Promise.resolve(blockedLoopSnapshot)
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect((await screen.findAllByText("需处理")).length).toBeGreaterThan(0)

    fireEvent.click(screen.getByRole("button", { name: "查看工作流" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("get_workflow_run", {
        runId: "wf-failed",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "查看 Loop" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("get_loop_schedule", {
        loopId: "loop-blocked",
      })
    })
  })

  it("opens operational and soak evidence from readiness actions", async () => {
    const writeText = vi.fn(async (_value: string) => {})
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    })
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_operational_gate")
        return Promise.resolve(domainOperationalGateReport())
      if (name === "generate_domain_soak_report") return Promise.resolve(domainSoakReport())
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect((await screen.findAllByText("需处理")).length).toBeGreaterThan(0)
    expect(screen.getByText("真实样本验收")).toBeTruthy()
    expect(screen.getByText("验收快照")).toBeTruthy()
    const visibleAcceptanceSnapshotId = screen.getByText(/acc-[0-9a-f]{8}/).textContent ?? ""
    expect(visibleAcceptanceSnapshotId).toMatch(/acc-[0-9a-f]{8}/)
    expect(screen.getByText("样本有事故")).toBeTruthy()
    expect(screen.getByText("证据等级")).toBeTruthy()
    expect(screen.getByText("阻塞样本")).toBeTruthy()
    expect(screen.getByText("仍有阻塞缺口或失败守门，不能作为验收证据。")).toBeTruthy()
    expect(screen.getByText("来源分布")).toBeTruthy()
    expect(
      screen.getByText(
        "evidence 0 · workflow 0 · connector 0 · fixture/mock 0 · manual 0 · public 0 · restricted 0",
      ),
    ).toBeTruthy()
    expect(screen.getByText("控制面组成")).toBeTruthy()
    expect(screen.getByText("workflow 1/2 · loop 1/1 · campaign 0/0 · connector 0")).toBeTruthy()
    expect(screen.getByText("验收结论")).toBeTruthy()
    expect(screen.getByText("不可验收")).toBeTruthy()
    expect(screen.getByText("34% · 3/8")).toBeTruthy()
    expect(screen.getByText("证据链")).toBeTruthy()
    expect(screen.getByText("缺来源/草稿/决策证据")).toBeTruthy()
    expect(screen.getByText("样本新鲜")).toBeTruthy()
    expect(screen.getByText("2m 前")).toBeTruthy()
    expect(screen.getByText("跨天覆盖")).toBeTruthy()
    expect(screen.getByText("1/2 天，缺跨天样本")).toBeTruthy()
    expect(screen.getByText("预算健康")).toBeTruthy()
    expect(screen.getByText("耗尽 1 次 · 10/10")).toBeTruthy()
    expect(screen.getByText("守门通过")).toBeTruthy()
    expect(screen.getByText("运行稳定性、长跑审计 未通过")).toBeTruthy()
    expect(screen.getByText("3 条")).toBeTruthy()
    expect(screen.getByText("验收矩阵")).toBeTruthy()
    expect(screen.getByText("Campaign 样本")).toBeTruthy()
    expect(screen.getByText("缺通过的 Campaign item")).toBeTruthy()
    expect(screen.getAllByText("长跑审计仍有事故需要收口。").length).toBeGreaterThan(0)

    fireEvent.click(screen.getByRole("button", { name: "复制验收报告" }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining("# 真实样本验收"))
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining("预算健康：耗尽 1 次 · 10/10"))
      expect(writeText).toHaveBeenCalledWith(
        expect.stringContaining("工作流输出预算已耗尽，需收口性能或上下文策略。"),
      )
    })
    const acceptanceReport = String(writeText.mock.calls[0]?.[0] ?? "")
    expect(acceptanceReport).toContain("## 审计索引")
    expect(acceptanceReport).toContain(`快照 ID：${visibleAcceptanceSnapshotId}`)
    expect(acceptanceReport).toContain(
      "证据等级：阻塞样本 - 仍有阻塞缺口或失败守门，不能作为验收证据。",
    )
    expect(acceptanceReport).toContain(
      "来源分布：evidence 0 · workflow 0 · connector 0 · fixture/mock 0 · manual 0 · public 0 · restricted 0",
    )
    expect(acceptanceReport).toContain(
      "控制面组成：workflow 1/2 · loop 1/1 · campaign 0/0 · connector 0",
    )
    expect(acceptanceReport).toContain("跨天覆盖：1/2 天")
    expect(acceptanceReport).toContain("连接器 E2E evidence：0（执行 0 / 复核 0）")
    expect(acceptanceReport).toContain(
      "Gate 快照：export=missing · connector=missing · e2e=missing · operational=failed · soak=failed",
    )
    expect(acceptanceReport).toContain("Evidence IDs：无")
    expect(acceptanceReport).toContain("## 复核协议")
    expect(acceptanceReport).toContain(
      "只有验收结论为“可验收”时，当前样本才可作为最终验收证据",
    )
    expect(acceptanceReport).toContain("连接器 E2E 必须来自测试账号或沙箱数据")
    expect(acceptanceReport).toContain("## 守门状态")
    expect(acceptanceReport).toContain("验收结论：不可验收 - 长跑审计仍有事故需要收口。")
    expect(acceptanceReport).toContain("交付守门：未评估")
    expect(acceptanceReport).toContain("运行稳定性：阻塞 (failed)")
    expect(acceptanceReport).toContain("workflow_failed_residue=failed")
    expect(acceptanceReport).toContain("## 最近证据")
    expect(acceptanceReport).toContain("暂无 evidence")
    expect(acceptanceReport).toContain("## 长跑审计")
    expect(acceptanceReport).toContain("事故：Workflow failed · workflow/failed/critical")
    expect(acceptanceReport).toContain("时间线：Workflow failed · workflow/failed · 4m")
    expect(acceptanceReport).toContain("## 推荐下一步")
    expect(acceptanceReport).toContain("Repair the failed workflow.")
    expect(acceptanceReport).toContain("## 验收矩阵")
    expect(acceptanceReport).toContain("[待补] Campaign 样本：缺通过的 Campaign item")
    expect(acceptanceReport).toContain("证据：Campaign item 使用 deterministic trace pack 或真实 agent 样本。")
    expect(acceptanceReport).toContain("[待扩展] 连接器 E2E：当前会话未观察外部动作")
    expect(acceptanceReport).toContain("证据：使用测试账号或沙箱数据，避免真实用户生产账号。")

    const evidenceRequirement = screen.getByText("缺来源/草稿/决策证据")
    const evidenceRequirementRow = evidenceRequirement.parentElement
    expect(evidenceRequirementRow).toBeTruthy()
    fireEvent.click(
      within(evidenceRequirementRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "补齐真实样本验收必需项：证据链（缺来源/草稿/决策证据）",
        activeForm: "正在补齐真实样本验收必需项",
      })
    })

    const budgetRequirement = screen.getByText("耗尽 1 次 · 10/10")
    const budgetRequirementRow = budgetRequirement.parentElement
    expect(budgetRequirementRow).toBeTruthy()
    fireEvent.click(
      within(budgetRequirementRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "补齐真实样本验收必需项：预算健康（耗尽 1 次 · 10/10）",
        activeForm: "正在补齐真实样本验收必需项",
      })
    })

    const campaignLane = screen.getByText("缺通过的 Campaign item")
    const campaignLaneRow = campaignLane.parentElement?.parentElement
    expect(campaignLaneRow).toBeTruthy()
    fireEvent.click(
      within(campaignLaneRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "补齐真实样本验收跑道：Campaign 样本\n\n当前状态：\n- 缺通过的 Campaign item\n\n采样动作：\n- 跑一个 deterministic 或真实 agent campaign item，确认可取消、可 retry、可复核。\n\n需要记录的证据：\n- Campaign item 使用 deterministic trace pack 或真实 agent 样本。\n- 至少一个 item passed，失败 item 有分类和 retry / cancel 证据。\n- 保留 campaign summary，能追溯输入、判断标准和输出。\n\n完成后刷新：\n- 刷新运行稳定性 Gate。\n- 刷新长跑审计 Soak Report。\n- 重新复制真实样本验收报告。",
        activeForm: "正在补齐真实样本验收跑道",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "采样清单" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: expect.stringContaining("补齐真实样本验收清单："),
        activeForm: "正在补齐真实样本验收清单",
      })
    })
    const acceptancePlanTask = transportMock.call.mock.calls.find(
      ([name, args]) =>
        name === "create_session_task" &&
        String(args?.activeForm ?? "") === "正在补齐真实样本验收清单",
    )?.[1]
    const acceptancePlanContent = String(acceptancePlanTask?.content ?? "")
    expect(acceptancePlanContent).toContain("审计索引：")
    expect(acceptancePlanContent).toContain(`快照 ID：${visibleAcceptanceSnapshotId}`)
    expect(acceptancePlanContent).toContain(
      "证据等级：阻塞样本 - 仍有阻塞缺口或失败守门，不能作为验收证据。",
    )
    expect(acceptancePlanContent).toContain(
      "来源分布：evidence 0 · workflow 0 · connector 0 · fixture/mock 0 · manual 0 · public 0 · restricted 0",
    )
    expect(acceptancePlanContent).toContain(
      "控制面组成：workflow 1/2 · loop 1/1 · campaign 0/0 · connector 0",
    )
    expect(acceptancePlanContent).toContain("控制面：记录 3 · 已排空 2 · Connector E2E 0")
    expect(acceptancePlanContent).toContain("跨天覆盖：1/2 天")
    expect(acceptancePlanContent).toContain("连接器复核：执行 0 · 复核 0")
    expect(acceptancePlanContent).toContain("复核协议：")
    expect(acceptancePlanContent).toContain("转任务、按钮点击或人工声明不能替代真实 evidence")
    expect(acceptancePlanContent).toContain("验收结论：不可验收 - 长跑审计仍有事故需要收口。")
    expect(acceptancePlanContent).toContain("验收进度：34% (3/8)")
    expect(acceptancePlanContent).toContain("[待补] Campaign 样本：缺通过的 Campaign item")
    expect(acceptancePlanContent).toContain("证据：至少一个 item passed，失败 item 有分类和 retry / cancel 证据。")
    expect(acceptancePlanContent).toContain("刷新：刷新长跑审计 Soak Report。")
    expect(acceptancePlanContent).toContain("证据：使用测试账号或沙箱数据，避免真实用户生产账号。")

    const acceptanceGapRow = screen
      .getAllByText("长跑审计仍有事故需要收口。")
      .map((node) => node.parentElement)
      .find((row) => row && within(row).queryByRole("button", { name: "转任务" }))
    expect(acceptanceGapRow).toBeTruthy()
    fireEvent.click(within(acceptanceGapRow as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "补齐真实样本验收缺口：长跑审计仍有事故需要收口。",
        activeForm: "正在补齐真实样本验收缺口",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "查看稳定性" }))
    expect(await screen.findByText("运行稳定性")).toBeTruthy()
    expect(screen.getByText("workflow_failed_residue")).toBeTruthy()
    expect(screen.getAllByText("最长").length).toBeGreaterThan(0)
    expect(screen.getByText("稳定性建议")).toBeTruthy()
    const operationalRecommendation = screen
      .getAllByText("Open the failed workflow and repair it.")
      .find((element) =>
        String(element.parentElement?.parentElement?.className ?? "").includes("border-sky-500"),
      )
    expect(operationalRecommendation).toBeTruthy()
    if (!operationalRecommendation) throw new Error("missing operational recommendation row")
    const operationalRecommendationRow = operationalRecommendation.parentElement
    expect(operationalRecommendationRow).toBeTruthy()
    fireEvent.click(
      within(operationalRecommendationRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "处理运行稳定性建议：Open the failed workflow and repair it.",
        activeForm: "正在处理运行稳定性建议",
      })
    })
    const operationalCheck = screen.getByText("workflow_failed_residue")
    const operationalCheckRow = operationalCheck.parentElement
    expect(operationalCheckRow).toBeTruthy()
    fireEvent.click(
      within(operationalCheckRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "处理运行稳定性缺口：workflow_failed_residue（1）- A workflow run failed in the active window.",
        activeForm: "正在处理运行稳定性缺口：workflow_failed_residue",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "查看长跑" }))
    expect(await screen.findByText("长跑审计")).toBeTruthy()
    expect(screen.getByText("最近时间线")).toBeTruthy()
    expect(screen.getAllByText("Workflow failed").length).toBeGreaterThan(1)
    expect(screen.getAllByText("4m").length).toBeGreaterThan(1)
    expect(screen.getAllByText("2m").length).toBeGreaterThan(0)
    expect(screen.getByText("新鲜")).toBeTruthy()
    expect(screen.getByText("干预")).toBeTruthy()
    expect(screen.getByText("Token")).toBeTruthy()
    expect(screen.getByText("10/10")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "复制报告" }))
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith("Workflow failed")
    })
  })

  it("flags stale real sample freshness in acceptance coverage", async () => {
    const staleSoak = domainSoakReport()
    staleSoak.status = "insufficient_data"
    staleSoak.summary = {
      ...staleSoak.summary,
      latestActivityAgeSecs: 8 * 24 * 60 * 60,
      criticalIncidents: 0,
      warningIncidents: 0,
      incidents: 0,
    }
    staleSoak.incidents = []
    staleSoak.timeline = []
    staleSoak.recommendedNextSteps = []

    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_operational_gate")
        return Promise.resolve(
          domainOperationalGateReport({
            status: "passed",
            checks: [],
            blockers: [],
            recommendedNextSteps: [],
          }),
        )
      if (name === "generate_domain_soak_report") return Promise.resolve(staleSoak)
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("真实样本验收")).toBeTruthy()
    expect(screen.getByText("样本新鲜")).toBeTruthy()
    expect(screen.getByText("8d 前，超过 24h")).toBeTruthy()
    expect(screen.getByText("最近长任务样本过旧或缺少新鲜度信号。")).toBeTruthy()

    const freshnessRequirement = screen.getByText("8d 前，超过 24h")
    const freshnessRequirementRow = freshnessRequirement.parentElement
    expect(freshnessRequirementRow).toBeTruthy()
    fireEvent.click(
      within(freshnessRequirementRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "补齐真实样本验收必需项：样本新鲜（8d 前，超过 24h）",
        activeForm: "正在补齐真实样本验收必需项",
      })
    })
  })

  it("includes recent evidence provenance in copied acceptance review packets", async () => {
    const writeText = vi.fn(async (_value: string) => {})
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    })
    const cleanOperationalGate = domainOperationalGateReport({
      status: "passed",
      checks: [],
      blockers: [],
      recommendedNextSteps: [],
    })
    const cleanSoakReport = domainSoakReport({
      status: "passed",
      summary: {
        ...domainSoakReport().summary,
        criticalIncidents: 0,
        warningIncidents: 0,
        incidents: 0,
        workflowBudgetExhaustedEvents: 0,
        sampleDays: 2,
        requiredSampleDays: 2,
      },
      incidents: [],
      timeline: [],
      recommendedNextSteps: [],
      operationalGate: cleanOperationalGate,
    })
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_domain_evidence")
        return Promise.resolve([
          domainEvidenceItem({
            id: "e-source",
            evidenceType: "source_cited",
            title: "Primary source reviewed",
            accessScope: "public",
            redactionStatus: "none",
          }),
        ])
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(cleanOperationalGate)
      if (name === "generate_domain_soak_report") return Promise.resolve(cleanSoakReport)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("Primary source reviewed")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "复制验收报告" }))

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining("Primary source reviewed"))
    })
    const acceptanceReport = String(writeText.mock.calls[0]?.[0] ?? "")
    expect(acceptanceReport).toMatch(/快照 ID：acc-[0-9a-f]{8}/)
    expect(acceptanceReport).toContain(
      "证据等级：局部验收 - 必需项已通过，但覆盖面仍窄；不能代表全量通用能力。",
    )
    expect(acceptanceReport).toContain(
      "来源分布：evidence 1 · workflow 0 · connector 0 · fixture/mock 0 · manual 0 · public 1 · restricted 0",
    )
    expect(acceptanceReport).toContain(
      "控制面组成：workflow 1/2 · loop 1/1 · campaign 0/0 · connector 0",
    )
    expect(acceptanceReport).toContain("source_cited · research · public/none")
    expect(acceptanceReport).toContain("(e-source)")

    fireEvent.click(screen.getByRole("button", { name: "采样清单" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: expect.stringContaining("补齐真实样本验收清单："),
        activeForm: "正在补齐真实样本验收清单",
      })
    })
    const acceptancePlanTask = transportMock.call.mock.calls.find(
      ([name, args]) =>
        name === "create_session_task" &&
        String(args?.activeForm ?? "") === "正在补齐真实样本验收清单",
    )?.[1]
    const acceptancePlanContent = String(acceptancePlanTask?.content ?? "")
    expect(acceptancePlanContent).toMatch(/快照 ID：acc-[0-9a-f]{8}/)
    expect(acceptancePlanContent).toContain(
      "证据等级：局部验收 - 必需项已通过，但覆盖面仍窄；不能代表全量通用能力。",
    )
    expect(acceptancePlanContent).toContain(
      "来源分布：evidence 1 · workflow 0 · connector 0 · fixture/mock 0 · manual 0 · public 1 · restricted 0",
    )
    expect(acceptancePlanContent).toContain(
      "控制面组成：workflow 1/2 · loop 1/1 · campaign 0/0 · connector 0",
    )
    expect(acceptancePlanContent).toContain("验收结论：可局部复核")
    expect(acceptancePlanContent).toContain("继续补其它通用领域样本，避免只证明单一场景。")
  })

  it("creates a task from domain soak incidents", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "generate_domain_soak_report") return Promise.resolve(domainSoakReport())
      if (name === "evaluate_domain_operational_gate")
        return Promise.resolve(domainOperationalGateReport())
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    await screen.findByText("长跑审计")
    const incidentTitle = screen
      .getAllByText("Workflow failed")
      .find((element) => element.className.includes("font-medium"))
    expect(incidentTitle).toBeTruthy()
    if (!incidentTitle) throw new Error("missing soak incident row")
    const incidentRow = incidentTitle.parentElement
    expect(incidentRow).toBeTruthy()
    fireEvent.click(within(incidentRow as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "处理长跑事故：Workflow failed（workflow/failed）- Repair the workflow before continuing the loop.",
        activeForm: "正在处理长跑事故：Workflow failed",
      })
    })

    const recommendation = screen
      .getAllByText("Repair the failed workflow.")
      .find((element) => element.tagName.toLowerCase() === "span")
    expect(recommendation).toBeTruthy()
    if (!recommendation) throw new Error("missing soak recommendation row")
    const recommendationRow = recommendation.parentElement
    expect(recommendationRow).toBeTruthy()
    fireEvent.click(
      within(recommendationRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "处理长跑审计建议：Repair the failed workflow.",
        activeForm: "正在处理长跑审计建议",
      })
    })
  })

  it("keeps the acceptance gate requirement pending until a gate is observed", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("真实样本验收")).toBeTruthy()
    expect(screen.getByText("0% · 0/8")).toBeTruthy()
    expect(screen.getByText("守门通过")).toBeTruthy()
    expect(screen.getByText("缺少守门通过样本")).toBeTruthy()
  })

  it("opens export and connector guard evidence from readiness actions", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_artifact_export_guard")
        return Promise.resolve(domainArtifactExportGuardReport())
      if (name === "evaluate_domain_connector_action_guard")
        return Promise.resolve(domainConnectorActionGuardReport())
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect((await screen.findAllByText("需处理")).length).toBeGreaterThan(0)
    expect(screen.getByText("2 个守门")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "查看交付" }))
    expect(await screen.findByText("交付守门")).toBeTruthy()
    expect(screen.getByText("artifact_reviewed")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "查看外部动作" }))
    expect(await screen.findByText("外部动作守门")).toBeTruthy()
    expect(screen.getByText("explicit_user_approval")).toBeTruthy()
  })

  it("runs artifact-scoped domain quality from export guard", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_artifact_export_guard")
        return Promise.resolve(domainArtifactExportGuardReport())
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "run_domain_quality") return Promise.resolve(domainQualitySnapshot())
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    const reviewButtons = await screen.findAllByRole("button", { name: "复核产物" })
    fireEvent.click(reviewButtons[0])

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("run_domain_quality", {
        input: {
          sessionId: "s1",
          domain: "research",
          artifactTitle: "Research brief",
          artifactKind: "brief",
          sourceMetadata: {
            sourceType: "artifact_export_guard",
            artifactPath: null,
            artifactTitle: "Research brief",
            artifactKind: "brief",
            artifactGuardStatus: "failed",
          },
        },
      })
    })
  })

  it("records explicit export review evidence from the export guard", async () => {
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_artifact_export_guard")
        return Promise.resolve(domainArtifactExportGuardReport())
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "record_domain_evidence") {
        const input = args?.input as RecordDomainEvidenceInput
        return Promise.resolve({
          id: "de-export-review",
          goalId: input.goalId ?? null,
          sessionId: input.sessionId ?? "s1",
          projectId: input.projectId ?? null,
          domain: input.domain,
          evidenceType: input.evidenceType,
          title: input.title,
          summary: input.summary ?? null,
          sourceMetadata: input.sourceMetadata ?? {},
          confidence: input.confidence ?? null,
          accessScope: input.accessScope ?? "session",
          redactionStatus: input.redactionStatus ?? "none",
          createdAt: "2026-01-01T00:10:00Z",
          updatedAt: "2026-01-01T00:10:00Z",
        } satisfies DomainEvidenceItem)
      }
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    const exportReviewButtons = await screen.findAllByRole("button", { name: "导出复核" })
    fireEvent.click(exportReviewButtons[0])

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("record_domain_evidence", {
        input: expect.objectContaining({
          goalId: null,
          sessionId: "s1",
          projectId: null,
          domain: "research",
          evidenceType: "artifact_reviewed",
          title: "导出复核：Research brief",
          summary: "用户确认已完成最终交付复核。",
          confidence: 1,
          accessScope: "session",
          redactionStatus: "none",
        }),
      })
    })

    const recordCall = transportMock.call.mock.calls.find(
      ([name]) => name === "record_domain_evidence",
    )
    expect(recordCall).toBeTruthy()
    const input = (recordCall?.[1] as { input: RecordDomainEvidenceInput }).input
    expect(input.sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "artifact_export_guard_confirmation",
        marker: "exportReview",
        exportReview: true,
        guardStatus: "failed",
        guardGeneratedAt: "2026-01-01T00:07:00Z",
        artifactTitle: "Research brief",
        artifactKind: "brief",
        reviewedEvidenceIds: ["e-sensitive"],
        reviewReasons: ["sensitive_unreviewed"],
        blockers: ["artifact review missing"],
        export: { reviewed: true },
      }),
    )
    const sourceMetadata = input.sourceMetadata as Record<string, unknown>
    expect(sourceMetadata.exportReady).toBeUndefined()
    expect(sourceMetadata.redactionChecked).toBeUndefined()
  })

  it("records connector approval and rollback evidence without mixing markers", async () => {
    const rollbackPlan = "Undo by deleting the sent draft and sending a correction note."
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard")
        return Promise.resolve(domainConnectorActionGuardReport())
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "record_domain_evidence") {
        const input = args?.input as RecordDomainEvidenceInput
        return Promise.resolve({
          id: `de-${input.evidenceType}`,
          goalId: input.goalId ?? null,
          sessionId: input.sessionId ?? "s1",
          projectId: input.projectId ?? null,
          domain: input.domain,
          evidenceType: input.evidenceType,
          title: input.title,
          summary: input.summary ?? null,
          sourceMetadata: input.sourceMetadata ?? {},
          confidence: input.confidence ?? null,
          accessScope: input.accessScope ?? "session",
          redactionStatus: input.redactionStatus ?? "none",
          createdAt: "2026-01-01T00:11:00Z",
          updatedAt: "2026-01-01T00:11:00Z",
        } satisfies DomainEvidenceItem)
      }
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    const approveButtons = await screen.findAllByRole("button", { name: "批准动作" })
    fireEvent.click(approveButtons[0])

    await waitFor(() => {
      const calls = transportMock.call.mock.calls.filter(([name]) => name === "record_domain_evidence")
      expect(calls).toHaveLength(1)
    })

    const rollbackInputs = screen.getAllByPlaceholderText("回滚方案")
    fireEvent.change(rollbackInputs[0], { target: { value: rollbackPlan } })
    const rollbackButtons = screen.getAllByRole("button", { name: "记录回滚" })
    fireEvent.click(rollbackButtons[0])

    await waitFor(() => {
      const calls = transportMock.call.mock.calls.filter(([name]) => name === "record_domain_evidence")
      expect(calls).toHaveLength(2)
    })

    const evidenceCalls = transportMock.call.mock.calls
      .filter(([name]) => name === "record_domain_evidence")
      .map(([, args]) => (args as { input: RecordDomainEvidenceInput }).input)

    expect(evidenceCalls[0]).toEqual(
      expect.objectContaining({
        sessionId: "s1",
        domain: "inbox",
        evidenceType: "user_decision",
        title: "批准外部动作：gmail:send",
        summary: "用户确认该外部动作可以进入执行前审批流程；真正执行仍需工具审批。",
        confidence: 1,
        accessScope: "session",
        redactionStatus: "none",
      }),
    )
    expect(evidenceCalls[0].sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "connector_action_guard_confirmation",
        marker: "explicitUserApproval",
        explicitUserApproval: true,
        userApproved: true,
        approved: true,
        connector: "gmail",
        action: "send",
        toolName: "gmail_send",
        risk: "external_write",
        guardStatus: "failed",
        relatedEvidenceIds: ["e-draft"],
        blockers: ["approval missing"],
        approval: { explicit: true, approved: true },
        decision: { approved: true, confirmed: true },
      }),
    )
    expect((evidenceCalls[0].sourceMetadata as Record<string, unknown>).rollbackPlan).toBeUndefined()

    expect(evidenceCalls[1]).toEqual(
      expect.objectContaining({
        sessionId: "s1",
        domain: "inbox",
        evidenceType: "connector_context_collected",
        title: "回滚方案：gmail:send",
        summary: rollbackPlan,
        confidence: 1,
        accessScope: "session",
        redactionStatus: "none",
      }),
    )
    expect(evidenceCalls[1].sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "connector_action_guard_confirmation",
        marker: "rollbackPlan",
        connector: "gmail",
        action: "send",
        toolName: "gmail_send",
        rollbackPlan,
        canRollback: true,
        rollback: { available: true, plan: rollbackPlan },
      }),
    )
    expect(
      (evidenceCalls[1].sourceMetadata as Record<string, unknown>).explicitUserApproval,
    ).toBeUndefined()
  })

  it("creates tasks from export guard evidence and connector guard checks", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_artifact_export_guard")
        return Promise.resolve(domainArtifactExportGuardReport())
      if (name === "evaluate_domain_connector_action_guard")
        return Promise.resolve(domainConnectorActionGuardReport())
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    const exportCheckLabel = (await screen.findAllByText("artifact_reviewed"))[0]
    const exportCheckRow = exportCheckLabel.parentElement
    expect(exportCheckRow).toBeTruthy()
    fireEvent.click(within(exportCheckRow as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "处理交付守门缺口：artifact_reviewed（0）- The delivery artifact has not been reviewed.",
        activeForm: "正在处理交付守门缺口：artifact_reviewed",
      })
    })

    const exportEvidenceLabel = await screen.findByText("Private source")
    const exportEvidenceRow = exportEvidenceLabel.parentElement
    expect(exportEvidenceRow).toBeTruthy()
    fireEvent.click(within(exportEvidenceRow as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "复核交付证据：Private source（sensitive_unreviewed）- connector / pending",
        activeForm: "正在复核交付证据：Private source",
      })
    })

    const connectorCheckLabel = (await screen.findAllByText("explicit_user_approval"))[0]
    const connectorCheckRow = connectorCheckLabel.parentElement
    expect(connectorCheckRow).toBeTruthy()
    fireEvent.click(
      within(connectorCheckRow as HTMLElement).getByRole("button", { name: "转任务" }),
    )

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "处理外部动作守门缺口：explicit_user_approval（0）- No explicit user approval evidence exists for the external action.",
        activeForm: "正在处理外部动作守门缺口：explicit_user_approval",
      })
    })
  })

  it("surfaces connector E2E gate evidence from readiness actions", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_connector_e2e_gate")
        return Promise.resolve(domainConnectorE2EGateReport())
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect((await screen.findAllByText("连接器 E2E")).length).toBeGreaterThan(0)
    expect(screen.getByText("execution_result")).toBeTruthy()
    expect(screen.getByText("post_action_verification")).toBeTruthy()
    expect(screen.getByText("外部动作还缺端到端执行与复核样本。")).toBeTruthy()

    const e2eCheckLabel = screen.getByText("execution_result")
    const e2eCheckRow = e2eCheckLabel.parentElement
    expect(e2eCheckRow).toBeTruthy()
    fireEvent.click(within(e2eCheckRow as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content:
          "处理连接器 E2E 缺口：execution_result（0）- The connector action has not produced an execution result yet.",
        activeForm: "正在处理连接器 E2E 缺口：execution_result",
      })
    })

    fireEvent.click(screen.getByRole("button", { name: "查看 E2E" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("evaluate_domain_connector_e2e_gate", {
        input: {
          sessionId: "s1",
          requireConnectorInput: true,
          requireDraft: true,
          requireExplicitApproval: true,
          requireExecutionResult: true,
          requirePostActionVerification: true,
          requireRollbackPlan: true,
          requireExportGuardForDelivery: true,
        },
      })
    })
  })

  it("records connector E2E execution and verification evidence", async () => {
    const executionResult = "Message sent successfully; provider result id msg-123."
    const verificationResult = "Read Gmail thread back and confirmed the reply is visible."
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "evaluate_domain_connector_e2e_gate")
        return Promise.resolve(domainConnectorE2EGateReport())
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "record_domain_evidence") {
        const input = args?.input as RecordDomainEvidenceInput
        return Promise.resolve({
          id: `de-${input.evidenceType}`,
          goalId: input.goalId ?? null,
          sessionId: input.sessionId ?? "s1",
          projectId: input.projectId ?? null,
          domain: input.domain,
          evidenceType: input.evidenceType,
          title: input.title,
          summary: input.summary ?? null,
          sourceMetadata: input.sourceMetadata ?? {},
          confidence: input.confidence ?? null,
          accessScope: input.accessScope ?? "session",
          redactionStatus: input.redactionStatus ?? "none",
          createdAt: "2026-01-01T00:12:00Z",
          updatedAt: "2026-01-01T00:12:00Z",
        } satisfies DomainEvidenceItem)
      }
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect((await screen.findAllByText("连接器 E2E")).length).toBeGreaterThan(0)
    expect(screen.getByText("下一步：记录执行结果")).toBeTruthy()
    expect(
      (screen.getByRole("button", { name: "记录复核" }) as HTMLButtonElement).disabled,
    ).toBe(true)

    fireEvent.change(screen.getByPlaceholderText("执行结果"), {
      target: { value: executionResult },
    })
    fireEvent.click(screen.getByRole("button", { name: "记录执行" }))

    await waitFor(() => {
      const calls = transportMock.call.mock.calls.filter(
        ([name]) => name === "record_domain_evidence",
      )
      expect(calls).toHaveLength(1)
    })
    expect(await screen.findByText("下一步：记录执行后复核")).toBeTruthy()

    fireEvent.change(screen.getByPlaceholderText("执行后复核"), {
      target: { value: verificationResult },
    })
    fireEvent.click(screen.getByRole("button", { name: "记录复核" }))

    await waitFor(() => {
      const calls = transportMock.call.mock.calls.filter(
        ([name]) => name === "record_domain_evidence",
      )
      expect(calls).toHaveLength(2)
    })

    const evidenceCalls = transportMock.call.mock.calls
      .filter(([name]) => name === "record_domain_evidence")
      .map(([, args]) => (args as { input: RecordDomainEvidenceInput }).input)

    expect(evidenceCalls[0]).toEqual(
      expect.objectContaining({
        sessionId: "s1",
        domain: "inbox",
        evidenceType: "connector_action_executed",
        title: "执行结果：gmail:send",
        summary: executionResult,
        confidence: 1,
        accessScope: "session",
        redactionStatus: "none",
      }),
    )
    expect(evidenceCalls[0].sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "connector_e2e_gate_sample",
        marker: "action_execution",
        connector: "gmail",
        action: "send",
        toolName: "gmail_send",
        risk: "external_write",
        gateStatus: "insufficient_data",
        relatedEvidenceIds: ["e-connector-input"],
        actionExecuted: true,
        executed: true,
        execution: { status: "recorded", summary: executionResult },
        result: { status: "recorded", summary: executionResult },
      }),
    )
    expect(
      (evidenceCalls[0].sourceMetadata as Record<string, unknown>).postActionVerification,
    ).toBeUndefined()

    expect(evidenceCalls[1]).toEqual(
      expect.objectContaining({
        sessionId: "s1",
        domain: "inbox",
        evidenceType: "connector_action_verified",
        title: "执行后复核：gmail:send",
        summary: verificationResult,
        confidence: 1,
        accessScope: "session",
        redactionStatus: "none",
      }),
    )
    expect(evidenceCalls[1].sourceMetadata).toEqual(
      expect.objectContaining({
        sourceType: "connector_e2e_gate_sample",
        marker: "post_action_verification",
        connector: "gmail",
        action: "send",
        toolName: "gmail_send",
        postActionVerification: true,
        externalStateVerified: true,
        deliveryVerified: true,
        verification: { passed: true, verified: true, summary: verificationResult },
      }),
    )
    expect((evidenceCalls[1].sourceMetadata as Record<string, unknown>).actionExecuted).toBeUndefined()
  })

  it("creates a task from domain workbench next-step gaps", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "on" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "list_domain_evidence") return Promise.resolve([])
      if (name === "evaluate_domain_artifact_export_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_action_guard") return Promise.resolve(null)
      if (name === "evaluate_domain_connector_e2e_gate") return Promise.resolve(null)
      if (name === "evaluate_domain_operational_gate") return Promise.resolve(null)
      if (name === "generate_domain_soak_report") return Promise.resolve(null)
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    const taskButtons = await screen.findAllByRole("button", { name: "转任务" })
    fireEvent.click(taskButtons[0])

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_session_task", {
        sessionId: "s1",
        content: "先让模型记录来源、草稿或决策证据。",
        activeForm: "正在处理通用任务缺口：先让模型记录来源、草稿或决策证据。",
      })
    })
  })

  it("links workflow loop rows to their derived workflow run", async () => {
    const otherRun = workflowRun({
      id: "wf-other",
      kind: "coding.other",
      state: "completed",
      updatedAt: "2026-01-01T00:05:00Z",
    })
    const derivedRun = workflowRun({
      id: "wf-loop",
      kind: "domain:research",
      state: "running",
      origin: "loop:loop-1",
      updatedAt: "2026-01-01T00:04:00Z",
    })
    const snapshots = new Map([
      [otherRun.id, workflowSnapshot(otherRun)],
      [derivedRun.id, workflowSnapshot(derivedRun)],
    ])
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([otherRun, derivedRun])
      if (name === "get_workflow_run") {
        return Promise.resolve(snapshots.get(String(args?.runId)) ?? null)
      }
      if (name === "list_loop_schedules") return Promise.resolve([loopSchedule()])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("domain:research")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Loop/ }))
    fireEvent.click(screen.getByRole("button", { name: "查看工作流" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("get_workflow_run", { runId: "wf-loop" })
    })
  })

  it("expands loop run history with workflow trace context", async () => {
    const snapshot = loopSnapshot()
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([snapshot.schedule])
      if (name === "get_loop_schedule") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: /Loop/ }))
    expect(await screen.findByText("Update the report")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "运行记录" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("get_loop_schedule", { loopId: "loop-1" })
    })
    expect(await screen.findByText("最近运行")).toBeTruthy()
    expect(screen.getByText("#1")).toBeTruthy()
    expect(screen.getByText("成功")).toBeTruthy()
    expect(screen.getByText("research-brief@1.0.0")).toBeTruthy()
    expect(screen.getByText("workflow launched")).toBeTruthy()
  })

  it("supports Loop Center view more run-now and policy editing", async () => {
    const schedules = Array.from({ length: 6 }, (_, index) =>
      loopSchedule({
        id: `loop-${index + 1}`,
        cronJobId: `cron-loop-${index + 1}`,
        prompt: `Loop prompt ${index + 1}`,
        updatedAt: `2026-01-01T00:0${index}:00Z`,
      }),
    )
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve(schedules)
      if (name === "get_loop_schedule") {
        const schedule = schedules.find((item) => item.id === args?.loopId) ?? schedules[0]
        return Promise.resolve(loopSnapshot({ schedule }))
      }
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? {})
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: /Loop/ }))
    expect(await screen.findByText("Loop prompt 6")).toBeTruthy()
    expect(screen.queryByText("Loop prompt 1")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: /查看更多 Loop/ }))
    expect(await screen.findByText("Loop prompt 1")).toBeTruthy()

    fireEvent.click(screen.getAllByRole("button", { name: "立即运行" })[0])
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("run_loop_schedule_now", {
        loopId: "loop-6",
      })
    })

    fireEvent.click(screen.getAllByRole("button", { name: "编辑策略" })[0])
    fireEvent.change(screen.getByLabelText("无进展上限"), { target: { value: "4" } })
    fireEvent.change(screen.getByLabelText("失败上限"), { target: { value: "5" } })
    fireEvent.change(screen.getByLabelText("降频间隔"), { target: { value: "10m" } })
    fireEvent.click(screen.getByRole("button", { name: "保存" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("update_loop_schedule_policy", {
        loopId: "loop-6",
        maxRuns: null,
        maxRuntimeSecs: null,
        tokenBudget: null,
        maxNoProgressRuns: 4,
        maxFailures: 5,
        backoffSecs: 600,
      })
    })
  })

  it("creates event-triggered loops from the Loop Center", async () => {
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_active_goal") return Promise.resolve(goalSnapshotWithWorkflowTemplate())
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "list_loop_schedules") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "create_loop_schedule") return Promise.resolve(args ?? {})
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: /Loop/ }))
    fireEvent.click(screen.getByRole("button", { name: "新建" }))
    fireEvent.click(await screen.findByRole("button", { name: "Event" }))
    fireEvent.click(screen.getByRole("button", { name: "创建 Loop" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_loop_schedule", {
        sessionId: "s1",
        prompt: "",
        triggerKind: "event",
        triggerSpec: {
          eventName: "workflow:updated",
          filters: { workflowState: "completed" },
          debounceSecs: 30,
        },
        executionStrategy: "continue",
        goalId: "goal-auto",
        goalCriterionId: undefined,
        maxRuns: null,
        maxRuntimeSecs: null,
        tokenBudget: null,
        maxNoProgressRuns: 3,
        maxFailures: 3,
        backoffSecs: 300,
      })
    })
  })

  it("shows an actionable workflow empty state before any workflow run exists", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("准备开始工作流运行")).toBeTruthy()
    expect(screen.getByText("已设置")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "开始工作流运行" }))

    expect(screen.getByLabelText("从目标开始")).toBeTruthy()
  })

  it("lets the user change the session execution mode from the workspace", async () => {
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "set_execution_mode") return Promise.resolve({ mode: args?.mode })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("Execution Mode")).toBeTruthy()

    fireEvent.click(screen.getByText("深入"))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("set_execution_mode", {
        sessionId: "s1",
        mode: "deep",
      })
    })
  })

  it("lets the user create and immediately run a workflow script from the workspace", async () => {
    const run = workflowRun({ state: "draft" })
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "preview_workflow_script") return Promise.resolve(workflowScriptPreview())
      if (name === "create_workflow_run") return Promise.resolve(run)
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))

    const script = `export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Run" });
  await workflow.validate({ commands: ["pnpm typecheck"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "done", verification: ["pnpm typecheck"], residualRisk: [] });
}`
    fireEvent.click(screen.getByRole("button", { name: /高级脚本/ }))
    fireEvent.change(screen.getByLabelText("Script"), { target: { value: script } })
    fireEvent.click(screen.getByRole("switch"))

    expect((screen.getByRole("button", { name: "创建并运行" }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    fireEvent.click(screen.getByRole("button", { name: "预检" }))
    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("preview_workflow_script", {
        sessionId: "s1",
        scriptSource: script,
        executionMode: "guarded",
      })
    })
    expect(await screen.findByText("预检通过")).toBeTruthy()
    expect(screen.getAllByText("授权清单").length).toBeGreaterThan(0)

    fireEvent.click(screen.getByRole("button", { name: "创建并运行" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_workflow_run", {
        sessionId: "s1",
        kind: "general.workflow",
        executionMode: "guarded",
        scriptSource: script,
        budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
        runImmediately: true,
      })
    })
  })

  it("generates a goal-driven workflow draft before preflight", async () => {
    const run = workflowRun({ state: "draft" })
    const snapshot = workflowSnapshot(run)
    let previewedScript = ""
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "preview_workflow_script") {
        previewedScript = String(args?.scriptSource ?? "")
        return Promise.resolve(workflowScriptPreview())
      }
      if (name === "create_workflow_run") return Promise.resolve(run)
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))
    fireEvent.change(screen.getByLabelText("从目标开始"), {
      target: { value: "修复设置页保存 Provider 后没有刷新状态的问题" },
    })
    fireEvent.click(screen.getByRole("button", { name: "生成可预检草稿" }))
    fireEvent.click(screen.getByRole("button", { name: "预检" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "preview_workflow_script",
        expect.objectContaining({
          sessionId: "s1",
          executionMode: "guarded",
        }),
      )
    })

    expect(previewedScript).toContain("修复设置页保存 Provider 后没有刷新状态的问题")
    expect(previewedScript).toContain("workflow.spawnAgent")
    expect(previewedScript).toContain("workflow.waitAll")
    expect(previewedScript).toContain("Budget:")

    fireEvent.click(await screen.findByRole("button", { name: "创建并运行" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_workflow_run",
        expect.objectContaining({
          sessionId: "s1",
          kind: "general.workflow",
          executionMode: "guarded",
          budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
          runImmediately: true,
        }),
      )
    })
  })

  it("generates a domain workflow draft from the workspace template picker", async () => {
    const template = domainWorkflowTemplate()
    const draft = domainWorkflowDraft(template)
    const run = workflowRun({
      kind: draft.workflowKind,
      state: "draft",
      executionMode: draft.executionMode,
      scriptSource: draft.scriptSource,
    })
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "list_domain_workflow_templates") return Promise.resolve([template])
      if (name === "preview_domain_workflow") return Promise.resolve(draft)
      if (name === "create_workflow_run") return Promise.resolve(run)
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))
    expect((await screen.findAllByText("Research brief")).length).toBeGreaterThan(0)

    fireEvent.change(screen.getByLabelText("从目标开始"), {
      target: { value: "调研新版浏览器自动化能力并整理风险" },
    })
    fireEvent.click(screen.getByRole("button", { name: "生成领域草稿" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("preview_domain_workflow", {
        templateId: "research-brief",
        version: "1.0.0",
        sessionId: "s1",
        goalId: undefined,
        taskType: "technical_research",
        objective: "调研新版浏览器自动化能力并整理风险",
        modeOverride: "guarded",
      })
    })

    expect(await screen.findByText("草稿来自 Research brief")).toBeTruthy()
    expect(screen.getByText(/At least three dated sources/)).toBeTruthy()
    expect(screen.getByText(/external_publish/)).toBeTruthy()
    expect(screen.getByText("预检通过")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "创建并运行" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("create_workflow_run", {
        sessionId: "s1",
        kind: "domain:research",
        executionMode: "guarded",
        scriptSource: draft.scriptSource,
        budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
        goalId: undefined,
        runImmediately: true,
      })
    })
  })

  it("materializes a draft chat session before previewing and creating a workflow", async () => {
    const run = workflowRun({ id: "wf-created", sessionId: "s-created", state: "draft" })
    const snapshot = workflowSnapshot(run)
    const onEnsureSession = vi.fn(() => Promise.resolve("s-created"))
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "preview_workflow_script") return Promise.resolve(workflowScriptPreview())
      if (name === "create_workflow_run") return Promise.resolve(run)
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel(
      {
        workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
        git: null,
      },
      {
        sessionId: null,
        sessionMeta: null,
        onEnsureSession,
      },
    )

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))
    expect(screen.getByText("预检时会自动创建并切换到一个新会话")).toBeTruthy()

    fireEvent.change(screen.getByLabelText("从目标开始"), {
      target: { value: "实现自动创建 workflow 会话" },
    })
    fireEvent.click(screen.getByRole("button", { name: "生成可预检草稿" }))
    fireEvent.click(screen.getByRole("button", { name: "预检" }))

    await waitFor(() => {
      expect(onEnsureSession).toHaveBeenCalledTimes(1)
      expect(transportMock.call).toHaveBeenCalledWith(
        "preview_workflow_script",
        expect.objectContaining({
          sessionId: "s-created",
          executionMode: "guarded",
        }),
      )
    })

    fireEvent.click(await screen.findByRole("button", { name: "创建并运行" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_workflow_run",
        expect.objectContaining({
          sessionId: "s-created",
          runImmediately: true,
        }),
      )
    })
  })

  it("materializes a draft chat session before enabling workflow mode", async () => {
    const onEnsureSession = vi.fn(() => Promise.resolve("s-created"))
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "get_workflow_mode") return Promise.resolve({ mode: "off" })
      if (name === "get_execution_mode") return Promise.resolve({ mode: "off" })
      if (name === "set_workflow_mode") return Promise.resolve({ mode: args?.mode ?? "on" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "get_coding_trend_report") return Promise.resolve(null)
      if (name === "get_lsp_status") return Promise.resolve(null)
      if (name === "get_lsp_diagnostics") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(
      {
        workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
        git: null,
      },
      {
        sessionId: null,
        sessionMeta: null,
        onEnsureSession,
      },
    )

    const enableWorkflowButton = (await screen.findAllByRole("button")).find((button) =>
      button.textContent?.includes("模型按需编排"),
    )
    expect(enableWorkflowButton).toBeTruthy()
    fireEvent.click(enableWorkflowButton!)

    await waitFor(() => {
      expect(onEnsureSession).toHaveBeenCalledTimes(1)
      expect(transportMock.call).toHaveBeenCalledWith("set_workflow_mode", {
        sessionId: "s-created",
        mode: "on",
      })
    })
  })

  it("keeps goal-driven workflow drafts stopped when no working directory is set", async () => {
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "preview_workflow_script") return Promise.resolve(workflowScriptPreview())
      if (name === "create_workflow_run") return Promise.resolve(workflowRun({ state: "draft" }))
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve(args ?? [])
    })

    renderPanel(null)

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))
    expect(
      screen.getByText("当前会话未设置工作目录；目标草稿会先创建为待启动，设置目录后再运行。"),
    ).toBeTruthy()

    fireEvent.change(screen.getByLabelText("从目标开始"), {
      target: { value: "修复设置页保存 Provider 后没有刷新状态的问题" },
    })
    fireEvent.click(screen.getByRole("button", { name: "生成可预检草稿" }))

    expect((screen.getByRole("switch") as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByRole("button", { name: "创建" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "创建并运行" })).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "预检" }))
    expect(await screen.findByText("预检通过")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "创建" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_workflow_run",
        expect.objectContaining({
          sessionId: "s1",
          runImmediately: false,
        }),
      )
    })
  })

  it("blocks workflow creation when script preflight fails", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([])
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "preview_workflow_script") {
        return Promise.resolve(
          workflowScriptPreview({
            gate: {
              issues: [
                {
                  severity: "error",
                  code: "missing_finish",
                  message: "Script does not finish through workflow.finish(...).",
                  suggestion: "Return a structured final result.",
                },
              ],
            },
            gatePassed: false,
            gateFeedback: "Workflow Script Gate failed.",
            canCreate: false,
            canRunImmediately: false,
          }),
        )
      }
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    fireEvent.click(await screen.findByRole("button", { name: "新建工作流" }))
    fireEvent.click(screen.getByRole("button", { name: /高级脚本/ }))
    fireEvent.change(screen.getByLabelText("Script"), {
      target: {
        value:
          "export default async function main(workflow) { await workflow.task.create({ title: 'x' }); }",
      },
    })
    fireEvent.click(screen.getByRole("button", { name: "预检" }))

    expect(await screen.findByText("预检未通过")).toBeTruthy()
    expect(screen.getByText("Return a structured final result.")).toBeTruthy()
    expect((screen.getByRole("button", { name: "创建" }) as HTMLButtonElement).disabled).toBe(true)
    expect(transportMock.call).not.toHaveBeenCalledWith("create_workflow_run", expect.anything())
  })

  it("surfaces approval summary and primary workflow actions", async () => {
    const run = workflowRun()
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("当前焦点：等待授权")).toBeTruthy()
    expect(screen.queryByText("需要批准后继续")).toBeNull()
    expect(await screen.findByText("下一步：确认授权")).toBeTruthy()
    expect(screen.getAllByRole("button", { name: "查看 Trace" }).length).toBeGreaterThan(0)
    expect(await screen.findByText("授权清单")).toBeTruthy()
    expect(screen.getAllByText("调用").length).toBeGreaterThan(0)
    expect(screen.getAllByText("需批准").length).toBeGreaterThan(0)
    expect(screen.getAllByText("2").length).toBeGreaterThan(0)
    expect(screen.getAllByText("1").length).toBeGreaterThan(0)
    expect(screen.getAllByText("write-file").length).toBeGreaterThan(0)
    expect(screen.getAllByText("需批准").length).toBeGreaterThan(0)
    expect(screen.getAllByRole("button", { name: "批准" }).length).toBeGreaterThan(0)
    expect(await screen.findByText("运行时间线")).toBeTruthy()
    expect(screen.getByText("最近 2 条")).toBeTruthy()
    expect(await screen.findByText("审批审计")).toBeTruthy()
    expect(screen.getByText("等待批准")).toBeTruthy()
    expect(screen.getAllByText("待处理").length).toBeGreaterThan(0)
    expect(screen.getByText("最近信号")).toBeTruthy()
  })

  it("shows granted approval history in the workflow overview", async () => {
    const run = workflowRun({ state: "running" })
    const snapshot: WorkflowRunSnapshot = {
      ...workflowSnapshot(run),
      events: [
        ...workflowSnapshot(run).events,
        {
          id: 3,
          runId: run.id,
          seq: 3,
          eventType: "run_state_changed",
          payload: {
            from: "awaiting_approval",
            to: "running",
            reason: "approval_granted",
          },
          createdAt: "2026-01-01T00:01:30Z",
        },
        {
          id: 4,
          runId: run.id,
          seq: 4,
          eventType: "run_control_action",
          payload: {
            action: "approve",
            reason: "approval_granted",
            resultState: "running",
            accepted: true,
            surface: "user_control",
          },
          createdAt: "2026-01-01T00:01:31Z",
        },
        {
          id: 5,
          runId: run.id,
          seq: 5,
          eventType: "run_runtime_launch",
          payload: {
            accepted: true,
            owner: "tauri:approve:pid:123",
            reason: "primary_spawn_accepted",
            pid: 123,
          },
          createdAt: "2026-01-01T00:01:32Z",
        },
        {
          id: 6,
          runId: run.id,
          seq: 6,
          eventType: "run_runtime_result",
          payload: {
            status: "finished",
            accepted: true,
            reason: "runtime_returned",
            finalState: "completed",
            hasOutput: false,
            owner: "tauri:approve:pid:123",
            pid: 123,
          },
          createdAt: "2026-01-01T00:01:33Z",
        },
      ],
    }
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("审批审计")).toBeTruthy()
    expect(screen.getByText("已批准")).toBeTruthy()
    expect(screen.getByText("已通过")).toBeTruthy()
    expect(screen.getAllByText("控制动作").length).toBeGreaterThan(0)
    expect(screen.getAllByText("approve · running · approval_granted").length).toBeGreaterThan(0)
    expect(screen.getAllByText("启动请求").length).toBeGreaterThan(0)
    expect(
      screen.getAllByText("已接收 · tauri:approve:pid:123 · primary_spawn_accepted").length,
    ).toBeGreaterThan(0)
    expect(screen.getAllByText("启动结果").length).toBeGreaterThan(0)
    expect(screen.getAllByText("finished · completed · runtime_returned").length).toBeGreaterThan(0)
  })

  it("shows the bound worktree runtime in workflow overview", async () => {
    const run = workflowRun({ worktreeId: "wt-run" })
    const snapshot: WorkflowRunSnapshot = {
      ...workflowSnapshot(run),
      events: [
        {
          id: 1,
          runId: run.id,
          seq: 1,
          eventType: "run_worktree_attached",
          payload: {
            worktreeId: "wt-run",
            path: "/repo-worktrees/wt-run",
            state: "handoff",
          },
          createdAt: "2026-01-01T00:00:30Z",
        },
      ],
    }
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("运行位置 · wt-run")).toBeTruthy()
    expect(await screen.findByText("/repo-worktrees/wt-run")).toBeTruthy()
    expect(screen.getAllByText("运行位置已绑定").length).toBeGreaterThan(0)
    expect(screen.getAllByText("Trace").length).toBeGreaterThan(0)
  })

  it("surfaces the active workflow focus and jumps to the relevant detail tab", async () => {
    const run = workflowRun({ state: "running" })
    const snapshot: WorkflowRunSnapshot = {
      run,
      ops: [
        {
          id: "op-validate",
          runId: run.id,
          opKey: "main/op#2(workflow.validate)",
          opType: "validate",
          effectClass: "non_idempotent",
          inputHash: "hash-validate",
          input: { label: "targeted-validation", commands: ["pnpm typecheck"] },
          state: "started",
          output: null,
          error: null,
          childHandle: null,
          startedAt: "2026-01-01T00:01:00Z",
          completedAt: null,
        },
      ],
      events: [
        {
          id: 1,
          runId: run.id,
          seq: 1,
          eventType: "op_started",
          payload: { opKey: "main/op#2(workflow.validate)", opType: "validate", state: "started" },
          createdAt: "2026-01-01T00:01:00Z",
        },
      ],
    }
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("当前焦点：正在执行 targeted-validation")).toBeTruthy()
    expect(await screen.findByText("下一步：观察运行进度")).toBeTruthy()
    const validationTab = screen.getByRole("tab", { name: /Validation/ })
    expect(validationTab.getAttribute("aria-selected")).toBe("false")

    fireEvent.click(screen.getAllByRole("button", { name: "查看 Validation" })[0])

    expect(validationTab.getAttribute("aria-selected")).toBe("true")
  })

  it("lets the user expand workflow op details from the trace", async () => {
    const run = workflowRun({ state: "running" })
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    fireEvent.click(await screen.findByRole("button", { name: "展开步骤详情" }))

    expect(await screen.findByText("步骤详情")).toBeTruthy()
    expect(screen.getAllByText(/write-file/).length).toBeGreaterThan(1)
  })

  it("keeps late failed workflow steps visible in the trace focus area", async () => {
    const run = workflowRun({ state: "failed" })
    const completedOps: WorkflowRunSnapshot["ops"] = Array.from({ length: 7 }, (_, index) => ({
      id: `op-${index + 1}`,
      runId: run.id,
      opKey: `main/op#${index + 1}(workflow.tool)`,
      opType: "tool",
      effectClass: "idempotent",
      inputHash: `hash-${index + 1}`,
      input: { label: `setup-${index + 1}` },
      state: "completed",
      output: { summary: `setup ${index + 1} complete` },
      error: null,
      childHandle: null,
      startedAt: "2026-01-01T00:01:00Z",
      completedAt: "2026-01-01T00:01:30Z",
    }))
    const snapshot: WorkflowRunSnapshot = {
      run,
      ops: [
        ...completedOps,
        {
          id: "op-late-tool",
          runId: run.id,
          opKey: "main/op#8(workflow.tool)",
          opType: "tool",
          effectClass: "non_idempotent",
          inputHash: "hash-late-tool",
          input: { label: "late-write-step", name: "write" },
          state: "failed",
          output: null,
          error: { message: "late write failed" },
          childHandle: null,
          startedAt: "2026-01-01T00:08:00Z",
          completedAt: "2026-01-01T00:08:30Z",
        },
      ],
      events: [
        {
          id: 8,
          runId: run.id,
          seq: 8,
          eventType: "op_failed",
          payload: {
            opKey: "main/op#8(workflow.tool)",
            opType: "tool",
            state: "failed",
          },
          createdAt: "2026-01-01T00:08:30Z",
        },
      ],
    }
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("关注步骤")).toBeTruthy()
    expect(screen.getAllByText("late-write-step").length).toBeGreaterThan(0)
    expect(screen.getByText(/前 6\/8 个步骤/)).toBeTruthy()
    expect(screen.getByText("关键信号")).toBeTruthy()
    expect(screen.getAllByText("步骤失败").length).toBeGreaterThan(0)
  })

  it("lets the user start a draft workflow run from the workspace", async () => {
    const run = workflowRun({ state: "draft" })
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "run_workflow_run") return Promise.resolve({ ...run, state: "running" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect((await screen.findAllByText("待启动")).length).toBeGreaterThan(0)

    fireEvent.click(screen.getAllByRole("button", { name: "运行" })[0])

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("run_workflow_run", { runId: "wf-1" })
    })
  })

  it("shows output token budget usage in the workflow summary", async () => {
    const run = workflowRun({
      state: "blocked",
      blockedReason: "workflow_budget_output_tokens_exhausted",
      budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
    })
    const snapshot = workflowSnapshot(run)
    snapshot.events.push({
      id: 3,
      runId: run.id,
      seq: 3,
      eventType: "budget_usage",
      payload: {
        spentOutputTokens: 10000,
        maxOutputTokens: 10000,
        exhausted: true,
        reason: "workflow_budget_output_tokens_exhausted",
      },
      createdAt: "2026-01-01T00:02:00Z",
    })
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    fireEvent.click(await screen.findByText("coding.feature"))

    expect((await screen.findAllByText("输出预算")).length).toBeGreaterThan(0)
    expect(screen.getAllByText("10.0k/10.0k").length).toBeGreaterThan(0)
    expect(screen.getAllByText("预算用量").length).toBeGreaterThan(0)
  })

  it("confirms before cancelling a workflow run", async () => {
    const run = workflowRun({ state: "running" })
    const snapshot = workflowSnapshot(run)
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "cancel_workflow_run") return Promise.resolve({ ...run, state: "cancelled" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect((await screen.findAllByText("coding.feature")).length).toBeGreaterThan(0)

    fireEvent.click(screen.getAllByRole("button", { name: "取消" })[0])

    expect(screen.getByText("取消这个工作流运行？")).toBeTruthy()
    expect(screen.getByText(/已有 trace 会保留/)).toBeTruthy()
    expect(transportMock.call).not.toHaveBeenCalledWith("cancel_workflow_run", expect.anything())

    fireEvent.click(screen.getByRole("button", { name: "确认取消" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("cancel_workflow_run", { runId: "wf-1" })
    })
  })

  it("disables the cancel confirmation when the run becomes terminal while the dialog is open", async () => {
    const listeners = new Map<string, Array<(payload: unknown) => void>>()
    transportMock.listen.mockImplementation(
      (eventName: string, handler: (payload: unknown) => void) => {
        const handlers = listeners.get(eventName) ?? []
        handlers.push(handler)
        listeners.set(eventName, handlers)
        return () => {
          const next = (listeners.get(eventName) ?? []).filter((current) => current !== handler)
          if (next.length > 0) {
            listeners.set(eventName, next)
          } else {
            listeners.delete(eventName)
          }
        }
      },
    )
    let currentRun = workflowRun({ state: "running" })
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([currentRun])
      if (name === "get_workflow_run") return Promise.resolve(workflowSnapshot(currentRun))
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "cancel_workflow_run")
        return Promise.resolve({ ...currentRun, state: "cancelled" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect((await screen.findAllByText("coding.feature")).length).toBeGreaterThan(0)
    fireEvent.click(screen.getAllByRole("button", { name: "取消" })[0])
    expect(screen.getByText("取消这个工作流运行？")).toBeTruthy()

    currentRun = workflowRun({
      state: "completed",
      updatedAt: "2026-01-01T00:03:00Z",
      completedAt: "2026-01-01T00:03:00Z",
    })
    act(() => {
      for (const handler of listeners.get("workflow:updated") ?? []) {
        handler(currentRun)
      }
    })

    await waitFor(() => {
      expect(screen.getAllByText("已完成").length).toBeGreaterThan(0)
    })
    const confirm = screen.getByRole("button", { name: "确认取消" }) as HTMLButtonElement
    expect(confirm.disabled).toBe(true)

    fireEvent.click(confirm)
    expect(transportMock.call).not.toHaveBeenCalledWith("cancel_workflow_run", expect.anything())
  })

  it("polls active workflow runs as a fallback when live events are missed", async () => {
    vi.useFakeTimers()
    const running = workflowRun({ state: "running", kind: "coding.running" })
    const completed = workflowRun({
      state: "completed",
      kind: "coding.completed",
      updatedAt: "2026-01-01T00:02:00Z",
      completedAt: "2026-01-01T00:02:00Z",
    })
    let listCalls = 0
    let currentRun = running
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") {
        listCalls += 1
        currentRun = listCalls >= 2 ? completed : running
        return Promise.resolve([currentRun])
      }
      if (name === "get_workflow_run") return Promise.resolve(workflowSnapshot(currentRun))
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
    expect(screen.getByText("coding.running")).toBeTruthy()

    await act(async () => {
      vi.advanceTimersByTime(4000)
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(listCalls).toBeGreaterThanOrEqual(2)
    expect(screen.getByText("coding.completed")).toBeTruthy()
  })

  it("renders validation command details and recovery guidance", async () => {
    const writeText = vi.fn(async (_value: string) => {})
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    })
    const run = workflowRun({ state: "failed", worktreeId: "wt-repair" })
    const snapshot: WorkflowRunSnapshot = {
      run,
      ops: [
        {
          id: "op-validate",
          runId: run.id,
          opKey: "main/op#2(workflow.validate)",
          opType: "validate",
          effectClass: "non_idempotent",
          inputHash: "hash-validate",
          input: { commands: ["pnpm typecheck", "pnpm test"] },
          state: "completed",
          output: {
            ok: false,
            summary: "1/2 validation command(s) failed",
            results: [
              {
                command: "pnpm typecheck",
                cwd: "/repo",
                jobStatus: "completed",
                ok: true,
                exitCode: 0,
                output: "ok",
              },
              {
                command: "pnpm test",
                cwd: "/repo",
                jobStatus: "completed",
                ok: false,
                exitCode: 1,
                output: "expected value to be true",
              },
            ],
          },
          error: null,
          childHandle: null,
          startedAt: "2026-01-01T00:01:00Z",
          completedAt: "2026-01-01T00:02:00Z",
        },
      ],
      events: [
        {
          id: 1,
          runId: run.id,
          seq: 1,
          eventType: "guarded_repair_validation_failed",
          payload: {
            opKey: "main/op#2(workflow.validate)",
            summary: "1/2 validation command(s) failed",
            failed: 1,
            total: 2,
            stopReason: "validation_failed",
          },
          createdAt: "2026-01-01T00:02:00Z",
        },
      ],
    }
    let previewedRepairScript = ""
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([run])
      if (name === "get_workflow_run") return Promise.resolve(snapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "list_managed_worktrees") return Promise.resolve([managedWorktree()])
      if (name === "preview_workflow_script") {
        previewedRepairScript = String(args?.scriptSource ?? "")
        return Promise.resolve(workflowScriptPreview())
      }
      if (name === "create_workflow_run") {
        return Promise.resolve(
          workflowRun({
            id: "wf-repair",
            kind: "general.workflow",
            state: "draft",
            parentRunId: "wf-1",
            origin: "repair",
          }),
        )
      }
      if (name === "create_session_task") return Promise.resolve([])
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("当前焦点：验证失败")).toBeTruthy()
    expect(screen.queryByText("有失败步骤")).toBeNull()
    expect(await screen.findByText("下一步：修复验证失败")).toBeTruthy()

    expect(await screen.findByText("pnpm typecheck")).toBeTruthy()
    expect(screen.getByText("pnpm test")).toBeTruthy()
    expect(screen.getByText(/expected value to be true/)).toBeTruthy()

    fireEvent.click(screen.getAllByRole("button", { name: "展开验证输出" })[1])
    expect(await screen.findByText("验证输出")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "生成修复草稿" }))

    await waitFor(() => {
      expect(screen.getByLabelText("从目标开始")).toBeTruthy()
    })
    const objective = screen.getByLabelText("从目标开始") as HTMLTextAreaElement
    fireEvent.click(screen.getByRole("button", { name: /高级脚本/ }))
    const script = screen.getByLabelText("Script") as HTMLTextAreaElement
    expect(objective.value).toContain("继续修复失败的工作流运行 wf-1")
    expect(objective.value).toContain("expected value to be true")
    expect(script.value).toContain("expected value to be true")
    expect(script.value).toContain("workflow.spawnAgent")
    expect(screen.getByText("修复自 wf-1")).toBeTruthy()
    expect(screen.getByText(/不会覆盖原运行/)).toBeTruthy()
    expect(screen.getByText("repair-wt")).toBeTruthy()
    expect(screen.getByRole("button", { name: "创建并运行修复" })).toBeTruthy()

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "preview_workflow_script",
        expect.objectContaining({
          sessionId: "s1",
          executionMode: "guarded",
        }),
      )
    })
    expect(await screen.findByText("预检通过")).toBeTruthy()
    expect(previewedRepairScript).toContain("继续修复失败的工作流运行 wf-1")
    expect(previewedRepairScript).toContain("expected value to be true")

    fireEvent.click(screen.getByRole("button", { name: "复制修复提示" }))

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(1)
    })
    const prompt = String(writeText.mock.calls[0]?.[0] ?? "")
    expect(prompt).toContain("工作流失败上下文")
    expect(prompt).toContain("state: failed")
    expect(prompt).toContain("main/op#2(workflow.validate)")
    expect(prompt).toContain("pnpm test")
    expect(prompt).toContain("expected value to be true")

    const recoveryTitle = screen.getByText("下一步：修复验证失败")
    const recoveryCard = recoveryTitle.parentElement?.parentElement
    expect(recoveryCard).toBeTruthy()
    fireEvent.click(within(recoveryCard as HTMLElement).getByRole("button", { name: "转任务" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_session_task",
        expect.objectContaining({
          sessionId: "s1",
          activeForm: "正在修复失败工作流 wf-1",
          content: expect.stringContaining("修复失败工作流 wf-1"),
        }),
      )
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_session_task",
        expect.objectContaining({
          content: expect.stringContaining("expected value to be true"),
        }),
      )
    })

    fireEvent.click(screen.getByRole("button", { name: "创建并运行修复" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_workflow_run",
        expect.objectContaining({
          sessionId: "s1",
          kind: "general.workflow",
          executionMode: "guarded",
          parentRunId: "wf-1",
          origin: "repair",
          worktreeId: "wt-repair",
          runImmediately: true,
        }),
      )
    })
  })

  it("surfaces persisted workflow derivation links", async () => {
    const child = workflowRun({
      id: "wf-child",
      kind: "coding.repair",
      state: "draft",
      parentRunId: "wf-parent",
      origin: "repair",
    })
    const childSnapshot: WorkflowRunSnapshot = {
      ...workflowSnapshot(child),
      events: [
        {
          id: 10,
          runId: child.id,
          seq: 10,
          eventType: "run_derived_child_created",
          payload: { parentRunId: child.id, childRunId: "wf-grandchild", origin: "repair" },
          createdAt: "2026-01-01T00:03:00Z",
        },
      ],
    }
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_workflow_runs") return Promise.resolve([child])
      if (name === "get_workflow_run") return Promise.resolve(childSnapshot)
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("修复自 wf-parent")).toBeTruthy()
    expect(await screen.findByText("已生成修复运行 wf-grandchild")).toBeTruthy()
  })

  it("uses the latest repair source when switching between failed workflow runs", async () => {
    const oldRun = workflowRun({ id: "wf-old", state: "failed", kind: "coding.old" })
    const newRun = workflowRun({ id: "wf-new", state: "failed", kind: "coding.new" })
    const fillerRuns = Array.from({ length: 6 }, (_, index) =>
      workflowRun({
        id: `wf-history-${index}`,
        state: "completed",
        kind: `coding.history.${index}`,
      }),
    )
    const snapshotFor = (run: WorkflowRun): WorkflowRunSnapshot => ({
      run,
      ops: [
        {
          id: `op-${run.id}`,
          runId: run.id,
          opKey: "main/op#2(workflow.tool)",
          opType: "tool",
          effectClass: "non_idempotent",
          inputHash: `hash-${run.id}`,
          input: { label: `repair-${run.id}`, name: "write" },
          state: "failed",
          output: null,
          error: { message: `${run.id} failed` },
          childHandle: null,
          startedAt: "2026-01-01T00:01:00Z",
          completedAt: "2026-01-01T00:02:00Z",
        },
      ],
      events: [],
    })
    const snapshots = new Map([
      [oldRun.id, snapshotFor(oldRun)],
      [newRun.id, snapshotFor(newRun)],
      ...fillerRuns.map((run) => [run.id, workflowSnapshot(run)] as const),
    ])
    transportMock.call.mockImplementation((name: string, args?: Record<string, unknown>) => {
      if (name === "list_workflow_runs") return Promise.resolve([oldRun, ...fillerRuns, newRun])
      if (name === "get_workflow_run") {
        return Promise.resolve(snapshots.get(String(args?.runId)) ?? snapshotFor(oldRun))
      }
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "preview_workflow_script") return Promise.resolve(workflowScriptPreview())
      if (name === "create_workflow_run") {
        return Promise.resolve(
          workflowRun({
            id: "wf-repair",
            kind: "general.workflow",
            state: "draft",
            parentRunId: String(args?.parentRunId ?? ""),
            origin: "repair",
          }),
        )
      }
      if (name === "get_background_job") return Promise.resolve(null)
      return Promise.resolve([])
    })

    renderPanel(null)

    expect(await screen.findByText("coding.old")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "生成修复草稿" }))
    expect(await screen.findByText("修复自 wf-old")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "另有 2 个历史运行" }))
    fireEvent.click(screen.getByRole("button", { name: /coding\.new/ }))
    expect((await screen.findAllByText("wf-new failed")).length).toBeGreaterThan(0)
    fireEvent.click(screen.getByRole("button", { name: "生成修复草稿" }))
    expect(await screen.findByText("修复自 wf-new")).toBeTruthy()
    expect(await screen.findByText("预检通过")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "创建修复运行" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "create_workflow_run",
        expect.objectContaining({
          parentRunId: "wf-new",
          origin: "repair",
        }),
      )
    })
  })
})
