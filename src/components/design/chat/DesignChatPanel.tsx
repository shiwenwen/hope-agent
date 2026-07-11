import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import { Plus, History, FileStack, Blocks, RotateCcw, GitFork } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import AgentSwitcher from "@/components/chat/AgentSwitcher"
import { useSidebarDisplayMode } from "@/components/chat/sidebar/useSidebarDisplayMode"
import { useChatStream } from "@/components/chat/hooks/useChatStream"
import { useClickOutside } from "@/hooks/useClickOutside"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { ChatAttachment } from "@/lib/transport"
import type { Message, PendingFileQuote } from "@/types/chat"
import type { DesignRecipe } from "@/types/design"
import { useDesignChat } from "./useDesignChat"
import { DesignConversationHistory } from "./DesignConversationHistory"
import { DesignToolboxPopover } from "./DesignToolboxPopover"

/** Starter prompts for the empty chat (click fills the composer, no auto-send).
 *  Both title and prompt are i18n so 12 locales stay complete; fallbacks are the
 *  zh source. Kept generic so they read well against any open artifact. */
const DESIGN_STARTERS: {
  key: string
  icon: string
  titleKey: string
  titleFallback: string
  promptKey: string
  promptFallback: string
}[] = [
  {
    // B6-2：先规划大纲再生成（对话式 outline-first，对齐参照的软性两段流）。
    key: "outline",
    icon: "🗂️",
    titleKey: "design.chat.starterOutlineTitle",
    titleFallback: "先规划大纲",
    promptKey: "design.chat.starterOutlinePrompt",
    promptFallback: "先别急着做，请先给我一份结构大纲（分节 / 分页的标题与要点、叙事顺序），我确认后你再按大纲生成正式产物。",
  },
  {
    key: "palette",
    icon: "🎨",
    titleKey: "design.chat.starterPaletteTitle",
    titleFallback: "调整配色",
    promptKey: "design.chat.starterPalettePrompt",
    promptFallback: "把整体配色调得更高级一些：主色更克制、层次更清晰、对比度可读。",
  },
  {
    key: "dark",
    icon: "🌙",
    titleKey: "design.chat.starterDarkTitle",
    titleFallback: "出深色版",
    promptKey: "design.chat.starterDarkPrompt",
    promptFallback: "基于当前设计做一个深色模式版本，保持信息层级与对比度可读。",
  },
  {
    key: "layout",
    icon: "📐",
    titleKey: "design.chat.starterLayoutTitle",
    titleFallback: "改布局",
    promptKey: "design.chat.starterLayoutPrompt",
    promptFallback: "把这个页面改成更清晰的布局：拉开留白、统一间距与字号层级。",
  },
]

/** 回合后的 next-step 引导动作（B2-1）：点击填 composer 不自动发，让用户永远知道下一步能做
 *  什么。title/prompt 均 i18n；fallback 为 zh 源。 */
const DESIGN_NEXT_STEP_ACTIONS: {
  key: string
  icon: string
  titleKey: string
  titleFallback: string
  promptKey: string
  promptFallback: string
}[] = [
  {
    key: "refine",
    icon: "✨",
    titleKey: "design.nextStep.refineTitle",
    titleFallback: "更精致",
    promptKey: "design.nextStep.refinePrompt",
    promptFallback: "把当前设计再精致一档：统一间距与字号层级、收敛配色、圆角与阴影更克制。",
  },
  {
    key: "dark",
    icon: "🌙",
    titleKey: "design.nextStep.darkTitle",
    titleFallback: "深色版",
    promptKey: "design.nextStep.darkPrompt",
    promptFallback: "基于当前设计出一个深色模式版本，保持信息层级与对比度可读。",
  },
  {
    key: "variant",
    icon: "🔀",
    titleKey: "design.nextStep.variantTitle",
    titleFallback: "出个变体",
    promptKey: "design.nextStep.variantPrompt",
    promptFallback: "另出一个不同气质的设计变体供对比（同内容、不同视觉方向）。",
  },
  {
    key: "critique",
    icon: "🔎",
    titleKey: "design.nextStep.critiqueTitle",
    titleFallback: "质量评审",
    promptKey: "design.nextStep.critiquePrompt",
    promptFallback: "对当前产物做一次质量评审，指出可改进的层级、间距、对比与可用性问题。",
  },
]

