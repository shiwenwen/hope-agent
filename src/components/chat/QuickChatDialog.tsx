import React, { useState, useEffect, useCallback, useMemo, useRef } from "react"
import { createPortal } from "react-dom"
import { toast } from "sonner"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { X, Plus, ChevronDown, Bot, ExternalLink } from "lucide-react"
import { Button } from "@/components/ui/button"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import IncognitoToggle from "@/components/chat/input/IncognitoToggle"
import { useQuickChatSession } from "./useQuickChatSession"
import { useChatStream } from "./useChatStream"
import type { CommandResult } from "./slash-commands/types"
import type { AgentSummaryForSidebar, PendingMessageQuote } from "@/types/chat"
import type { QuickPromptAddResult, QuickPromptConfig, QuickPromptItem } from "@/types/quickPrompts"
import { recentUserInputHistory } from "./quick-prompts/messageQuickPrompts"

interface QuickChatDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onNavigateToSession?: (sessionId: string) => void
}

export default function QuickChatDialog({
  open,
  onOpenChange,
  onNavigateToSession,
}: QuickChatDialogProps) {
  const { t } = useTranslation()
  const session = useQuickChatSession(open)
  const agentMenuRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  // Quick chat is transient — no reattach logic is wired, but the seq cursor
  // still has to be supplied to `useChatStream` and local-only is fine here.
  const quickStreamSeqRef = useRef<Map<string, number>>(new Map())
  const quickEndedStreamIdsRef = useRef<Map<string, string>>(new Map())
  const [quickPrompts, setQuickPrompts] = useState<QuickPromptItem[]>([])
  const [composerFocusSignal, setComposerFocusSignal] = useState<number | undefined>(undefined)

  // Effective incognito = persisted session.incognito (continued chat) or
  // draft toggle (new chat). Same shape as `ChatScreen` so `useChatStream`
  // and `IncognitoToggle` see the same value.
  const currentSessionMeta = useMemo(
    () =>
      session.currentSessionId
        ? (session.sessions.find((s) => s.id === session.currentSessionId) ?? null)
        : null,
    [session.sessions, session.currentSessionId],
  )
  const incognitoEnabled = session.currentSessionId
    ? (currentSessionMeta?.incognito ?? false)
    : session.draftIncognito
  const inputHistory = useMemo(() => recentUserInputHistory(session.messages), [session.messages])

  useEffect(() => {
    if (!open) return
    let cancelled = false
    getTransport()
      .call<QuickPromptConfig>("get_quick_prompt_config")
      .then((config) => {
        if (!cancelled) setQuickPrompts(config.items ?? [])
      })
      .catch(() => {
        if (!cancelled) setQuickPrompts([])
      })
    return () => {
      cancelled = true
    }
  }, [open])

  const handleAddQuickPrompt = useCallback(
    async (content: string) => {
      if (incognitoEnabled) return
      try {
        const result = await getTransport().call<QuickPromptAddResult>("add_quick_prompt", {
          content,
        })
        setQuickPrompts((prev) => {
          if (result.duplicate) {
            return prev.some((item) => item.id === result.item.id) ? prev : [result.item, ...prev]
          }
          return [result.item, ...prev.filter((item) => item.id !== result.item.id)]
        })
        toast.success(
          result.duplicate ? t("chat.quickPrompts.duplicate") : t("chat.quickPrompts.added"),
        )
      } catch {
        toast.error(t("chat.quickPrompts.addFailed"))
      }
    },
    [incognitoEnabled, t],
  )

  // ── Stream Hook ─────────────────────────────────
  const stream = useChatStream({
    messages: session.messages,
    setMessages: session.setMessages,
    currentSessionId: session.currentSessionId,
    setCurrentSessionId: session.setCurrentSessionId,
    currentSessionIdRef: session.currentSessionIdRef,
    currentAgentId: session.currentAgentId,
    agentName: session.agentName,
    loading: session.loading,
    setLoading: session.setLoading,
    loadingSessionsRef: session.loadingSessionsRef,
    setLoadingSessionIds: session.setLoadingSessionIds,
    sessionCacheRef: session.sessionCacheRef,
    sessions: session.sessions,
    agents: session.agents,
    manualModelOverrideRef: session.manualModelOverrideRef,
    reasoningEffort: session.reasoningEffort,
    temperatureOverride: session.sessionTemperature,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: quickStreamSeqRef,
    endedStreamIdsRef: quickEndedStreamIdsRef,
    incognitoEnabled,
  })
  const handleMessageQuote = useCallback(
    (quote: PendingMessageQuote) => {
      stream.setPendingMessageQuotes((prev) => [...prev, quote])
      setComposerFocusSignal((prev) => (prev ?? 0) + 1)
    },
    [stream],
  )

  // Draft-only incognito toggle handler. No useCallback — see QuickChatWindow
  // for the React Compiler dep-inference rationale.
  const handleIncognitoChange = (enabled: boolean) => {
    if (session.currentSessionId) return
    session.setDraftIncognito(enabled)
  }

  // ── Keyboard handling ───────────────────────────
  useEffect(() => {
    if (!open) return
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault()
        onOpenChange(false)
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [open, onOpenChange])

  // ── Slash command action handler ────────────────
  const handleCommandAction = useCallback(
    (result: CommandResult) => {
      const action = result.action
      if (!action) return
      if (action.type === "switchAgent") {
        session.handleSwitchAgent(action.agentId)
      } else if (action.type === "newSession") {
        session.handleNewChat()
        if (action.sessionId) {
          getTransport()
            .call("delete_session_cmd", { sessionId: action.sessionId })
            .catch(() => {})
        }
      }
    },
    [session],
  )

  // ── Navigate to full conversation ───────────────
  const handleNavigate = useCallback(
    (sessionId: string) => {
      onOpenChange(false)
      onNavigateToSession?.(sessionId)
    },
    [onOpenChange, onNavigateToSession],
  )

  if (!open) return null

  const currentAgent = session.agents.find((a) => a.id === session.currentAgentId)

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]"
      onClick={(e) => {
        if (e.target === e.currentTarget) onOpenChange(false)
      }}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/40 backdrop-blur-sm animate-in fade-in duration-150" />

      {/* Dialog */}
      <div
        ref={containerRef}
        className={cn(
          "relative w-[640px] max-w-[90vw] max-h-[70vh] flex flex-col",
          "bg-background border border-border rounded-2xl shadow-2xl",
          "animate-in fade-in slide-in-from-top-4 duration-200",
        )}
      >
        {/* ── Header ─────────────────────────────── */}
        <div className="flex items-center gap-2 px-4 py-3 border-b border-border">
          {/* Agent selector */}
          <AgentSelector
            agents={session.agents}
            currentAgent={currentAgent}
            onSelect={session.handleSwitchAgent}
            menuRef={agentMenuRef}
          />

          <div className="flex-1" />

          {/* Continuing session hint */}
          {session.currentSessionId && session.messages.length > 0 && (
            <span className="text-xs text-muted-foreground">{t("quickChat.continueSession")}</span>
          )}

          {/* Incognito toggle — draft state only (mirrors main chat). */}
          {!session.currentSessionId && (
            <IncognitoToggle
              sessionId={null}
              enabled={incognitoEnabled}
              onChange={handleIncognitoChange}
            />
          )}

          {/* New chat button */}
          <Button
            variant="ghost"
            size="sm"
            onClick={session.handleNewChat}
            className="h-7 px-2 text-xs gap-1"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("quickChat.newChat")}
          </Button>

          {/* Open in main window — only when there's a session with messages */}
          {session.currentSessionId && session.messages.length > 0 && onNavigateToSession && (
            <IconTip label={t("quickChat.viewFullChat")}>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => handleNavigate(session.currentSessionId!)}
                className="h-7 w-7"
                aria-label={t("quickChat.viewFullChat")}
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          )}

          {/* Close button */}
          <Button
            variant="ghost"
            size="icon"
            onClick={() => onOpenChange(false)}
            className="h-7 w-7"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* ── Messages ──────────────────────────── */}
        <MessageList
          messages={session.messages}
          loading={session.loading}
          agents={session.agents}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
          sessionId={session.currentSessionId}
          incognito={incognitoEnabled}
          onAddQuickPrompt={incognitoEnabled ? undefined : handleAddQuickPrompt}
          onAddMessageQuote={handleMessageQuote}
        />

        {/* ── Approval Dialog ────────────────────── */}
        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        {/* ── Input Area ──────────────────────────── */}
        <div className="border-t border-border px-3 py-2">
          <ChatInput
            input={stream.input}
            onInputChange={stream.setInput}
            inputHistory={inputHistory}
            quickPrompts={quickPrompts}
            onSend={() => stream.handleSend()}
            loading={session.loading}
            availableModels={session.availableModels}
            activeModel={session.activeModel}
            unavailableModelPreference={session.unavailableModelPreference}
            reasoningEffort={session.reasoningEffort}
            onModelChange={session.handleModelChange}
            onEffortChange={session.handleEffortChange}
            onEffortReset={session.resetEffort}
            sessionTemperature={session.sessionTemperature}
            onSessionTemperatureChange={session.handleTemperatureChange}
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
            pendingMessageQuotes={stream.pendingMessageQuotes}
            onRemoveMessageQuote={(i) =>
              stream.setPendingMessageQuotes((prev) => prev.filter((_, idx) => idx !== i))
            }
            focusSignal={composerFocusSignal}
            pendingMessage={stream.pendingMessage}
            pendingSends={stream.pendingSends}
            onCancelPending={() => stream.setPendingMessage(null)}
            onDiscardPending={() => stream.setPendingMessage(null)}
            onEditPending={stream.editPendingSend}
            onDiscardPendingItem={stream.discardPendingSend}
            onSendPending={stream.sendPendingSend}
            onForceInsertPending={stream.forceInsertPendingSend}
            onCancelForceInsertPending={stream.cancelForceInsertPendingSend}
            onStop={stream.handleStop}
            currentSessionId={session.currentSessionId}
            currentAgentId={session.currentAgentId}
            enableAgentMention
            agents={session.agents}
            onCommandAction={handleCommandAction}
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionModeByUser}
            sandboxMode={stream.sandboxMode}
            onSandboxModeChange={stream.setSandboxModeByUser}
          />
        </div>
      </div>
    </div>,
    document.body,
  )
}

