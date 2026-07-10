import { useMemo, useState } from "react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import ChatInput from "@/components/chat/input/ChatInput"
import { setTransport } from "@/lib/transport-provider"
import type { Transport } from "@/lib/transport"
import type { ActiveModel, AvailableModel, SandboxMode, SessionMode } from "@/types/chat"
import type { GoalSnapshot } from "@/components/chat/workspace/useGoal"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"

const CHAT_INPUT_SMOKE_SESSION_ID = "chat-input-smoke-session"

function nowIso(offsetMinutes = 0): string {
  return new Date(Date.UTC(2026, 6, 8, 12, offsetMinutes, 0)).toISOString()
}

function installChatInputSmokeTransport() {
  const transport = {
    call: async <T,>(command: string, args?: unknown): Promise<T> => {
      switch (command) {
        case "get_awareness_config":
          return { enabled: true } as T
        case "list_slash_commands":
          return [
            {
              name: "goal",
              category: "utility",
              descriptionKey: "slashCommands.goal.description",
              hasArgs: true,
              argsOptional: true,
              argOptions: ["status", "pause", "resume", "clear"],
            },
            {
              name: "workflow",
              category: "utility",
              descriptionKey: "slashCommands.workflow.description",
              hasArgs: true,
              argsOptional: true,
              argOptions: ["on", "off", "ultracode", "status"],
            },
          ] as T
        case "get_workflow_mode":
        case "set_workflow_mode": {
          const mode =
            typeof args === "object" && args !== null && "mode" in args
              ? (args as { mode?: unknown }).mode
              : "on"
          return { mode: mode === "ultracode" ? "ultracode" : mode === "off" ? "off" : "on" } as T
        }
        case "list_session_kbs_cmd":
          return [] as T
        case "check_sandbox_available":
          return { installed: true, running: true, error: null } as T
        default:
          return [] as T
      }
    },
    listen: () => () => {},
    searchFiles: async () => ({
      root: "/repo",
      matches: [
        {
          name: "ChatInput.tsx",
          path: "/repo/src/components/chat/input/ChatInput.tsx",
          relPath: "src/components/chat/input/ChatInput.tsx",
          isDir: false,
          score: 1,
        },
      ],
      truncated: false,
    }),
    listServerDirectory: async () => ({ path: "/repo", entries: [], truncated: false }),
    loadSessionEnvironment: async () => ({}),
    loadSessionArtifacts: async () => ({
      files: [],
      sources: [],
      filesTruncated: false,
      sourcesTruncated: false,
    }),
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
      relPath: "ChatInput.tsx",
      content: "",
      isBinary: false,
      mime: "text/typescript",
      totalLines: 0,
      sizeBytes: 0,
      truncated: false,
    }),
    previewExtractDoc: async () => ({ relPath: "document.txt", kind: "office", text: "", images: [] }),
    previewRawUrl: async () => null,
  } as unknown as Transport
  setTransport(transport)
}

installChatInputSmokeTransport()

const availableModels: AvailableModel[] = [
  {
    providerId: "openai",
    providerName: "OpenAI",
    apiType: "openai",
    modelId: "gpt-5.5-medium",
    modelName: "GPT-5.5 中",
    inputTypes: ["text", "image"],
    contextWindow: 256000,
    maxTokens: 8192,
    reasoning: true,
    thinkingStyle: "openai",
  },
  {
    providerId: "anthropic",
    providerName: "Anthropic",
    apiType: "anthropic",
    modelId: "claude-sonnet-workflow-ultra-long-name",
    modelName: "Claude Sonnet 工作流验证超长模型名称",
    inputTypes: ["text"],
    contextWindow: 200000,
    maxTokens: 8192,
    reasoning: true,
    thinkingStyle: "anthropic",
  },
]

const activeModel: ActiveModel = {
  providerId: "openai",
  modelId: "gpt-5.5-medium",
}

function goalSnapshot(): GoalSnapshot {
  return {
    goal: {
      id: "goal-chat-input-smoke",
      sessionId: CHAT_INPUT_SMOKE_SESSION_ID,
      objective: "验证输入框所有模式、菜单和弹层在窄屏下不裁剪、不重叠",
      completionCriteria:
        "[required] 模型菜单、工作流菜单、权限和沙箱菜单不被输入框裁剪\n[required] Goal 与 Plan 模式互斥\n[required] + 号收纳后仍能访问工作目录、附件和快捷命令",
      revision: 2,
      domain: null,
      workflowTemplateId: null,
      workflowTemplateVersion: null,
      workflowTaskType: null,
      state: "active",
      modeSnapshot: null,
      budgetTokenLimit: null,
      budgetTimeLimitSecs: null,
      budgetTurnLimit: null,
      createdAt: nowIso(0),
      updatedAt: nowIso(6),
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: {
        evaluatorKind: "post_turn",
        status: "incomplete",
        summary: "浏览器 smoke 正在检查输入框弹层。",
        nextEvidenceNeeded: ["采集窄屏模型菜单、工作流菜单和 + 号菜单截图。"],
      },
      closureDecision: null,
      closureReason: null,
      closedAt: null,
      followUpItems: [],
    },
    links: [],
    events: [],
    criteriaItems: [
      { id: "input-smoke-crit-1", text: "菜单不被裁剪", kind: "required" },
      { id: "input-smoke-crit-2", text: "模式互斥", kind: "required" },
    ],
    criteria: [],
    evidence: [],
    timeline: [],
    workflowRuns: [],
    tasks: [],
  }
}