/** The design artifact the user currently has open in the preview — injected as
 *  per-turn context so "改这个 / 当前" resolves to it without the user restating. */
export interface DesignChatContext {
  id: string
  title: string
  kind: string
}

export interface DesignChatPanelHandle {
  /** Stage a selection (e.g. a preview comment) as a removable quote chip. */
  addQuote: (quote: PendingFileQuote) => void
  /** Append text/token to the composer input. */
  insertToken: (token: string) => void
  /** Stage an image File as a chat attachment (B4-1 画框批注合成图 → vision 输入)。 */
  addImageAttachment: (file: File) => void
}

interface Props {
  /** The design project this conversation is anchored to. */
  projectId: string | null
  /** Artifact currently open in the preview (per-turn context; may be null). */
  activeArtifact: DesignChatContext | null
  /** Name of the active design system, for the context note. */
  systemName?: string | null
  /** Whether the panel is actually visible (defers network loads until shown). */
  active?: boolean
  /** Click a staged quote chip → focus that element in the preview. */
  onJumpToQuote?: (q: PendingFileQuote) => void
  /** Click a "本轮产物" chip → open/focus that artifact in the preview. */
  onFocusArtifact?: (artifactId: string) => void
  /** Resolve an artifact id → title (for the Produced chip label). */
  resolveArtifactTitle?: (artifactId: string) => string | null
  /** 设计模板库（工具箱 B2-2）；空则不显示工具箱按钮。 */
  recipes?: DesignRecipe[]
  /** 形态本地化标签（工具箱分组用）。 */
  kindLabel?: (kind: string) => string
}

/** design 工具里会「产/改产物」的 action（据此从本轮 tool_calls 提取产物 chip）。 */
const DESIGN_MUTATING_ACTIONS = new Set([
  "create_artifact",
  "update_artifact",
  "edit_element",
  "restyle",
  "restore",
])

/** 从一条 assistant 消息的 design 工具调用里提取本轮产/改的 artifactId（去重、按序）。 */
function producedArtifactIds(msg: Message): string[] {
  const ids: string[] = []
  const seen = new Set<string>()
  for (const tc of msg.toolCalls ?? []) {
    if (tc.name !== "design" || !tc.result || tc.isError) continue
    let action = ""
    try {
      action = (JSON.parse(tc.arguments) as { action?: string })?.action ?? ""
    } catch {
      /* ignore */
    }
    if (!DESIGN_MUTATING_ACTIONS.has(action)) continue
    // artifactId 优先取 result；缺失回退 arguments.artifact_id（edit_element / restyle 等就地精改
    // 不一定在 result 回 artifactId）——覆盖「未回 artifactId 的产/改」漏检。
    let aid: string | undefined
    try {
      aid = (JSON.parse(tc.result) as { artifactId?: string })?.artifactId
    } catch {
      /* ignore */
    }
    if (!aid) {
      try {
        aid = (JSON.parse(tc.arguments) as { artifact_id?: string })?.artifact_id
      } catch {
        /* ignore */
      }
    }
    if (aid && !seen.has(aid)) {
      seen.add(aid)
      ids.push(aid)
    }
  }
  return ids
}

/**
 * Embedded AI chat for the design space, shown as the left rail beside the
 * artifact preview. Reuses the main chat's streaming engine (`useChatStream`) +
 * render/input components, but the session is a design thread (`useDesignChat`):
 * anchored to the open project, injected with a trimmed tool set
 * (`toolScope: "design"`), and fed the currently-open artifact as per-turn
 * context so the model edits the right thing.
 */
