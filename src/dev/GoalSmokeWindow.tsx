import { useMemo, useState } from "react"
import { CheckCircle2, Target } from "lucide-react"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import ChatInput from "@/components/chat/input/ChatInput"
import { parseGoalObjectiveAndCriteria } from "@/components/chat/goalSlashCommand"
import { setTransport } from "@/lib/transport-provider"
import type { ChatStartArgs, Transport } from "@/lib/transport"
import type { ActiveModel, AvailableModel } from "@/types/chat"
import type { GoalSnapshot } from "@/components/chat/workspace/useGoal"

const GOAL_SMOKE_SESSION_ID = "goal-smoke-session"

function nowIso(offsetMinutes = 0): string {
  return new Date(Date.UTC(2026, 6, 8, 11, offsetMinutes, 0)).toISOString()
}

function goalTurnPrompt(visibleGoalText: string): string {
  return [
    "[SYSTEM: The user has just created or updated the durable Goal for this session.",
    "Treat the Active Goal system section as the source of truth, acknowledge briefly, then begin making progress.",
    "Do not expose internal goal ids, revision ids, or slash-command help unless the user asks for status details.]",
    "",
    visibleGoalText,
  ].join("\n")
}

function goalSnapshot(objective: string, completionCriteria: string): GoalSnapshot {
  return {
    goal: {
      id: "goal-smoke-first-turn",
      sessionId: GOAL_SMOKE_SESSION_ID,
      objective,
      completionCriteria,
      revision: 1,
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
      updatedAt: nowIso(0),
      completedAt: null,
      finalSummary: null,
      finalEvidence: {},
      blockedReason: null,
      lastEvaluatorResult: {
        evaluatorKind: "post_turn",
        status: "incomplete",
        summary: "First turn has started; evidence is not complete yet.",
        nextEvidenceNeeded: ["Run the first model turn with the Active Goal injected."],
      },
      closureDecision: null,
      closureReason: null,
      closedAt: null,
      followUpItems: [],
    },
    links: [],
    events: [],
    criteriaItems: completionCriteria
      ? [
          {
            id: "crit-goal-smoke",
            text: completionCriteria,
            kind: "required",
          },
        ]
      : [],
    criteria: [],
    evidence: [],
    timeline: [],
    workflowRuns: [],
    tasks: [],
  }
}

