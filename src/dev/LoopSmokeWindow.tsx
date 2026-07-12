import { useMemo, useState } from "react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import WorkspacePanel from "@/components/chat/workspace/WorkspacePanel"
import { setTransport } from "@/lib/transport-provider"
import type { Transport, WorkspaceEnvironmentSnapshot, SessionArtifacts } from "@/lib/transport"
import type { SessionMeta } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import type { GoalSnapshot } from "@/components/chat/workspace/useGoal"
import type {
  LoopSchedule,
  LoopSnapshot,
  LoopTriggerKind,
} from "@/components/chat/workspace/useLoopSchedules"

const LOOP_SMOKE_SESSION_ID = "loop-smoke-session"

function nowIso(offsetMinutes = 0): string {
  return new Date(Date.UTC(2026, 6, 8, 10, offsetMinutes, 0)).toISOString()
}

function loopSchedule(patch: Partial<LoopSchedule> = {}): LoopSchedule {
  return {
    id: "loop-smoke-dynamic",
    sessionId: LOOP_SMOKE_SESSION_ID,
    goalId: "goal-loop-smoke",
    goalCriterionId: null,
    goalCriterionText: null,
    goalCriterionKind: null,
    goalRevision: 1,
    cronJobId: "cron-loop-smoke",
    prompt: "Keep checking whether the release checklist is ready and report the next useful step.",
    triggerKind: "dynamic",
    triggerSpec: {
      fallbackSecs: 1200,
      fallbackUsed: false,
      maintenancePrompt: {
        enabled: true,
        source: "loop_md",
        path: "/repo/loop.md",
        contentHash: "loop-smoke-hash",
      },
    },
    executionStrategy: "continue",
    state: "active",
    maxRuns: null,
    runCount: 2,
    maxRuntimeSecs: null,
    tokenBudget: null,
    costBudgetMicros: null,
    progressState: "progressed",
    progressSummary: "Checked release status and scheduled another pass.",
    noProgressStreak: 0,
    failureStreak: 0,
    maxNoProgressRuns: 3,
    maxFailures: 3,
    backoffSecs: 300,
    nextRunAt: nowIso(25),
    cronStatus: "active",
    approvalPolicySnapshot: {},
    createdAt: nowIso(0),
    updatedAt: nowIso(10),
    completedAt: null,
    blockedReason: null,
    ...patch,
  }
}

function loopSnapshot(schedule: LoopSchedule): LoopSnapshot {
  return {
    schedule,
    runs: [
      {
        id: "lrun-loop-smoke-2",
        loopId: schedule.id,
        cronJobId: schedule.cronJobId,
        cronRunLogId: 2,
        sessionId: schedule.sessionId,
        seq: 2,
        state: "succeeded",
        triggerReason: "dynamic trigger from cron job cron-loop-smoke",
        resultSummary: "Release checklist still has one verification item, so the loop will check again.",
        error: null,
        progressState: "progressed",
        progressDelta: {},
        noProgressReason: null,
        schedulingDecision: "dynamic_reschedule_900s",
        trace: {
          triggerKind: "dynamic",
          maintenancePrompt: {
            enabled: true,
            source: "loop_md",
            path: "/repo/loop.md",
            contentHash: "loop-smoke-hash",
          },
          dynamicDecision: {
            source: "tool",
            action: "reschedule",
            delaySecs: 900,
            reason: "Waiting for the final release verification to finish.",
          },
        },
        startedAt: nowIso(9),
        finishedAt: nowIso(10),
      },
    ],
  }
}

function goalSnapshot(): GoalSnapshot {
  return {
    goal: {
      id: "goal-loop-smoke",
      sessionId: LOOP_SMOKE_SESSION_ID,
      objective: "Finish the release checklist",
      completionCriteria: "All required release checklist items are verified.",
      revision: 1,
      domain: "project_ops",
      workflowTemplateId: null,
      workflowTemplateVersion: null,
      workflowTaskType: null,
      state: "active",
      modeSnapshot: null,
      budgetTokenLimit: null,
      budgetTimeLimitSecs: null,
      budgetTurnLimit: null,
      createdAt: nowIso(0),
      updatedAt: nowIso(10),
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: null,
      closureDecision: null,
      closureReason: null,
      closedAt: null,
      followUpItems: [],
    },
    links: [],
    events: [],
    criteriaItems: [
      {
        id: "crit-release-ready",
        text: "Release checklist is verified",
        kind: "required",
      },
    ],
    criteria: [],
    evidence: [],
    timeline: [],
    workflowRuns: [],
    tasks: [],
  }
}

function sessionArtifacts(): SessionArtifacts {
  return {
    files: [],
    sources: [],
    browser: [],
    filesTruncated: false,
    sourcesTruncated: false,
    browserTruncated: false,
  }
}

function workspaceEnvironment(): WorkspaceEnvironmentSnapshot {
  return {
    workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
    git: {
      root: "/repo",
      branch: "loop-v3-smoke",
      detached: false,
      head: "1234567",
      worktrees: [{ path: "/repo", branch: "loop-v3-smoke", isCurrent: true }],
      status: {
        changedFiles: 1,
        stagedFiles: 0,
        unstagedFiles: 1,
        untrackedFiles: 0,
        conflictedFiles: 0,
        linesAdded: 24,
        linesRemoved: 2,
        clean: false,
      },
      sync: {
        upstream: "origin/loop-v3-smoke",
        remote: "https://example.test/repo.git",
        ahead: 0,
        behind: 0,
        state: "upToDate",
      },
      lastCommit: { hash: "1234567", subject: "Loop v3 smoke fixture" },
    },
  }
}

