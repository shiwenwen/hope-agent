import { useState } from "react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import WorkspacePanel from "@/components/chat/workspace/WorkspacePanel"
import { setTransport } from "@/lib/transport-provider"
import type { Transport, WorkspaceEnvironmentSnapshot, SessionArtifacts } from "@/lib/transport"
import type {
  WorkflowRun,
  WorkflowRunSnapshot,
  WorkflowRunState,
  WorkflowScriptPreview,
} from "@/components/chat/workspace/useWorkflowRuns"
import type { SessionMeta } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

type SmokeScenario = "approval" | "running" | "failed" | "completed"

const SMOKE_SESSION_PREFIX = "workflow-smoke"

function workflowRun(scenario: SmokeScenario, patch: Partial<WorkflowRun> = {}): WorkflowRun {
  const stateByScenario: Record<SmokeScenario, WorkflowRunState> = {
    approval: "awaiting_approval",
    running: "running",
    failed: "failed",
    completed: "completed",
  }
  return {
    id: `${SMOKE_SESSION_PREFIX}-${scenario}-run`,
    sessionId: `${SMOKE_SESSION_PREFIX}-${scenario}`,
    kind: scenario === "failed" ? "general.repair" : "general.workflow",
    state: stateByScenario[scenario],
    executionMode: "guarded",
    scriptHash: "abcdef123456",
    scriptSource: "export default async function main(workflow) {}",
    budget: { maxScriptSecs: 180, maxOps: 24 },
    cursorSeq: 4,
    primaryOwner: null,
    blockedReason: null,
    parentRunId: scenario === "failed" ? "workflow-smoke-parent" : null,
    origin: scenario === "failed" ? "repair" : null,
    createdAt: "2026-07-01T01:00:00.000Z",
    updatedAt: "2026-07-01T01:05:00.000Z",
    completedAt: scenario === "completed" ? "2026-07-01T01:05:00.000Z" : null,
    ...patch,
  }
}

function workflowSnapshot(scenario: SmokeScenario): WorkflowRunSnapshot {
  const run = workflowRun(scenario)
  const baseOps: WorkflowRunSnapshot["ops"] = [
    {
      id: "op-observe",
      runId: run.id,
      opKey: "main/op#1(workflow.fileSearch)",
      opType: "fileSearch",
      effectClass: "pure",
      inputHash: "hash-observe",
      input: { query: "Workflow GUI smoke", label: "target-files" },
      state: "completed",
      output: {
        summary: "Found 4 likely files",
        matches: ["WorkspacePanel.tsx", "workflow/db.rs"],
      },
      error: null,
      childHandle: null,
      startedAt: "2026-07-01T01:00:10.000Z",
      completedAt: "2026-07-01T01:00:20.000Z",
    },
    {
      id: "op-agent",
      runId: run.id,
      opKey: "main/op#2(workflow.spawnAgent)",
      opType: "spawnAgent",
      effectClass: "non_idempotent",
      inputHash: "hash-agent",
      input: { label: "implement-target", task: "Tighten Workspace workflow UX" },
      state: scenario === "approval" ? "pending" : "completed",
      output:
        scenario === "approval"
          ? null
          : {
              label: "implement-target",
              status: "completed",
              runId: "subagent-smoke-1",
              sessionId: "subagent-session-smoke",
              task: "Tighten Workspace workflow UX",
            },
      error: null,
      childHandle: "subagent-smoke-1",
      startedAt: "2026-07-01T01:01:00.000Z",
      completedAt: scenario === "approval" ? null : "2026-07-01T01:02:10.000Z",
    },
    {
      id: "op-validate",
      runId: run.id,
      opKey: "main/op#3(workflow.validate)",
      opType: "validate",
      effectClass: "non_idempotent",
      inputHash: "hash-validate",
      input: { label: "targeted-validation", commands: ["pnpm typecheck"] },
      state: scenario === "running" ? "started" : "completed",
      output: {
        ok: scenario !== "failed",
        summary:
          scenario === "failed"
            ? "1/1 validation command(s) failed"
            : "1/1 validation command(s) passed",
        results: [
          {
            command: "pnpm typecheck",
            cwd: "/repo",
            jobStatus: scenario === "running" ? "running" : "completed",
            ok: scenario !== "failed",
            exitCode: scenario === "running" ? null : scenario === "failed" ? 1 : 0,
            output: scenario === "failed" ? "Type error in WorkflowRunFocusCard props" : "ok",
          },
        ],
      },
      error: null,
      childHandle: "job-smoke-validate",
      startedAt: "2026-07-01T01:02:20.000Z",
      completedAt: scenario === "running" ? null : "2026-07-01T01:03:00.000Z",
    },
  ]

  return {
    run,
    ops: baseOps,
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
              line: 12,
              toolName: "edit",
              decision: "ask",
              strict: true,
              dynamic: false,
              reason: "edit-class tool requires approval",
              label: "apply-ui-polish",
              args: { path: "src/components/chat/workspace/WorkspacePanel.tsx" },
            },
            {
              api: "workflow.validate",
              line: 24,
              toolName: "exec",
              decision: "allow",
              strict: false,
              dynamic: false,
              reason: "targeted validation",
              label: "pnpm-typecheck",
              args: { command: "pnpm typecheck" },
            },
          ],
          truncated: false,
        },
        createdAt: "2026-07-01T01:00:00.000Z",
      },
      {
        id: 2,
        runId: run.id,
        seq: 2,
        eventType: scenario === "approval" ? "script_permission_approval_required" : "op_started",
        payload:
          scenario === "approval"
            ? { summary: { total: 2, ask: 1, dynamic: 1, deny: 0, strict: 1 } }
            : { opKey: "main/op#3(workflow.validate)", opType: "validate", state: "started" },
        createdAt: "2026-07-01T01:02:20.000Z",
      },
      ...(scenario === "failed"
        ? [
            {
              id: 3,
              runId: run.id,
              seq: 3,
              eventType: "guarded_repair_validation_failed",
              payload: {
                opKey: "main/op#3(workflow.validate)",
                summary: "1/1 validation command(s) failed",
                failed: 1,
                total: 1,
                stopReason: "validation_failed",
              },
              createdAt: "2026-07-01T01:03:00.000Z",
            },
            {
              id: 4,
              runId: run.id,
              seq: 4,
              eventType: "run_derived_child_created",
              payload: {
                parentRunId: run.id,
                childRunId: "workflow-smoke-repair-child",
                origin: "repair",
              },
              createdAt: "2026-07-01T01:04:00.000Z",
            },
          ]
        : []),
    ],
  }
}

