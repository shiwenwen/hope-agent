import { useMemo, useState } from "react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import WorkspacePanel from "@/components/chat/workspace/WorkspacePanel"
import { createTaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import { setTransport } from "@/lib/transport-provider"
import type { GoalSnapshot } from "@/components/chat/workspace/useGoal"
import type { LoopSchedule, LoopSnapshot } from "@/components/chat/workspace/useLoopSchedules"
import type { WorkflowRun, WorkflowRunSnapshot } from "@/components/chat/workspace/useWorkflowRuns"
import type {
  DomainArtifactExportGuardReport,
  DomainConnectorActionGuardReport,
  DomainConnectorE2EGateReport,
  DomainEvidenceItem,
  DomainOperationalGateReport,
  DomainSoakReport,
  SessionArtifacts,
  Transport,
  WorkspaceEnvironmentSnapshot,
} from "@/lib/transport"
import type { SessionMeta, Task } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type { BackgroundJobSnapshot } from "@/types/background-jobs"

const WORKSPACE_SMOKE_SESSION_ID = "workspace-v35-smoke-session"
const WORKSPACE_SMOKE_PROJECT_ID = "workspace-v35-smoke-project"

function nowIso(offsetMinutes = 0): string {
  return new Date(Date.UTC(2026, 6, 8, 12, offsetMinutes, 0)).toISOString()
}

const smokeTasks: Task[] = [
  {
    id: 1,
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    content: "确认默认工作台只展示状态故事和主要操作",
    activeForm: "正在确认工作台默认状态",
    batchId: "workspace-v35",
    status: "completed",
    createdAt: nowIso(1),
    updatedAt: nowIso(4),
  },
  {
    id: 2,
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    content: "展开高级诊断并核对长跑/守门信息",
    activeForm: "正在核对高级诊断",
    batchId: "workspace-v35",
    status: "in_progress",
    createdAt: nowIso(5),
    updatedAt: nowIso(10),
  },
  {
    id: 3,
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    content: "跑窄屏和宽屏截图验收",
    activeForm: "正在准备截图验收",
    batchId: "workspace-v35",
    status: "pending",
    createdAt: nowIso(8),
    updatedAt: nowIso(8),
  },
]

function workflowRun(): WorkflowRun {
  return {
    id: "workflow-v35-smoke",
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    kind: "general.workspace_ux",
    state: "running",
    executionMode: "guarded",
    scriptHash: "workspacev35",
    scriptSource: "export default async function main(workflow) {}",
    budget: { sizeGuideline: "medium", maxScriptSecs: 600, maxOps: 32, outputTokenBudget: 12000 },
    cursorSeq: 4,
    primaryOwner: null,
    blockedReason: null,
    parentRunId: null,
    origin: "workflow_mode",
    goalId: "goal-v35-smoke",
    goalCriterionId: "crit-v35-workspace",
    goalCriterionText: "工作台默认信息架构通过人工界面验收",
    goalCriterionKind: "required",
    goalRevision: 2,
    createdAt: nowIso(2),
    updatedAt: nowIso(12),
    completedAt: null,
  }
}

function workflowSnapshot(): WorkflowRunSnapshot {
  const run = workflowRun()
  return {
    run,
    ops: [
      {
        id: "op-workspace-observe",
        runId: run.id,
        opKey: "main/op#1(workflow.tool)",
        opType: "tool",
        effectClass: "pure",
        inputHash: "observe",
        input: { name: "file_search", args: { query: "WorkspacePanel V3.5 UX" }, label: "观察" },
        state: "completed",
        output: { summary: "已定位工作台面板和输入框菜单界面。" },
        error: null,
        childHandle: null,
        startedAt: nowIso(3),
        completedAt: nowIso(4),
      },
      {
        id: "op-workspace-validate",
        runId: run.id,
        opKey: "main/op#2(workflow.validate)",
        opType: "validate",
        effectClass: "non_idempotent",
        inputHash: "validate",
        input: {
          label: "工作台回归测试",
          commands: ["pnpm vitest run src/components/chat/workspace/WorkspacePanel.test.tsx"],
        },
        state: "completed",
        output: {
          ok: true,
          summary: "WorkspacePanel 测试 63/63 通过",
          results: [
            {
              command: "pnpm vitest run src/components/chat/workspace/WorkspacePanel.test.tsx",
              cwd: "/repo",
              jobStatus: "completed",
              ok: true,
              exitCode: 0,
              output: "63 passed",
            },
          ],
        },
        error: null,
        childHandle: "job-workspace-panel-tests",
        startedAt: nowIso(5),
        completedAt: nowIso(8),
      },
      {
        id: "op-workspace-smoke",
        runId: run.id,
        opKey: "main/op#3(workflow.agent)",
        opType: "agent",
        effectClass: "non_idempotent",
        inputHash: "smoke",
        input: { label: "界面验收", task: "检查窄屏和宽屏工作台布局。" },
        state: "started",
        output: null,
        error: null,
        childHandle: "subagent-gui-smoke",
        startedAt: nowIso(12),
        completedAt: null,
      },
    ],
    events: [
      {
        id: 1,
        runId: run.id,
        seq: 1,
        eventType: "workflow_phase_report",
        payload: { phase: "workspace-ux", summary: "回归测试已通过，界面验收正在补齐。" },
        createdAt: nowIso(8),
      },
    ],
  }
}

function loopSchedule(): LoopSchedule {
  return {
    id: "loop-v35-smoke",
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    goalId: "goal-v35-smoke",
    goalCriterionId: "crit-v35-workspace",
    goalCriterionText: "工作台默认信息架构通过人工界面验收",
    goalCriterionKind: "required",
    goalRevision: 2,
    cronJobId: "cron-v35-smoke",
    prompt: "持续验证 V3.5 工作台界面验收状态，直到 GUI 证据收集完成。",
    triggerKind: "dynamic",
    triggerSpec: {
      fallbackSecs: 1800,
      fallbackUsed: false,
      maintenancePrompt: {
        enabled: true,
        source: "built_in",
        path: null,
        contentHash: "workspace-v35",
      },
    },
    executionStrategy: "workflow",
    state: "active",
    maxRuns: null,
    runCount: 3,
    maxRuntimeSecs: null,
    tokenBudget: null,
    costBudgetMicros: null,
    progressState: "progressed",
    progressSummary: "WorkspacePanel 测试已通过，正在补齐 GUI 截图证据。",
    noProgressStreak: 0,
    failureStreak: 0,
    maxNoProgressRuns: 3,
    maxFailures: 3,
    backoffSecs: 600,
    nextRunAt: nowIso(40),
    cronStatus: "active",
    approvalPolicySnapshot: {},
    createdAt: nowIso(0),
    updatedAt: nowIso(12),
    completedAt: null,
    blockedReason: null,
  }
}

function loopSnapshot(): LoopSnapshot {
  const schedule = loopSchedule()
  return {
    schedule,
    runs: [
      {
        id: "loop-run-v35-smoke",
        loopId: schedule.id,
        cronJobId: schedule.cronJobId,
        cronRunLogId: 3,
        sessionId: WORKSPACE_SMOKE_SESSION_ID,
        seq: 3,
        state: "succeeded",
        triggerReason: "工作台 V3.5 界面验收动态触发",
        resultSummary: "回归测试已通过，安排后续 GUI 证据补充。",
        error: null,
        progressState: "progressed",
        progressDelta: {},
        noProgressReason: null,
        schedulingDecision: "dynamic_reschedule_1800s",
        trace: {
          workflowRunId: "workflow-v35-smoke",
          dynamicDecision: {
            source: "tool",
            action: "reschedule",
            delaySecs: 1800,
            reason: "Manual screenshot evidence is still pending.",
          },
        },
        startedAt: nowIso(10),
        finishedAt: nowIso(12),
      },
    ],
  }
}

function goalSnapshot(): GoalSnapshot {
  return {
    goal: {
      id: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      objective: "完成 V3.5 工作台默认体验与高级诊断界面验收",
      completionCriteria: [
        "[required] 默认工作台只展示状态故事和主要行动。",
        "[required] 高级诊断展开后不丢失运行稳定性、长跑审计和守门证据。",
        "[required] 窄屏和宽屏无明显溢出、裁剪或错行。",
      ].join("\n"),
      revision: 2,
      domain: "project_ops",
      workflowTemplateId: null,
      workflowTemplateVersion: null,
      workflowTaskType: null,
      state: "active",
      modeSnapshot: "guarded",
      budgetTokenLimit: null,
      budgetTimeLimitSecs: null,
      budgetTurnLimit: null,
      createdAt: nowIso(0),
      updatedAt: nowIso(12),
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: {
        evaluatorKind: "post_turn",
        status: "incomplete",
        summary: "WorkspacePanel 回归测试已通过，GUI 截图证据仍需补齐。",
        nextEvidenceNeeded: ["narrow screenshot", "wide screenshot", "popover clipping check"],
      },
      closureDecision: null,
      closureReason: null,
      closedAt: null,
      followUpItems: [],
    },
    links: [],
    events: [
      {
        id: 1,
        goalId: "goal-v35-smoke",
        seq: 1,
        kind: "goal_runner_evaluated",
        payload: { status: "incomplete", summary: "GUI evidence pending." },
        createdAt: nowIso(12),
      },
    ],
    criteriaItems: [
      {
        id: "crit-v35-default",
        text: "默认工作台只展示状态故事和主要行动",
        kind: "required",
      },
      {
        id: "crit-v35-advanced",
        text: "高级诊断展开后不丢失运行稳定性、长跑审计和守门证据",
        kind: "required",
      },
      {
        id: "crit-v35-layout",
        text: "窄屏和宽屏无明显溢出、裁剪或错行",
        kind: "required",
      },
    ],
    criteria: [
      {
        id: "crit-v35-default",
        text: "默认工作台只展示状态故事和主要行动",
        kind: "required",
        status: "satisfied",
        evidenceIds: ["ev-workspace-tests"],
        reason: "WorkspacePanel 测试已覆盖折叠区和默认区。",
      },
      {
        id: "crit-v35-advanced",
        text: "高级诊断展开后不丢失运行稳定性、长跑审计和守门证据",
        kind: "required",
        status: "satisfied",
        evidenceIds: ["ev-advanced-tests"],
        reason: "WorkspacePanel 测试已覆盖运行稳定性、长跑、交付和连接器守门面板。",
      },
      {
        id: "crit-v35-layout",
        text: "窄屏和宽屏无明显溢出、裁剪或错行",
        kind: "required",
        status: "missing",
        evidenceIds: [],
        reason: "Needs GUI screenshot evidence.",
      },
    ],
    evidence: [
      {
        id: "ev-workspace-tests",
        sourceType: "validation",
        sourceId: "vitest-workspace-panel",
        relation: "validation_passed",
        title: "WorkspacePanel 测试通过",
        summary: "V3.5 i18n 和信息架构更新后，测试 63/63 通过。",
        metadata: {},
        createdAt: nowIso(13),
      },
      {
        id: "ev-advanced-tests",
        sourceType: "validation",
        sourceId: "vitest-workspace-advanced",
        relation: "advanced_diagnostics_visible",
        title: "Advanced diagnostics panel coverage",
        summary: "已覆盖运行稳定性、长跑、交付守门、连接器守门和连接器端到端（E2E）面板。",
        metadata: {},
        createdAt: nowIso(13),
      },
    ],
    timeline: [],
    budget: {
      tokensUsed: 0,
      elapsedSecs: 13 * 60,
      turnsUsed: 4,
      warning: false,
      exhausted: false,
      warnings: [],
      exceeded: [],
    },
    workflowRuns: [workflowRun()],
    tasks: smokeTasks,
  }
}

function evidenceItems(): DomainEvidenceItem[] {
  return [
    {
      id: "domain-ev-source",
      goalId: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      domain: "project_ops",
      evidenceType: "source_cited",
      title: "V3.5 roadmap updated",
      summary: "路线图已记录新的工作台信息架构和剩余证明路线。",
      sourceMetadata: { path: "agent-control-plane-v3-roadmap.md" },
      confidence: 0.96,
      accessScope: "session",
      redactionStatus: "safe",
      createdAt: nowIso(14),
      updatedAt: nowIso(14),
    },
    {
      id: "domain-ev-artifact",
      goalId: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      domain: "project_ops",
      evidenceType: "artifact_reviewed",
      title: "WorkspacePanel 回归记录",
      summary: "记录当前证据和剩余界面验收要求。",
      sourceMetadata: { path: "v3.5-workspace-ux-regression-note.md" },
      confidence: 0.92,
      accessScope: "session",
      redactionStatus: "safe",
      createdAt: nowIso(15),
      updatedAt: nowIso(15),
    },
  ]
}

function operationalGate(): DomainOperationalGateReport {
  return {
    generatedAt: nowIso(16),
    status: "insufficient_data",
    scope: "session",
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    projectId: WORKSPACE_SMOKE_PROJECT_ID,
    domain: "project_ops",
    since: nowIso(-1440),
    thresholds: {
      windowDays: 2,
      minWorkflowRuns: 1,
      maxFailedWorkflowRuns: 0,
      maxBlockedWorkflowRuns: 0,
      maxCancelledWorkflowRuns: 0,
      maxActiveWorkflowRuns: 1,
      minLoopRuns: 1,
      maxFailedLoopRuns: 0,
      maxActiveCampaigns: 0,
      maxFailedCampaignItems: 0,
    },
    summary: {
      workflowRuns: 1,
      completedWorkflowRuns: 0,
      failedWorkflowRuns: 0,
      blockedWorkflowRuns: 0,
      cancelledWorkflowRuns: 0,
      activeWorkflowRuns: 1,
      pausedWorkflowRuns: 0,
      awaitingApprovalWorkflowRuns: 0,
      loopSchedules: 1,
      activeLoopSchedules: 1,
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
      latestActivityAt: nowIso(16),
      maxActiveWorkAgeSecs: 780,
    },
    checks: [
      {
        name: "active_workflow_drain",
        status: "insufficient_data",
        severity: "warning",
        expected: "active workflow drains before closure",
        actual: "1",
        detail: "工作台验收工作流仍在收集截图证据。",
      },
    ],
    blockers: [],
    recommendedNextSteps: ["接受 V3.5 前，补齐窄屏和宽屏工作台截图。"],
  }
}

function soakReport(): DomainSoakReport {
  const gate = operationalGate()
  return {
    generatedAt: nowIso(17),
    status: "insufficient_data",
    scope: "session",
    sessionId: WORKSPACE_SMOKE_SESSION_ID,
    projectId: WORKSPACE_SMOKE_PROJECT_ID,
    domain: "project_ops",
    windowDays: 2,
    since: nowIso(-1440),
    until: nowIso(17),
    summary: {
      workflowRuns: 1,
      completedWorkflowRuns: 0,
      failedWorkflowRuns: 0,
      blockedWorkflowRuns: 0,
      cancelledWorkflowRuns: 0,
      activeWorkflowRuns: 1,
      awaitingApprovalWorkflowRuns: 0,
      repairWorkflowRuns: 0,
      approvalEvents: 0,
      approvalRequestEvents: 0,
      approvalDecisionEvents: 0,
      openApprovalWaits: 0,
      pauseEvents: 0,
      resumeEvents: 0,
      cancelEvents: 0,
      recoveryEvents: 0,
      workflowControlInterventionEvents: 0,
      workflowBudgetUsageEvents: 1,
      workflowBudgetExhaustedEvents: 0,
      maxWorkflowOutputTokensSpent: 1200,
      maxWorkflowOutputTokenBudget: 12000,
      averageApprovalWaitSecs: null,
      maxApprovalWaitSecs: null,
      maxOpenApprovalWaitSecs: null,
      averageWorkflowDrainSecs: null,
      maxWorkflowDrainSecs: null,
      latestActivityAt: nowIso(16),
      latestActivityAgeSecs: 60,
      sampleDays: 1,
      requiredSampleDays: 2,
      loopRuns: 1,
      succeededLoopRuns: 1,
      failedLoopRuns: 0,
      activeLoopRuns: 0,
      averageLoopDurationSecs: 120,
      maxLoopDurationSecs: 120,
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
      incidents: 0,
      criticalIncidents: 0,
      warningIncidents: 0,
      totalRecords: 3,
    },
    incidents: [],
    timeline: [
      {
        source: "workflow",
        id: "workflow-v35-smoke",
        label: "工作台界面验收工作流",
        status: "running",
        at: nowIso(12),
        durationSecs: 780,
      },
      {
        source: "loop",
        id: "loop-run-v35-smoke",
        label: "动态 Loop 已重新调度",
        status: "succeeded",
        at: nowIso(12),
        durationSecs: 120,
      },
    ],
    recommendedNextSteps: ["补齐两个视口宽度的长任务截图证据。"],
    markdown: "# 工作台 V3.5 界面验收\n\n回归测试已通过；GUI 截图证据仍待补齐。",
    operationalGate: gate,
  }
}

function exportGuard(): DomainArtifactExportGuardReport {
  return {
    generatedAt: nowIso(18),
    status: "passed",
    scope: {
      scope: "session",
      goalId: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      domain: "project_ops",
    },
    artifactPath: "v3.5-workspace-ux-regression-note.md",
    artifactTitle: "V3.5 工作台体验回归收敛记录",
    artifactKind: "markdown",
    thresholds: {
      requireArtifactCreated: true,
      requireArtifactReviewed: true,
      maxSensitiveUnreviewed: 0,
      maxRedactionPending: 0,
    },
    summary: {
      evidenceItems: 2,
      artifactCreated: 1,
      artifactReviewed: 1,
      exportReviewed: 1,
      sensitiveEvidence: 0,
      sensitiveUnreviewed: 0,
      redactionPending: 0,
      privateOrConnectorEvidence: 0,
    },
    checks: [
      {
        name: "artifact_reviewed",
        status: "passed",
        severity: "info",
        expected: "artifact reviewed",
        actual: "1",
        detail: "V3.5 记录已保存验证结果和剩余 proof。",
      },
    ],
    blockers: [],
    recommendedNextSteps: [],
    evidenceRequiringReview: [],
  }
}

function connectorGuard(): DomainConnectorActionGuardReport {
  return {
    generatedAt: nowIso(18),
    status: "insufficient_data",
    scope: {
      scope: "session",
      goalId: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      domain: "project_ops",
    },
    toolName: "google_docs",
    connector: "google-drive",
    action: "update_document",
    risk: "delivery",
    thresholds: {
      requireExplicitApproval: true,
      requireRollbackPlan: true,
      requireExportGuardForDelivery: true,
    },
    summary: {
      evidenceItems: 1,
      actionEvidence: 1,
      approvalEvidence: 0,
      rollbackEvidence: 0,
      sensitiveEvidence: 0,
      deliveryAction: true,
      exportGuardStatus: "passed",
    },
    checks: [
      {
        name: "explicit_user_approval",
        status: "failed",
        severity: "critical",
        expected: "explicit approval before external mutation",
        actual: "0",
        detail: "连接器写入不属于本次界面验收范围，应继续守门。",
      },
    ],
    blockers: ["外部连接器写入必须等用户明确批准证据齐全后才能执行。"],
    recommendedNextSteps: ["记录连接器端到端（E2E）证据前，先使用沙箱连接器账号。"],
    relatedEvidence: [],
  }
}

function connectorE2E(): DomainConnectorE2EGateReport {
  return {
    generatedAt: nowIso(18),
    status: "insufficient_data",
    scope: {
      scope: "session",
      goalId: "goal-v35-smoke",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      domain: "project_ops",
    },
    toolName: "google_docs",
    connector: "google-drive",
    action: "update_document",
    risk: "delivery",
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
      evidenceItems: 1,
      connectorInputEvidence: 1,
      draftEvidence: 0,
      approvalEvidence: 0,
      executionEvidence: 0,
      verificationEvidence: 0,
      rollbackEvidence: 0,
      sensitiveEvidence: 0,
      deliveryAction: true,
      connectorActionGuardStatus: "insufficient_data",
      exportGuardStatus: "passed",
    },
    checks: [
      {
        name: "draft_or_preview",
        status: "insufficient_data",
        severity: "warning",
        expected: "draft before execution",
        actual: "0",
        detail: "界面验收夹具不会执行真实外部连接器动作。",
      },
    ],
    blockers: [],
    recommendedNextSteps: ["在 V3.6 strict proof 中运行沙箱连接器端到端（E2E）。"],
    relatedEvidence: [],
  }
}