function installGoalSmokeTransport() {
  const transport = {
    call: async <T,>(command: string): Promise<T> => {
      switch (command) {
        case "get_awareness_config":
          return { enabled: false } as T
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
          ] as T
        case "get_workflow_mode":
        case "set_workflow_mode":
          return { mode: "off" } as T
        case "list_session_kbs_cmd":
          return [] as T
        default:
          return [] as T
      }
    },
    listen: () => () => {},
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
      relPath: "goal.md",
      content: "",
      isBinary: false,
      mime: "text/markdown",
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

installGoalSmokeTransport()

const activeModel: ActiveModel = { providerId: "openai", modelId: "gpt-smoke" }
const availableModels: AvailableModel[] = [
  {
    providerId: "openai",
    providerName: "OpenAI",
    apiType: "openai",
    modelId: "gpt-smoke",
    modelName: "GPT Smoke",
    inputTypes: ["text"],
    contextWindow: 128000,
    maxTokens: 4096,
    reasoning: true,
    thinkingStyle: "openai",
  },
]

type SmokeResult = {
  visibleText: string
  startChatArgs: Pick<ChatStartArgs, "message" | "displayText" | "goalTrigger" | "initialGoal">
  ensureSessionCalls: number
}

export default function GoalSmokeWindow() {
  const [input, setInput] = useState(
    "/goal 完成 Goal 首轮启动验证 --criteria 不提前创建空会话，并在第一轮模型执行前创建 durable Goal",
  )
  const [ensureSessionCalls, setEnsureSessionCalls] = useState(0)
  const [result, setResult] = useState<SmokeResult | null>(null)
  const snapshot = useMemo(() => {
    const initialGoal = result?.startChatArgs.initialGoal
    if (!initialGoal) return null
    return goalSnapshot(initialGoal.objective, initialGoal.completionCriteria ?? "")
  }, [result])

  const handleGoalModeSubmit = async (objective: string): Promise<boolean> => {
    const parsed = parseGoalObjectiveAndCriteria(objective)
    const initialObjective = parsed.objective || objective.trim()
    const initialCriteria = parsed.completionCriteria.trim()
    const promptGoalText = initialCriteria
      ? `${initialObjective}\n\nCompletion criteria:\n${initialCriteria}`
      : initialObjective
    const startChatArgs: SmokeResult["startChatArgs"] = {
      message: goalTurnPrompt(promptGoalText),
      displayText: objective.trim(),
      goalTrigger: true,
      initialGoal: {
        objective: initialObjective,
        completionCriteria: initialCriteria || undefined,
      },
    }
    setResult({
      visibleText: objective.trim(),
      startChatArgs,
      ensureSessionCalls,
    })
    return true
  }

  return (
    <TooltipProvider>
      <div className="min-h-screen bg-background text-foreground">
        <main className="mx-auto flex min-h-screen max-w-[980px] flex-col gap-4 px-4 py-6">
          <header className="space-y-1">
            <h1 className="text-base font-semibold">Goal V3.3 First-Turn Smoke</h1>
            <p className="text-sm text-muted-foreground">
              Dev-only harness for slash-style Goal input, first-turn initialGoal payload, and
              composer status.
            </p>
          </header>

          <section className="rounded-lg border border-border bg-card p-4 shadow-sm">
            <div className="mb-3 flex items-center gap-2 text-sm font-medium">
              <Target className="h-4 w-4 text-emerald-600" />
              Composer
            </div>
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
              onUpdateFile={() => {}}
              currentSessionId={null}
              currentAgentId="ha-main"
              onEnsureSession={async () => {
                setEnsureSessionCalls((value) => value + 1)
                return GOAL_SMOKE_SESSION_ID
              }}
              permissionMode="default"
              onPermissionModeChange={() => {}}
              sandboxMode="off"
              onSandboxModeChange={() => {}}
              goalSnapshot={snapshot}
              onGoalModeSubmit={handleGoalModeSubmit}
              hero={false}
            />
          </section>

          <section className="grid gap-3 md:grid-cols-[1fr_1fr]">
            <div className="rounded-lg border border-border bg-card p-4 shadow-sm">
              <div className="mb-2 flex items-center gap-2 text-sm font-medium">
                <CheckCircle2 className="h-4 w-4 text-emerald-600" />
                Assertions
              </div>
              <dl className="space-y-2 text-sm">
                <div className="flex items-center justify-between gap-3">
                  <dt className="text-muted-foreground">startChat calls</dt>
                  <dd data-testid="goal-smoke-start-chat-calls">{result ? 1 : 0}</dd>
                </div>
                <div className="flex items-center justify-between gap-3">
                  <dt className="text-muted-foreground">ensureSession calls</dt>
                  <dd data-testid="goal-smoke-ensure-session-calls">
                    {result?.ensureSessionCalls ?? ensureSessionCalls}
                  </dd>
                </div>
                <div className="flex items-center justify-between gap-3">
                  <dt className="text-muted-foreground">goalTrigger</dt>
                  <dd data-testid="goal-smoke-goal-trigger">
                    {result?.startChatArgs.goalTrigger ? "true" : "false"}
                  </dd>
                </div>
                <div className="flex items-center justify-between gap-3">
                  <dt className="text-muted-foreground">initialGoal</dt>
                  <dd data-testid="goal-smoke-initial-goal">
                    {result?.startChatArgs.initialGoal ? "present" : "missing"}
                  </dd>
                </div>
              </dl>
            </div>

            <div className="rounded-lg border border-border bg-card p-4 shadow-sm">
              <div className="mb-2 text-sm font-medium">Rendered Message</div>
              {result ? (
                <div
                  className="ml-auto max-w-[520px] rounded-lg bg-secondary px-4 py-3 text-sm"
                  data-testid="goal-smoke-user-bubble"
                >
                  <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-emerald-700">
                    <Target className="h-3.5 w-3.5" />
                    目标
                  </div>
                  <p className="whitespace-pre-wrap">{result.visibleText}</p>
                  <p className="mt-2 text-right text-xs text-muted-foreground">first turn</p>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Click send to submit the slash-style Goal draft.
                </p>
              )}
            </div>
          </section>

          {result?.startChatArgs.initialGoal ? (
            <section className="rounded-lg border border-emerald-200 bg-emerald-50 p-4 text-sm text-emerald-950">
              <div className="mb-2 font-medium">Captured initialGoal payload</div>
              <pre className="overflow-auto rounded-md bg-white/70 p-3 text-xs">
                {JSON.stringify(result.startChatArgs, null, 2)}
              </pre>
            </section>
          ) : null}
        </main>
        <Toaster richColors position="bottom-right" />
      </div>
    </TooltipProvider>
  )
}