function workflowScriptPreview(): WorkflowScriptPreview {
  return {
    gate: { issues: [] },
    gatePassed: true,
    gateFeedback: "Workflow Script Gate passed.",
    permission: {
      summary: { total: 2, allow: 1, ask: 1, dynamic: 0, deny: 0, strict: 1 },
      calls: [
        {
          api: "workflow.validate",
          line: 24,
          toolName: "exec",
          decision: "ask",
          strict: true,
          dynamic: false,
          label: "targeted-validation",
          args: { command: "pnpm typecheck" },
        },
      ],
      truncated: false,
    },
    canCreate: true,
    canRunImmediately: true,
    requiresApproval: true,
    hasDenials: false,
  }
}

function sessionArtifacts(): SessionArtifacts {
  return {
    files: [
      {
        path: "/repo/src/components/chat/workspace/WorkspacePanel.tsx",
        kind: "modified",
        readLines: 120,
        linesAdded: 42,
        linesRemoved: 18,
      },
    ],
    sources: [
      {
        kind: "url",
        url: "https://example.test/workflow-smoke",
        origin: "message",
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
    workingDir: { path: "/repo", source: "session", exists: true, name: "repo" },
    git: {
      root: "/repo",
      branch: "phase2-product",
      detached: false,
      head: "abcdef1",
      worktrees: [{ path: "/repo", branch: "phase2-product", isCurrent: true }],
      status: {
        changedFiles: 3,
        stagedFiles: 0,
        unstagedFiles: 3,
        untrackedFiles: 0,
        conflictedFiles: 0,
        linesAdded: 128,
        linesRemoved: 24,
        clean: false,
      },
      sync: {
        upstream: "origin/phase2-product",
        remote: "https://example.test/repo.git",
        ahead: 1,
        behind: 0,
        state: "ahead",
      },
      lastCommit: { hash: "abcdef1", subject: "Polish workflow control center" },
    },
  }
}

function installSmokeTransport() {
  const transport = {
    call: async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      const sessionId =
        typeof args?.sessionId === "string" ? args.sessionId : `${SMOKE_SESSION_PREFIX}-approval`
      const scenario = smokeScenarioFromSessionId(sessionId)
      switch (command) {
        case "list_workflow_runs":
          return [workflowRun(scenario)] as T
        case "get_workflow_run":
          return workflowSnapshot(smokeScenarioFromRunId(String(args?.runId ?? ""))) as T
        case "get_execution_mode":
        case "set_execution_mode":
          return { mode: args?.mode ?? "guarded" } as T
        case "get_workflow_mode":
        case "set_workflow_mode":
          return { mode: args?.mode ?? "on" } as T
        case "preview_workflow_script":
          return workflowScriptPreview() as T
        case "create_workflow_run":
          return workflowRun("running", {
            id: "workflow-smoke-created-run",
            sessionId,
            state: args?.runImmediately ? "running" : "draft",
            parentRunId: typeof args?.parentRunId === "string" ? args.parentRunId : null,
            origin: typeof args?.origin === "string" ? args.origin : null,
          }) as T
        case "run_workflow_run":
        case "approve_workflow_run":
        case "resume_workflow_run":
          return workflowRun(scenario, { state: "running" }) as T
        case "pause_workflow_run":
          return workflowRun(scenario, { state: "paused" }) as T
        case "cancel_workflow_run":
          return workflowRun(scenario, { state: "cancelled" }) as T
        case "list_session_kbs_cmd":
          return [] as T
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

function smokeScenarioFromSessionId(sessionId: string): SmokeScenario {
  if (sessionId.endsWith("-running")) return "running"
  if (sessionId.endsWith("-failed")) return "failed"
  if (sessionId.endsWith("-completed")) return "completed"
  return "approval"
}

function smokeScenarioFromRunId(runId: string): SmokeScenario {
  if (runId.includes("running")) return "running"
  if (runId.includes("failed")) return "failed"
  if (runId.includes("completed")) return "completed"
  return "approval"
}

const scenarioLabels: Record<SmokeScenario, string> = {
  approval: "Approval",
  running: "Running validation",
  failed: "Validation failed",
  completed: "Completed",
}

const smokeProject: ProjectMeta = {
  id: "workflow-smoke-project",
  name: "Workflow Smoke",
  createdAt: 0,
  updatedAt: 0,
  sortOrder: 0,
  archived: false,
  sessionCount: 1,
  unreadCount: 0,
}

installSmokeTransport()

export default function WorkflowSmokeWindow() {
  const [scenario, setScenario] = useState<SmokeScenario>("approval")
  const sessionId = `${SMOKE_SESSION_PREFIX}-${scenario}`
  const sessionMeta: SessionMeta = {
    id: sessionId,
    title: `Workflow smoke: ${scenario}`,
    agentId: "ha-main",
    createdAt: "2026-07-01T01:00:00.000Z",
    updatedAt: "2026-07-01T01:05:00.000Z",
    messageCount: 0,
    unreadCount: 0,
    channelUnreadCount: 0,
    hasError: false,
    pendingInteractionCount: 0,
    isCron: false,
    incognito: false,
    workingDir: "/repo",
  }

  return (
    <TooltipProvider>
      <div className="min-h-screen bg-background text-foreground">
        <header className="border-b border-border px-3 py-2">
          <div className="space-y-2">
            <div className="min-w-0">
              <h1 className="truncate text-sm font-semibold">Workflow GUI Smoke</h1>
              <p className="truncate text-xs text-muted-foreground">
                Dev-only viewport harness for Workspace workflow UI
              </p>
            </div>
            <div className="grid grid-cols-2 gap-1.5 sm:flex sm:flex-wrap">
              {Object.entries(scenarioLabels).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={[
                    "h-8 min-w-0 rounded-md border px-2 text-xs transition-colors",
                    scenario === key
                      ? "border-primary/60 bg-primary/10 text-foreground"
                      : "border-border bg-secondary/30 text-muted-foreground hover:bg-secondary",
                  ].join(" ")}
                  onClick={() => setScenario(key as SmokeScenario)}
                >
                  <span className="block truncate">{label}</span>
                </button>
              ))}
            </div>
          </div>
        </header>
        <main className="mx-auto flex h-[calc(100vh-89px)] max-w-[560px] flex-col overflow-hidden border-x border-border bg-card/30 sm:h-[calc(100vh-57px)]">
          <WorkspacePanel
            taskSnapshot={null}
            messages={[]}
            onOpenDiff={() => {}}
            onPreviewFile={() => {}}
            sessionId={sessionId}
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
            turnActive={scenario === "running"}
            backgroundJobs={[]}
            onClose={() => {}}
          />
        </main>
        <Toaster richColors position="bottom-right" />
      </div>
    </TooltipProvider>
  )
}