function installLoopSmokeTransport() {
  let loops: LoopSchedule[] = [loopSchedule()]
  const transport = {
    call: async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      switch (command) {
        case "get_active_goal":
          return goalSnapshot() as T
        case "list_loop_schedules":
          return loops as T
        case "get_loop_schedule": {
          const loopId = String(args?.loopId ?? "")
          const schedule = loops.find((item) => item.id === loopId) ?? loops[0]
          return loopSnapshot(schedule) as T
        }
        case "create_loop_schedule": {
          const created = loopSchedule({
            id: "loop-smoke-created",
            cronJobId: "cron-loop-smoke-created",
            prompt: typeof args?.prompt === "string" ? args.prompt : "",
            triggerKind:
              typeof args?.triggerKind === "string"
                ? (args.triggerKind as LoopTriggerKind)
                : "dynamic",
            triggerSpec:
              typeof args?.triggerSpec === "object" && args.triggerSpec !== null
                ? (args.triggerSpec as Record<string, unknown>)
                : { fallbackSecs: 1200, fallbackUsed: false },
            goalId: typeof args?.goalId === "string" ? args.goalId : "goal-loop-smoke",
            runCount: 0,
            progressState: null,
            progressSummary: null,
            nextRunAt: nowIso(30),
            createdAt: nowIso(12),
            updatedAt: nowIso(12),
          })
          loops = [created, ...loops.filter((item) => item.id !== created.id)]
          return created as T
        }
        case "run_loop_schedule_now":
        case "pause_loop_schedule":
        case "resume_loop_schedule":
        case "stop_loop_schedule":
        case "update_loop_schedule_policy":
          return loops[0] as T
        case "list_workflow_runs":
        case "list_session_kbs_cmd":
          return [] as T
        case "get_execution_mode":
        case "set_execution_mode":
          return { mode: args?.mode ?? "guarded" } as T
        case "get_workflow_mode":
        case "set_workflow_mode":
          return { mode: args?.mode ?? "off" } as T
        case "get_background_job":
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
      relPath: "loop.md",
      content: "Keep checking whether the release checklist is ready.",
      isBinary: false,
      mime: "text/markdown",
      totalLines: 1,
      sizeBytes: 54,
      truncated: false,
    }),
    previewExtractDoc: async () => ({ relPath: "document.txt", kind: "office", text: "", images: [] }),
    previewRawUrl: async () => null,
  } as unknown as Transport
  setTransport(transport)
}

installLoopSmokeTransport()

const smokeProject: ProjectMeta = {
  id: "loop-smoke-project",
  name: "Loop Smoke",
  createdAt: 0,
  updatedAt: 0,
  sortOrder: 0,
  archived: false,
  sessionCount: 1,
  unreadCount: 0,
  memoryCount: 0,
}

export default function LoopSmokeWindow() {
  const [wide, setWide] = useState(false)
  const sessionMeta = useMemo<SessionMeta>(
    () => ({
      id: LOOP_SMOKE_SESSION_ID,
      title: "Loop v3 smoke",
      agentId: "ha-main",
      createdAt: nowIso(0),
      updatedAt: nowIso(10),
      messageCount: 0,
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
              <h1 className="truncate text-sm font-semibold">Loop V3 GUI Smoke</h1>
              <p className="truncate text-xs text-muted-foreground">
                Dev-only Workspace harness for dynamic Loop creation and decision reason display
              </p>
            </div>
            <button
              type="button"
              className="h-8 shrink-0 rounded-md border border-border bg-secondary/30 px-2 text-xs text-muted-foreground hover:bg-secondary"
              onClick={() => setWide((value) => !value)}
            >
              {wide ? "Narrow" : "Wide"}
            </button>
          </div>
        </header>
        <main
          className={[
            "mx-auto flex h-[calc(100vh-49px)] flex-col overflow-hidden border-x border-border bg-card/30",
            wide ? "max-w-[960px]" : "max-w-[560px]",
          ].join(" ")}
        >
          <WorkspacePanel
            taskSnapshot={null}
            messages={[]}
            onOpenDiff={() => {}}
            onPreviewFile={() => {}}
            sessionId={LOOP_SMOKE_SESSION_ID}
            sessionMeta={sessionMeta}
            project={smokeProject}
            effectiveWorkingDir="/repo"
            workingDirSource="session"
            permissionMode="default"
            planState="off"
            activeModel={{ providerId: "openai", modelId: "gpt-smoke" }}
            agentName="Hope"
            reasoningEffort="medium"
            availableModels={[]}
            currentAgentId="ha-main"
            compacting={false}
            incognito={false}
            turnActive={false}
            backgroundJobs={[]}
            onClose={() => {}}
          />
        </main>
        <Toaster richColors position="bottom-right" />
      </div>
    </TooltipProvider>
  )
}
