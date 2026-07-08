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
import { Plus, History } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import AgentSwitcher from "@/components/chat/AgentSwitcher"
import { useChatStream } from "@/components/chat/hooks/useChatStream"
import { useClickOutside } from "@/hooks/useClickOutside"
import type { ChatAttachment } from "@/lib/transport"
import type { PendingFileQuote } from "@/types/chat"
import { useDesignChat } from "./useDesignChat"
import { DesignConversationHistory } from "./DesignConversationHistory"

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
  { projectId, activeArtifact, systemName, active = true, onJumpToQuote },
  ref,
) {
  const { t } = useTranslation()
  const isActive = active && !!projectId
  const session = useDesignChat(projectId, isActive)
  const seqRef = useRef<Map<string, number>>(new Map())
  const endedRef = useRef<Map<string, string>>(new Map())
  const [historyOpen, setHistoryOpen] = useState(false)
  const historyRef = useRef<HTMLDivElement>(null)
  useClickOutside(
    historyRef,
    useCallback(() => setHistoryOpen(false), []),
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
    }),
    [stream],
  )

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
            onSelect={session.handleSwitchAgent}
          />
        </div>
        <IconTip label={t("design.chat.newConversation", "新对话")}>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={session.handleNewThread}>
            <Plus className="h-4 w-4" />
          </Button>
        </IconTip>
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
            />
          )}
        </div>
      </div>

      {/* Messages — height-bounded flex column so MessageList scrolls internally. */}
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <MessageList
          messages={session.messages}
          loading={session.loading}
          agents={session.agents}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
          sessionId={session.currentSessionId}
        />
      </div>

      <ApprovalDialog requests={stream.approvalRequests} onRespond={stream.handleApprovalResponse} />

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
          pendingQuotes={stream.pendingQuotes}
          onRemoveQuote={(i) =>
            stream.setPendingQuotes((prev) => prev.filter((_, idx) => idx !== i))
          }
          onJumpToQuote={onJumpToQuote}
          pendingMessage={stream.pendingMessage}
          onCancelPending={() => stream.setPendingMessage(null)}
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
