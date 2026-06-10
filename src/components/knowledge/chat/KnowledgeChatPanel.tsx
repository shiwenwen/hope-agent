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
import type { KbDraftAttachment } from "@/types/knowledge"
import { useKnowledgeChat } from "./useKnowledgeChat"
import { KnowledgeConversationHistory } from "./KnowledgeConversationHistory"

/** Per-turn cap on the auto-injected current-note context (chars). Longer notes
 *  are truncated; the assistant uses `note_read` for the full text. */
const CURRENT_NOTE_CONTEXT_MAX = 4000

export interface KnowledgeChatPanelHandle {
  /** Stage a selection as a removable quote chip in the composer. */
  addQuote: (quote: PendingFileQuote) => void
  /** Append a `[[note]]` reference (or any token) to the composer input. */
  insertToken: (token: string) => void
}

interface Props {
  kbId: string | null
  /** Currently-open note's rel path (the conversation anchor + per-turn context). */
  notePath: string | null
  /** Reads the editor's current text for the per-turn current-note context. */
  getEditorValue: () => string
  /** Whether the panel is actually visible. The component stays mounted (so its
   *  imperative ref is always ready) but defers network loads until shown. */
  active?: boolean
}

/**
 * Embedded AI chat for the knowledge space, shown in the right panel as an
 * alternative to the backlinks view. Reuses the main chat's streaming engine
 * (`useChatStream`) + render/input components, but the session is a knowledge
 * thread (`useKnowledgeChat`): anchored to the open note, bound to the KB
 * (write) for cross-note retrieval, and injected with a trimmed tool set
 * (`toolScope: "knowledge"`).
 */
export const KnowledgeChatPanel = forwardRef<KnowledgeChatPanelHandle, Props>(
  function KnowledgeChatPanel({ kbId, notePath, getEditorValue, active = true }, ref) {
    const { t } = useTranslation()
    const isActive = active && !!kbId
    const session = useKnowledgeChat(kbId, notePath, isActive)
    const seqRef = useRef<Map<string, number>>(new Map())
    const endedRef = useRef<Map<string, string>>(new Map())
    const [historyOpen, setHistoryOpen] = useState(false)
    const historyRef = useRef<HTMLDivElement>(null)
    useClickOutside(
      historyRef,
      useCallback(() => setHistoryOpen(false), []),
    )

    // Draft KB attaches for the composer (no live session yet). The panel's own
    // KB stays attached (write) so its notes are reachable for `[[ ]]`/`@`; the
    // KnowledgePicker lets the user attach *other* spaces for joint Q&A. Once a
    // session exists the picker switches to live attach (sessionId) and this is
    // ignored. The bound KB can't be detached here — it's the panel's anchor.
    const [draftKbAttachments, setDraftKbAttachments] = useState<KbDraftAttachment[]>([])
    useEffect(() => {
      setDraftKbAttachments(kbId ? [{ kbId, access: "write" }] : [])
    }, [kbId])
    const handleDraftKbChange = useCallback(
      (next: KbDraftAttachment[]) => {
        const others = next.filter((a) => a.kbId !== kbId)
        setDraftKbAttachments(kbId ? [{ kbId, access: "write" }, ...others] : others)
      },
      [kbId],
    )

    // Stable readers for the per-turn current-note context so the injected
    // attachment always reflects the editor's live text + open note.
    const notePathRef = useRef(notePath)
    notePathRef.current = notePath
    const getEditorValueRef = useRef(getEditorValue)
    getEditorValueRef.current = getEditorValue

    const getExtraAttachments = useCallback((): ChatAttachment[] => {
      const path = notePathRef.current
      if (!path) return []
      const content = getEditorValueRef.current() ?? ""
      if (!content.trim()) return []
      const truncated = content.length > CURRENT_NOTE_CONTEXT_MAX
      const body = truncated
        ? `${content.slice(0, CURRENT_NOTE_CONTEXT_MAX)}\n…(truncated — use note_read for the full note)`
        : content
      return [
        {
          name: `current note: ${path}`,
          mime_type: "text/plain",
          source: "quote",
          data: body,
          file_path: path,
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
      draftKbAttachments,
      draftKbAnchorNote: notePath,
      toolScope: "knowledge",
      getExtraAttachments,
    })

    // Reconcile against DB truth when a turn finishes. On Tauri the per-call
    // channel already streamed the assistant live; on HTTP (no reattach wired
    // here) this is what fills in the final answer. Cheap for short threads.
    const prevLoadingRef = useRef(session.loading)
    useEffect(() => {
      const was = prevLoadingRef.current
      prevLoadingRef.current = session.loading
      if (was && !session.loading) {
        const sid = session.currentSessionIdRef.current
        if (sid) {
          void session.switchThread(sid)
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

    if (!kbId) {
      return (
        <div className="flex h-full items-center justify-center p-4 text-center text-xs text-muted-foreground">
          {t("knowledge.chatPanel.noKb")}
        </div>
      )
    }

    const currentAgent = session.agents.find((a) => a.id === session.currentAgentId)

    return (
      <div className="flex h-full min-h-0 min-w-0 flex-col">
        {/* Header: agent + new + history. No divider — blends with the surface
            like the main chat title bar (which is borderless bg-background). */}
        <div className="flex min-w-0 items-center gap-1 px-2 py-1.5">
          <div className="min-w-0 flex-1">
            <AgentSwitcher
              agents={session.agents}
              currentAgentId={session.currentAgentId}
              agentName={currentAgent?.name || t("chat.mainAgent")}
              onSelect={session.handleSwitchAgent}
            />
          </div>
          <IconTip label={t("knowledge.chatPanel.newConversation")}>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={session.handleNewThread}
            >
              <Plus className="h-4 w-4" />
            </Button>
          </IconTip>
          <div className="relative" ref={historyRef}>
            <IconTip label={t("knowledge.chatPanel.history")}>
              <Button
                variant="ghost"
                size="icon"
                className={cn("h-7 w-7", historyOpen && "bg-secondary")}
                onClick={() => {
                  if (!historyOpen) void session.reloadThreads()
                  setHistoryOpen((v) => !v)
                }}
              >
                <History className="h-4 w-4" />
              </Button>
            </IconTip>
            {historyOpen && (
              <KnowledgeConversationHistory
                threads={session.threads}
                activeSessionId={session.currentSessionId}
                onSearch={(q) => session.reloadThreads(q)}
                onPick={(sid) => {
                  setHistoryOpen(false)
                  void session.switchThread(sid)
                }}
              />
            )}
          </div>
        </div>

        {/* Messages */}
        <div className="min-h-0 flex-1">
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

        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        {/* Composer — no top divider; ChatInput supplies its own padding, so it
            sits directly on the surface like the main chat composer. */}
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
            pendingMessage={stream.pendingMessage}
            onCancelPending={() => stream.setPendingMessage(null)}
            onStop={stream.handleStop}
            currentSessionId={session.currentSessionId}
            currentAgentId={session.currentAgentId}
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionMode}
            enableNoteMention
            draftKbAttachments={draftKbAttachments}
            onDraftKbAttachChange={handleDraftKbChange}
          />
        </div>
      </div>
    )
  },
)

export default KnowledgeChatPanel
