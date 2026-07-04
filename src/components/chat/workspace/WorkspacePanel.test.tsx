// @vitest-environment jsdom

import type { ComponentProps } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { WorkspaceEnvironmentState } from "./useWorkspaceEnvironment"
import type {
  CodingImprovementProposal,
  CodingTrendReport,
  ContextRetrievalSnapshot,
  DomainEvidenceItem,
  DomainQualityRunSnapshot,
  DomainWorkflowDraft,
  DomainWorkflowTemplate,
  ManagedWorktree,
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
    fireEvent.change(screen.getByPlaceholderText("每行一个标准：功能完成、证据充分、风险可解释"), {
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

    fireEvent.click(await screen.findByText("Ship isolated worktree"))

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

    fireEvent.click(await screen.findByText("Write sourced research brief"))

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
})

describe("WorkspacePanel context retrieval section", () => {
  it("records a generated context summary as domain evidence", async () => {
    const snapshot = contextRetrievalSnapshot()
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
      if (name === "get_execution_mode") return Promise.resolve({ mode: "guarded" })
      if (name === "get_background_job") return Promise.resolve(null)
      if (name === "list_workflow_runs") return Promise.resolve([])
      return Promise.resolve([])
    })

    renderPanel({
      workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
      git: null,
    })

    expect(await screen.findByText("Browser automation notes")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "摘要" }))

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

    fireEvent.click(await screen.findByRole("button", { name: /领域复核/ }))
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
    const validationTab = screen.getByRole("tab", { name: /Validation/ })
    expect(validationTab.getAttribute("aria-selected")).toBe("false")

    fireEvent.click(screen.getByRole("button", { name: "查看 Validation" }))

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