function sessionArtifacts(): SessionArtifacts {
  return {
    files: [
      {
        path: "/repo/src/components/chat/workspace/WorkspacePanel.test.tsx",
        kind: "modified",
        readLines: 240,
        linesAdded: 210,
        linesRemoved: 45,
      },
      {
        path: "/plans/v3.5-workspace-ux-regression-note.md",
        kind: "modified",
        readLines: 84,
        linesAdded: 84,
        linesRemoved: 0,
      },
    ],
    sources: [
      {
        kind: "url",
        url: "https://code.claude.com/docs/en/hooks",
        origin: "web_search",
      },
      {
        kind: "url",
        url: "https://platform.openai.com/docs/codex",
        origin: "web_search",
      },
    ],
    browser: [],
    filesTruncated: false,
    sourcesTruncated: false,
    browserTruncated: false,
  }
}

function workspaceEnvironment(): WorkspaceEnvironmentSnapshot {
  return {
    workingDir: { path: "/repo", source: "session", exists: true, name: "hope-agent" },
    git: {
      root: "/repo",
      branch: "codex/coding-capability-roadmap",
      detached: false,
      head: "v35smok",
      worktrees: [{ path: "/repo", branch: "codex/coding-capability-roadmap", isCurrent: true }],
      status: {
        changedFiles: 4,
        stagedFiles: 0,
        unstagedFiles: 4,
        untrackedFiles: 1,
        conflictedFiles: 0,
        linesAdded: 318,
        linesRemoved: 45,
        clean: false,
      },
      sync: {
        upstream: "origin/codex/coding-capability-roadmap",
        remote: "https://example.test/hope-agent.git",
        ahead: 0,
        behind: 0,
        state: "upToDate",
      },
      lastCommit: { hash: "v35smok", subject: "添加工作台 V3.5 界面验收入口" },
    },
  }
}