// ── Agent Selector Sub-component ─────────────────

function AgentSelector({
  agents,
  currentAgent,
  onSelect,
  menuRef,
}: {
  agents: AgentSummaryForSidebar[]
  currentAgent?: AgentSummaryForSidebar
  onSelect: (agentId: string) => void
  menuRef: React.RefObject<HTMLDivElement | null>
}) {
  const { t } = useTranslation()
  const [menuOpen, setMenuOpen] = useState(false)

  // Close menu on outside click
  useEffect(() => {
    if (!menuOpen) return
    function onClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false)
      }
    }
    document.addEventListener("mousedown", onClick)
    return () => document.removeEventListener("mousedown", onClick)
  }, [menuOpen, menuRef])

  return (
    <div className="relative" ref={menuRef}>
      <button
        onClick={() => setMenuOpen(!menuOpen)}
        className={cn(
          "flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-sm",
          "hover:bg-muted transition-colors",
          menuOpen && "bg-muted",
        )}
      >
        <AgentAvatarIcon agent={currentAgent} size="sm" />
        <span className="font-medium">{currentAgent?.name || t("chat.mainAgent")}</span>
        <ChevronDown className="h-3 w-3 text-muted-foreground" />
      </button>

      <FloatingMenu
        open={menuOpen && agents.length > 0}
        positionClassName="top-full left-0 mt-1.5"
        originClassName="origin-top-left"
        className="ha-menu-from-top min-w-[200px] max-h-[240px] overflow-y-auto p-1.5"
        onEscapeKeyDown={() => setMenuOpen(false)}
      >
          {agents.map((agent) => (
            <button
              key={agent.id}
              onClick={() => {
                onSelect(agent.id)
                setMenuOpen(false)
              }}
              className={cn(
                "w-full text-left px-3 py-1.5 text-sm hover:bg-muted transition-colors flex items-center gap-2",
                agent.id === currentAgent?.id && "bg-muted/50",
              )}
            >
              <AgentAvatarIcon agent={agent} size="sm" />
              <span className="truncate">{agent.name}</span>
              {agent.id === currentAgent?.id && (
                <span className="ml-auto text-xs text-primary">●</span>
              )}
            </button>
          ))}
      </FloatingMenu>
    </div>
  )
}

// ── Agent Avatar Icon ───────────────────────────

function AgentAvatarIcon({
  agent,
  size = "sm",
}: {
  agent?: AgentSummaryForSidebar
  size?: "sm" | "md"
}) {
  const dim = size === "md" ? "w-6 h-6" : "w-5 h-5"
  const iconDim = size === "md" ? "h-3.5 w-3.5" : "h-3 w-3"
  return (
    <div
      className={cn(
        dim,
        "rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden",
      )}
    >
      {agent?.avatar ? (
        <img
          src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
          className="w-full h-full object-cover"
          alt=""
        />
      ) : agent?.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className={iconDim} />
      )}
    </div>
  )
}
