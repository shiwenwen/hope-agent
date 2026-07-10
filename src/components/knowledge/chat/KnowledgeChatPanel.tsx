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
import { toast } from "sonner"
import { Plus, History, Cat, FileArchive } from "lucide-react"

import { Button } from "@/components/ui/button"
import { IconTip, Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import AgentSwitcher from "@/components/chat/AgentSwitcher"
import { useChatStream } from "@/components/chat/hooks/useChatStream"
import { useClickOutside } from "@/hooks/useClickOutside"
import type { ChatAttachment } from "@/lib/transport"
import type { Message, PendingFileQuote } from "@/types/chat"
import type { KbDraftAttachment } from "@/types/knowledge"
import { useKnowledgeChat } from "./useKnowledgeChat"
import { KnowledgeConversationHistory } from "./KnowledgeConversationHistory"
import KnowledgeQueryFilingDialog from "./KnowledgeQueryFilingDialog"
import { useKnowledgeSprite } from "../sprite/useKnowledgeSprite"
import SpriteBubble from "../sprite/SpriteBubble"

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
  /** Increments on every editor change — drives the sprite edit-idle trigger. */
  editorRevision?: number
  /** Whether the panel is actually visible. The component stays mounted (so its
   *  imperative ref is always ready) but defers network loads until shown. */
  active?: boolean
  /** Click a staged quote chip → scroll the editor to that selection. */
  onJumpToQuote?: (q: PendingFileQuote) => void
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
  function KnowledgeChatPanel(
    { kbId, notePath, getEditorValue, editorRevision = 0, active = true, onJumpToQuote },
    ref,
  ) {
    const { t } = useTranslation()
    const isActive = active && !!kbId
    const session = useKnowledgeChat(kbId, notePath, isActive)
    const seqRef = useRef<Map<string, number>>(new Map())
    const endedRef = useRef<Map<string, string>>(new Map())
    const [historyOpen, setHistoryOpen] = useState(false)
    const [filingMessage, setFilingMessage] = useState<Message | null>(null)
    const historyRef = useRef<HTMLDivElement>(null)
    useEffect(() => {
      setFilingMessage(null)
    }, [session.currentSessionId])
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
    // Also bumps `conversationRevision` so the sprite can react to a finished turn.
    const prevLoadingRef = useRef(session.loading)
    const [conversationRevision, setConversationRevision] = useState(0)
    useEffect(() => {
      const was = prevLoadingRef.current
      prevLoadingRef.current = session.loading
      if (was && !session.loading) {
        setConversationRevision((n) => n + 1)
        const sid = session.currentSessionIdRef.current
        if (sid) {
          // Merge DB truth into the current thread (on HTTP this fills in the
          // final answer that wasn't streamed here). Merge-based + guarded so a
          // transient error never blanks the conversation and a late resolve
          // can't clobber a thread the user switched to.
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

    const sprite = useKnowledgeSprite({
      kbId,
      notePath,
      sessionId: session.currentSessionId,
      agentId: session.currentAgentId,
      editorRevision,
      conversationRevision,
      getEditorValue,
      getRecentMessages: () =>
        session.messages.map((m) => ({ role: m.role, text: m.content })),
      active,
    })

    const renderMessageActions = useCallback(
      (msg: Message) => {
        if (msg.role !== "assistant" || msg.dbId == null || !msg.content.trim()) return null
        return (
          <div className="mt-1 flex max-w-full justify-start">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => setFilingMessage(msg)}
            >
              <FileArchive className="h-3.5 w-3.5" />
              {t("knowledge.queryFile.action", "File")}
            </Button>
          </div>
        )
      },
      [t],
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
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                disabled={!sprite.ready}
                className={cn(
                  "relative h-7 w-7 overflow-visible",
                  sprite.enabled &&
                    "text-purple-500 hover:text-purple-500 dark:text-purple-400 dark:hover:text-purple-400",
                  sprite.casting &&
                    "text-fuchsia-500 hover:text-fuchsia-500 dark:text-fuchsia-400 dark:hover:text-fuchsia-400",
                )}
                onClick={() => {
                  const next = !sprite.enabled
                  sprite.setEnabled(next)
                  if (next) {
                    toast.success(t("knowledge.sprite.toastOn"), {
                      description: t("knowledge.sprite.toastOnDesc"),
                    })
                  } else {
                    toast(t("knowledge.sprite.toastOff"))
                  }
                }}
              >
                {/* Enabled: purple cat with slow, diffusing light-wave ripples.
                    Casting (LLM in flight): faster, brighter fuchsia "spell"
                    rings to signal the sprite is actively working. */}
                {sprite.enabled && (
                  <span
                    className="pointer-events-none absolute inset-0 flex items-center justify-center"
                    aria-hidden
                  >
                    {sprite.casting ? (
                      <>
                        <span className="absolute h-5 w-5 rounded-full border-2 border-fuchsia-400/70 animate-ping [animation-duration:1.1s]" />
                        <span className="absolute h-4 w-4 rounded-full border border-violet-400/60 animate-ping [animation-duration:1.1s] [animation-delay:0.45s]" />
                      </>
                    ) : (
                      <>
                        <span className="absolute h-4 w-4 rounded-full border border-purple-400/60 animate-ping [animation-duration:3s]" />
                        <span className="absolute h-4 w-4 rounded-full border border-purple-400/40 animate-ping [animation-duration:3s] [animation-delay:1.5s]" />
                      </>
                    )}
                  </span>
                )}
                <Cat
                  className={cn(
                    "relative h-4 w-4 transition-all",
                    sprite.enabled && !sprite.casting && "drop-shadow-[0_0_3px_#a855f7]",
                    sprite.casting && "animate-pulse drop-shadow-[0_0_6px_#d946ef]",
                  )}
                />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-[240px] leading-relaxed">
              <div className="font-medium">{t("knowledge.sprite.toggle", "Sprite mode")}</div>
              <div className="mt-0.5 text-muted-foreground">
                {t("knowledge.sprite.tooltipDesc")}
              </div>
            </TooltipContent>
          </Tooltip>
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
                  // Opening the popover: reset to the unfiltered first page (the
                  // popover's search box mounts empty).
                  if (!historyOpen) void session.reloadThreads("")
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

        {/* Messages — must be a flex column so MessageList (its root is
            `flex-1 … overflow-hidden`) is height-bounded and scrolls internally
            instead of growing to full content height and overflowing down over
            the sprite bubble + composer. */}
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
          <MessageList
            messages={session.messages}
            loading={session.loading}
            agents={session.agents}
            hasMore={session.hasMore}
            loadingMore={session.loadingMore}
            onLoadMore={session.handleLoadMore}
            sessionId={session.currentSessionId}
            renderMessageActions={renderMessageActions}
          />
        </div>

        <KnowledgeQueryFilingDialog
          kbId={kbId}
          sessionId={session.currentSessionId}
          currentNotePath={notePath}
          message={filingMessage}
          open={filingMessage != null}
          onOpenChange={(open) => {
            if (!open) setFilingMessage(null)
          }}
          onAfterApply={() => {
            const sid = session.currentSessionIdRef.current
            if (sid) void session.reconcileThread(sid)
            void session.reloadThreads()
          }}
        />

        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        {/* Sprite bubble — transient, above the composer, never in history. */}
        {sprite.suggestion && (
          <SpriteBubble
            suggestion={sprite.suggestion}
            agent={currentAgent}
            onDismiss={sprite.dismiss}
            onRespond={(text) => {
              stream.setInput(
                stream.input ? `${stream.input}\n\n> ${text}\n\n` : `> ${text}\n\n`,
              )
              sprite.dismiss()
            }}
          />
        )}

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
            onStop={stream.handleStop}
            currentSessionId={session.currentSessionId}
            currentAgentId={session.currentAgentId}
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionModeByUser}
            sandboxMode={stream.sandboxMode}
            onSandboxModeChange={stream.setSandboxModeByUser}
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