const taskSnapshot: TaskProgressSnapshot = {
  tasks: [
    {
      id: 1,
      sessionId: CHAT_INPUT_SMOKE_SESSION_ID,
      content: "打开模型菜单、工作流菜单、+ 号菜单和权限/沙箱菜单",
      activeForm: "检查输入框弹层",
      status: "in_progress",
      createdAt: nowIso(1),
      updatedAt: nowIso(3),
    },
    {
      id: 2,
      sessionId: CHAT_INPUT_SMOKE_SESSION_ID,
      content: "保存窄屏与宽屏截图",
      activeForm: "保存截图",
      status: "pending",
      createdAt: nowIso(2),
      updatedAt: nowIso(2),
    },
  ],
  total: 2,
  completed: 0,
  remaining: 2,
  inProgress: true,
}

export default function ChatInputSmokeWindow() {
  const [input, setInput] = useState("请用 workflow 多代理复核输入框所有模式弹层，并记录可见问题")
  const [permissionMode, setPermissionMode] = useState<SessionMode>("smart")
  const [sandboxMode, setSandboxMode] = useState<SandboxMode>("off")
  const [planState, setPlanState] = useState<"off" | "planning">("off")
  const [draftWorkflowMode, setDraftWorkflowMode] = useState<"off" | "on" | "ultracode">("on")
  const [goalModeSubmitCount, setGoalModeSubmitCount] = useState(0)
  const [wide, setWide] = useState(false)
  const snapshot = useMemo(() => goalSnapshot(), [])

  const handleGoalSubmit = async () => {
    setGoalModeSubmitCount((value) => value + 1)
    return true
  }

  return (
    <TooltipProvider>
      <div className="min-h-screen bg-background text-foreground">
        <Toaster />
        <header className="border-b border-border px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <h1 className="truncate text-sm font-semibold">输入框 V3.5 弹层验收</h1>
              <p className="truncate text-xs text-muted-foreground">
                开发专用：模型、工作流、目标、计划、权限、沙箱和 + 号菜单裁剪检查。
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
        <main className="flex min-h-[calc(100vh-49px)] items-center justify-center px-3 py-10">
          <div className={wide ? "w-full max-w-[1040px]" : "w-full max-w-[430px]"}>
            <ChatInput
              input={input}
              onInputChange={setInput}
              onSend={() => {}}
              loading={false}
              availableModels={availableModels}
              activeModel={activeModel}
              reasoningEffort="medium"
              onModelChange={() => {}}
              onEffortChange={() => {}}
              attachedFiles={[]}
              onAttachFiles={() => {}}
              onRemoveFile={() => {}}
              currentSessionId={CHAT_INPUT_SMOKE_SESSION_ID}
              currentAgentId="ha-main"
              onEnsureSession={async () => CHAT_INPUT_SMOKE_SESSION_ID}
              permissionMode={permissionMode}
              onPermissionModeChange={(mode) => setPermissionMode(mode)}
              sandboxMode={sandboxMode}
              onSandboxModeChange={setSandboxMode}
              sessionTemperature={0.7}
              onSessionTemperatureChange={() => {}}
              incognitoEnabled={false}
              projectId="input-smoke-project"
              workingDir="/repo"
              onWorkingDirChange={() => {}}
              planState={planState}
              onEnterPlanMode={() => setPlanState("planning")}
              onExitPlanMode={() => setPlanState("off")}
              draftWorkflowMode={draftWorkflowMode}
              onDraftWorkflowModeChange={setDraftWorkflowMode}
              goalSnapshot={snapshot}
              onGoalModeSubmit={handleGoalSubmit}
              onLoopModeSubmit={async () => true}
              onGoalUpdate={async () => true}
              onPauseGoal={async () => true}
              onResumeGoal={async () => true}
              onClearGoal={async () => true}
              onEvaluateGoal={async () => true}
              taskProgressSnapshot={taskSnapshot}
              executionState="running"
              onOpenWorkspace={() => {}}
              contextUsage={{ usedTokens: 48000, contextWindow: 128000, usedK: 48, ctxK: 128, pct: 38 }}
            />
            <div className="mt-3 rounded-lg border border-border bg-card/70 p-3 text-xs text-muted-foreground">
              <div>目标提交次数：{goalModeSubmitCount}</div>
              <div>权限：{permissionMode} · 沙箱：{sandboxMode} · 工作流：{draftWorkflowMode}</div>
            </div>
          </div>
        </main>
      </div>
    </TooltipProvider>
  )
}