function backgroundJobs(): BackgroundJobSnapshot[] {
  return [
    {
      jobId: "job-workspace-panel-tests",
      kind: "tool",
      status: "completed",
      tool: "exec",
      label: "pnpm vitest run WorkspacePanel.test.tsx",
      origin: "workflow",
      sessionId: WORKSPACE_SMOKE_SESSION_ID,
      createdAt: Date.parse(nowIso(5)),
      completedAt: Date.parse(nowIso(8)),
      error: null,
      resultPreview: "63 passed",
      resultPath: null,
      childCount: null,
      childrenTerminal: null,
      childrenCompleted: null,
      childrenFailed: null,
      subagentRunId: null,
      outputTail: "Test Files 1 passed\nTests 63 passed",
    },
  ]
}

function installWorkspaceSmokeTransport() {
  const transport = {
    call: async <T,>(command: string): Promise<T> => {
      switch (command) {
        case "get_active_goal":
          return goalSnapshot() as T
        case "list_workflow_runs":
          return [workflowRun()] as T
        case "get_workflow_run":
          return workflowSnapshot() as T
        case "list_loop_schedules":
          return [loopSchedule()] as T
        case "get_loop_schedule":
          return loopSnapshot() as T
        case "get_workflow_mode":
        case "set_workflow_mode":
          return { mode: "on" } as T
        case "get_execution_mode":
        case "set_execution_mode":
          return { mode: "guarded" } as T
        case "list_domain_evidence":
          return evidenceItems() as T
        case "evaluate_domain_operational_gate":
          return operationalGate() as T
        case "generate_domain_soak_report":
          return soakReport() as T
        case "evaluate_domain_artifact_export_guard":
          return exportGuard() as T
        case "evaluate_domain_connector_action_guard":
          return connectorGuard() as T
        case "evaluate_domain_connector_e2e_gate":
          return connectorE2E() as T
        case "list_session_kbs_cmd":
        case "list_review_runs":
        case "list_verification_runs":
        case "list_domain_quality_runs":
        case "list_managed_worktrees":
        case "list_domain_workflow_templates":
        case "list_saved_workflow_templates":
          return [] as T
        case "get_background_job":
        case "get_context_retrieval":
        case "get_lsp_status":
        case "get_lsp_diagnostics":
        case "get_coding_trend_report":
          return null as T
        default:
          return [] as T
      }
    },
    listen: () => () => {},
    loadSessionEnvironment: async () => workspaceEnvironment(),
    loadSessionArtifacts: async () => sessionArtifacts(),
    prepareFileData: (buffer: ArrayBuffer) => new Blob([buffer]),
    startChat: async () => "",
    resolveMediaUrl: () => null,
    resolveAssetUrl: (path: string | null | undefined) => path ?? null,
    openMedia: async () => {},
    downloadMedia: async () => {},
    openFilePath: async () => {},
    downloadFilePath: async () => {},
    revealMedia: async () => {},
    supportsLocalFileOps: () => true,
    pickLocalImage: async () => null,
    pickLocalDirectory: async () => null,
    previewReadText: async () => ({
      relPath: "WorkspacePanel.tsx",
      content: "",
      isBinary: false,
      mime: "text/plain",
      totalLines: 0,
      sizeBytes: 0,
      truncated: false,
    }),
    previewExtractDoc: async () => ({
      relPath: "document.txt",
      kind: "office",
      text: "",
      images: [],
    }),
    previewRawUrl: async () => null,
  } as unknown as Transport
  setTransport(transport)
}