export const DesignChatPanel = forwardRef<DesignChatPanelHandle, Props>(function DesignChatPanel(
  {
    projectId,
    activeArtifact,
    systemName,
    active = true,
    onJumpToQuote,
    onFocusArtifact,
    resolveArtifactTitle,
    recipes,
    kindLabel,
  },
  ref,
) {
  const { t } = useTranslation()
  const isActive = active && !!projectId
  const session = useDesignChat(projectId, isActive)
  // Follow 简约模式 (sidebar compact toggle) like the main chat title bar, so the
  // design panel's agent picker renders as a compact pill when it's on.
  const sidebarDisplayMode = useSidebarDisplayMode()
  const seqRef = useRef<Map<string, number>>(new Map())
  const endedRef = useRef<Map<string, string>>(new Map())
  const [historyOpen, setHistoryOpen] = useState(false)
  const historyRef = useRef<HTMLDivElement>(null)
  useClickOutside(
    historyRef,
    useCallback(() => setHistoryOpen(false), []),
  )
  const [toolboxOpen, setToolboxOpen] = useState(false)
  const toolboxRef = useRef<HTMLDivElement>(null)
  useClickOutside(
    toolboxRef,
    useCallback(() => setToolboxOpen(false), []),
  )

  // Stable readers so the per-turn context always reflects the live open artifact.
  const artifactRef = useRef(activeArtifact)
  artifactRef.current = activeArtifact
  const systemNameRef = useRef(systemName)
  systemNameRef.current = systemName
  const projectIdRef = useRef(projectId)
  projectIdRef.current = projectId

  // Inject the currently-open artifact + design system as an invisible per-turn
  // quote so "这个 / 当前 / restyle it" resolves without the user restating which
  // artifact. Structured (not a system instruction) — the model still uses the
  // `design` tool (get_artifact / update_artifact / restyle) to actually act.
  const getExtraAttachments = useCallback((): ChatAttachment[] => {
    const art = artifactRef.current
    const pid = projectIdRef.current
    if (!art || !pid) return []
    const sys = systemNameRef.current?.trim()
    const body =
      `<design_context>\n` +
      `project_id=${pid}\n` +
      `open_artifact_id=${art.id}\n` +
      `open_artifact_title=${art.title}\n` +
      `open_artifact_kind=${art.kind}\n` +
      (sys ? `design_system=${sys}\n` : "") +
      `用户当前正在预览这个产物；「这个 / 当前 / 它」默认指它。用 design 工具的 get_artifact 读全文、` +
      `update_artifact / restyle 就地改它并出新版本；新建才用 create_artifact。\n` +
      `</design_context>`
    return [
      {
        name: `当前产物: ${art.title}`,
        mime_type: "text/plain",
        source: "quote",
        data: body,
        file_path: art.id,
      },
    ]
  }, [])

  const agentName = useMemo(
    () => session.agents.find((a) => a.id === session.currentAgentId)?.name ?? "",
    [session.agents, session.currentAgentId],
  )

  const stream = useChatStream({
    messages: session.messages,
    setMessages: session.setMessages,
    currentSessionId: session.currentSessionId,
    setCurrentSessionId: session.setCurrentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    currentAgentId: session.currentAgentId,
    agentName,
    loading: session.loading,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    sessions: session.sessions,
    agents: session.agents,
    activeModel: session.activeModel,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: seqRef,
    endedStreamIdsRef: endedRef,
    reasoningEffort: session.reasoningEffort,
    incognitoEnabled: false,
    toolScope: "design",
    draftDesignProjectId: projectId,
    getExtraAttachments,
  })

  // Reconcile against DB truth when a turn finishes (on HTTP this fills in the
  // final answer that wasn't streamed here). Merge-based + guarded.
  const prevLoadingRef = useRef(session.loading)
  useEffect(() => {
    const was = prevLoadingRef.current
    prevLoadingRef.current = session.loading
    if (was && !session.loading) {
      const sid = session.currentSessionIdRef.current
      if (sid) {
        void session.reconcileThread(sid)
        void session.reloadThreads()
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.loading])

  useImperativeHandle(
    ref,
    () => ({
      addQuote: (quote) =>
        stream.setPendingQuotes((prev) =>
          prev.some((q) => q.path === quote.path && q.content === quote.content)
            ? prev
            : [...prev, quote],
        ),
      insertToken: (token) =>
        stream.setInput((prev) => (prev.trim() ? `${prev} ${token}` : token)),
      addImageAttachment: (file) => stream.setAttachedFiles((prev) => [...prev, file]),
    }),
    [stream],
  )

  // 本轮产物 chip 条（B0-8）：从 assistant 消息的 design 工具调用里提取产/改的产物，
  // 点击聚焦到右侧预览——让「这轮到底产出了什么」在对话流里可见可达。
  const renderMessageActions = useCallback(
    (msg: Message) => {
      if (msg.role !== "assistant" || !onFocusArtifact) return null
      const ids = producedArtifactIds(msg)
      if (ids.length === 0) return null
      return (
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {ids.map((id) => (
            <button
              key={id}
              type="button"
              onClick={() => onFocusArtifact(id)}
              className="flex items-center gap-1.5 rounded-md border border-border/60 bg-card px-2 py-1 text-xs text-muted-foreground transition-colors hover:border-primary/40 hover:text-foreground"
            >
              <FileStack className="h-3.5 w-3.5 shrink-0 opacity-70" />
              <span className="max-w-[180px] truncate">
                {resolveArtifactTitle?.(id) ?? t("design.chat.producedArtifact", "本轮产物")}
              </span>
            </button>
          ))}
        </div>
      )
    },
    [onFocusArtifact, resolveArtifactTitle, t],
  )

  // ── 重新生成（Wave 1-①，收敛后的稳健版）────────────────────────
  // 只在**已有成功文本回复**（末条 assistant 且 content 非空）后，于 next-step 条首位给
  // 「重新生成」快捷键，重跑上一句 user prompt。
  // 刻意**不**做「失败/空回合」的启发式恢复条：HTTP 传输下成功回合在 reconcile 完成前
  // 本就无 assistant 正文（会误报失败刷屏），且 tool-only 回合（改了产物但无尾随文本）也
  // content='' —— 无可靠、reconcile-safe 的失败信号可判，故不判，避免在成功回合误显「无结果」
  // + 重试重复产物（对抗 review 定位的 HIGH/MED 全簇）。失败本身仍由消息流里的 error 事件呈现。
  const lastUserContent = useMemo(() => {
    for (let i = session.messages.length - 1; i >= 0; i--) {
      if (session.messages[i].role === "user") return session.messages[i].content
    }
    return ""
  }, [session.messages])
  const retryLastTurn = useCallback(() => {
    const text = lastUserContent.trim()
    if (!text || session.loading) return
    // 注：handleSend(directText) 按设计不带原回合附件（纯文本重发）；重新生成场景可接受。
    void stream.handleSend(text)
  }, [lastUserContent, session.loading, stream])

  // Fork（分支）：同项目建新会话 + 拷贝当前对话历史，切到新线程继续探索另一方向。
  const [forking, setForking] = useState(false)
  const forkThread = useCallback(async () => {
    const sid = session.currentSessionId
    if (!sid || forking) return
    setForking(true)
    try {
      const newId = await getTransport().call<string>("fork_design_thread_cmd", { sessionId: sid })
      await session.switchThread(newId)
      toast.success(t("design.chat.forked", "已分支为新对话"))
    } catch (e) {
      logger.error("design", "DesignChatPanel", "fork failed", e)
      toast.error(t("design.chat.forkFailed", "分支失败"))
    } finally {
      setForking(false)
    }
  }, [session, forking, t])

  if (!projectId) {
    return (
      <div className="flex h-full items-center justify-center p-4 text-center text-xs text-muted-foreground">
        {t("design.chat.noProject", "打开一个设计项目后即可与 AI 对话")}
      </div>
    )
  }

  const currentAgent = session.agents.find((a) => a.id === session.currentAgentId)

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      {/* Header: agent + new + history — borderless, blends with the surface. */}
      <div className="flex min-w-0 items-center gap-1 px-2 py-1.5">
        <div className="min-w-0 flex-1">
          <AgentSwitcher
            agents={session.agents}
            currentAgentId={session.currentAgentId}
            agentName={currentAgent?.name || t("chat.mainAgent")}
            compactLabel={sidebarDisplayMode === "compact"}
            onSelect={session.handleSwitchAgent}
          />
        </div>
        {recipes && recipes.length > 0 && (
          <div className="relative" ref={toolboxRef}>
            <IconTip label={t("design.toolbox.title", "设计工具箱")}>
              <Button
                variant="ghost"
                size="icon"
                className={cn("h-7 w-7", toolboxOpen && "bg-secondary")}
                onClick={() => setToolboxOpen((v) => !v)}
              >
                <Blocks className="h-4 w-4" />
              </Button>
            </IconTip>
            {toolboxOpen && (
              <DesignToolboxPopover
                recipes={recipes}
                kindLabel={kindLabel}
                onPick={(prompt) => {
                  setToolboxOpen(false)
                  stream.setInput(prompt)
                }}
              />
            )}
          </div>
        )}
        <IconTip label={t("design.chat.newConversation", "新对话")}>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={session.handleNewThread}>
            <Plus className="h-4 w-4" />
          </Button>
        </IconTip>
        {session.currentSessionId && session.messages.length > 0 && (
          <IconTip label={t("design.chat.fork", "分支这个对话")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              disabled={forking}
              onClick={() => void forkThread()}
            >
              <GitFork className="h-4 w-4" />
            </Button>
          </IconTip>
        )}
        <div className="relative" ref={historyRef}>
          <IconTip label={t("design.chat.history", "历史对话")}>
            <Button
              variant="ghost"
              size="icon"
              className={cn("h-7 w-7", historyOpen && "bg-secondary")}
              onClick={() => {
                if (!historyOpen) void session.reloadThreads("")
                setHistoryOpen((v) => !v)
              }}
            >
              <History className="h-4 w-4" />
            </Button>
          </IconTip>
          {historyOpen && (
            <DesignConversationHistory
              threads={session.threads}
              activeSessionId={session.currentSessionId}
              onSearch={(q) => session.reloadThreads(q)}
              hasMore={session.threadsHasMore}
              onLoadMore={() => void session.loadMoreThreads()}
              onPick={(sid) => {
                setHistoryOpen(false)
                void session.switchThread(sid)
              }}
              onRename={(sid, title) => {
                void getTransport()
                  .call("rename_session_cmd", { sessionId: sid, title })
                  .then(() => session.reloadThreads())
                  .catch((e) =>
                    logger.error("ui", "DesignChat::rename", "rename thread failed", e),
                  )
              }}
              onDelete={(sid) => {
                void getTransport()
                  .call("delete_session_cmd", { sessionId: sid })
                  .then(() => {
                    // 删的是当前线程 → 回到草稿态；否则仅刷新历史。
                    if (session.currentSessionIdRef.current === sid) session.handleNewThread()
                    return session.reloadThreads()
                  })
                  .catch((e) =>
                    logger.error("ui", "DesignChat::delete", "delete thread failed", e),
                  )
              }}
            />
          )}
        </div>
      </div>

      {/* Messages — height-bounded flex column so MessageList scrolls internally.
          Empty draft (no messages) shows starter prompts (click fills, no auto-send).
          A pending ask_user question forces MessageList (its footer hosts the card),
          so a restored discovery / direction-card question is never hidden by the
          empty-state starters — even if the message load errored. */}
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        {session.messages.length === 0 && !session.loading && !session.pendingQuestionGroup ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-4 overflow-y-auto p-5 text-center">
            <div>
              <p className="text-sm font-medium text-foreground">
                {activeArtifact
                  ? t("design.chat.startTitleArtifact", "跟 AI 说，直接改这个产物")
                  : t("design.chat.startTitle", "跟 AI 说一句，开始设计")}
              </p>
              <p className="mx-auto mt-1 max-w-[15rem] text-xs leading-relaxed text-muted-foreground">
                {activeArtifact
                  ? t("design.chat.startSubArtifact", "描述想要的改动，AI 就地更新并出新版本。")
                  : t("design.chat.startSub", "一句话描述，AI 直接生成可交付的设计产物。")}
              </p>
            </div>
            <div className="flex w-full max-w-[17rem] flex-col gap-1.5">
              {DESIGN_STARTERS.map((s) => (
                <button
                  key={s.key}
                  type="button"
                  onClick={() => stream.setInput(t(s.promptKey, s.promptFallback))}
                  className="group flex items-center gap-2.5 rounded-xl border border-border/60 bg-card px-3 py-2 text-left transition-all hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-sm"
                >
                  <span className="text-base">{s.icon}</span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-xs font-medium">
                      {t(s.titleKey, s.titleFallback)}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          </div>
        ) : (
          <MessageList
            messages={session.messages}
            loading={session.loading}
            agents={session.agents}
            hasMore={session.hasMore}
            loadingMore={session.loadingMore}
            onLoadMore={session.handleLoadMore}
            sessionId={session.currentSessionId}
            renderMessageActions={renderMessageActions}
            pendingQuestionGroup={session.pendingQuestionGroup}
            onQuestionSubmitted={() => session.setPendingQuestionGroup(null)}
            askUserVariant="design"
          />
        )}
      </div>

      <ApprovalDialog requests={stream.approvalRequests} onRespond={stream.handleApprovalResponse} />

      {/* Next-step 引导条（B2-1）：idle + 末条是 assistant 回复（有正文）时显示，点击填 composer
          不自动发。输入框已有内容 / 生成中不显示，避免打扰。首位「重新生成」直接重跑上一句。 */}
      {!session.loading &&
        !stream.input.trim() &&
        session.messages[session.messages.length - 1]?.role === "assistant" &&
        session.messages[session.messages.length - 1]?.content.trim() && (
          <div className="flex flex-wrap gap-1.5 px-3 pb-1.5">
            {lastUserContent.trim() && (
              <button
                type="button"
                onClick={retryLastTurn}
                className="flex items-center gap-1 rounded-full border border-border/60 px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:border-primary/40 hover:bg-accent hover:text-foreground"
              >
                <RotateCcw className="h-3 w-3" />
                {t("design.chat.regenerate", "重新生成")}
              </button>
            )}
            {DESIGN_NEXT_STEP_ACTIONS.map((a) => (
              <button
                key={a.key}
                type="button"
                onClick={() => stream.setInput(t(a.promptKey, a.promptFallback))}
                className="flex items-center gap-1 rounded-full border border-border/60 px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:border-primary/40 hover:bg-accent hover:text-foreground"
              >
                <span>{a.icon}</span>
                {t(a.titleKey, a.titleFallback)}
              </button>
            ))}
          </div>
        )}

      {/* Composer — borderless, sits on the surface like the main chat composer. */}
      <div>
        <ChatInput
          input={stream.input}
          onInputChange={stream.setInput}
          onSend={() => stream.handleSend()}
          loading={session.loading}
          availableModels={session.availableModels}
          activeModel={session.activeModel}
          reasoningEffort={session.reasoningEffort}
          onModelChange={session.handleModelChange}
          onEffortChange={session.handleEffortChange}
          attachedFiles={stream.attachedFiles}
          onAttachFiles={stream.setAttachedFiles}
          onRemoveFile={(i) =>
            stream.setAttachedFiles((prev) => prev.filter((_, idx) => idx !== i))
          }
          onUpdateFile={(index, file) =>
            stream.setAttachedFiles((prev) =>
              prev.map((existing, idx) => (idx === index ? file : existing)),
            )
          }
          pendingQuotes={stream.pendingQuotes}
          onRemoveQuote={(i) =>
            stream.setPendingQuotes((prev) => prev.filter((_, idx) => idx !== i))
          }
          onJumpToQuote={onJumpToQuote}
          pendingMessage={stream.pendingMessage}
          onCancelPending={() => stream.setPendingMessage(null)}
          // 生成中排队多条：接内核已有 pendingSends 队列 UI（逐条编辑/删除/工具边界 force-insert 插队），
          // 取代退化的单条 pending chip。与主对话一致，复用同一 useChatStream 能力。
          pendingSends={stream.pendingSends}
          onDiscardPending={() => stream.setPendingMessage(null)}
          onEditPending={stream.editPendingSend}
          onDiscardPendingItem={stream.discardPendingSend}
          onForceInsertPending={stream.forceInsertPendingSend}
          onCancelForceInsertPending={stream.cancelForceInsertPendingSend}
          onStop={stream.handleStop}
          currentSessionId={session.currentSessionId}
          currentAgentId={session.currentAgentId}
          permissionMode={stream.permissionMode}
          onPermissionModeChange={stream.setPermissionModeByUser}
          sandboxMode={stream.sandboxMode}
          onSandboxModeChange={stream.setSandboxModeByUser}
        />
      </div>
    </div>
  )
})

export default DesignChatPanel