installWorkspaceSmokeTransport()

const smokeProject: ProjectMeta = {
  id: WORKSPACE_SMOKE_PROJECT_ID,
  name: "工作台 V3.5 验收",
  createdAt: 0,
  updatedAt: 0,
  sortOrder: 0,
  archived: false,
  sessionCount: 1,
  unreadCount: 0,
}

export default function WorkspaceSmokeWindow() {
  const [wide, setWide] = useState(false)
  const sessionMeta = useMemo<SessionMeta>(
    () => ({
      id: WORKSPACE_SMOKE_SESSION_ID,
      title: "工作台 V3.5 验收",
      agentId: "ha-main",
      projectId: WORKSPACE_SMOKE_PROJECT_ID,
      createdAt: nowIso(0),
      updatedAt: nowIso(18),
      messageCount: 4,
      unreadCount: 0,
      channelUnreadCount: 0,
      hasError: false,
      pendingInteractionCount: 0,
      isCron: false,
      incognito: false,
      workingDir: "/repo",
    }),
    [],
  )

  return (
    <TooltipProvider>
      <div className="min-h-screen bg-background text-foreground">
        <header className="border-b border-border px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <h1 className="truncate text-sm font-semibold">工作台 V3.5 界面验收</h1>
              <p className="truncate text-xs text-muted-foreground">
                开发专用：默认状态故事、高级诊断和响应式布局检查。
              </p>
            </div>
            <button
              type="button"
              className="h-8 shrink-0 rounded-md border border-border bg-secondary/30 px-2 text-xs text-muted-foreground hover:bg-secondary"
              onClick={() => setWide((value) => !value)}
            >
              {wide ? "窄屏" : "宽屏"}
            </button>
          </div>
        </header>
        <main
          className={[
            "mx-auto flex h-[calc(100vh-49px)] flex-col overflow-hidden border-x border-border bg-card/30",
            wide ? "max-w-[1040px]" : "max-w-[560px]",
          ].join(" ")}
        >
          <WorkspacePanel
            taskSnapshot={createTaskProgressSnapshot(smokeTasks)}
            taskExecutionState="running"
            messages={[]}
            onOpenDiff={() => {}}
            onPreviewFile={() => {}}
            sessionId={WORKSPACE_SMOKE_SESSION_ID}
            sessionMeta={sessionMeta}
            project={smokeProject}
            effectiveWorkingDir="/repo"
            workingDirSource="session"
            permissionMode="smart"
            planState="off"
            activeModel={{ providerId: "openai", modelId: "gpt-smoke" }}
            agentName="Hope"
            reasoningEffort="medium"
            availableModels={[]}
            currentAgentId="ha-main"
            compacting={false}
            incognito={false}
            turnActive={false}
            backgroundJobs={backgroundJobs()}
            onClose={() => {}}
          />
        </main>
        <Toaster richColors position="bottom-right" />
      </div>
    </TooltipProvider>
  )
}
