/**
 * QuickChatWindow — root component for the independent quick-chat Tauri window.
 * Rendered when `?window=quickchat` is in the URL (see main.tsx).
 */
import React, { useEffect, useCallback, useRef, useMemo, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { useTranslation } from "react-i18next"
import { initLanguageFromConfig } from "@/i18n/i18n"
import { Plus, ChevronDown, Bot, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { isLinuxDesktop } from "@/lib/platform"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { TooltipProvider } from "@/components/ui/tooltip"
import ChatInput from "@/components/chat/ChatInput"
import MessageList from "@/components/chat/MessageList"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import IncognitoToggle from "@/components/chat/input/IncognitoToggle"
import { useQuickChatSession } from "@/components/chat/useQuickChatSession"
import { useChatStream } from "@/components/chat/useChatStream"
import type { CommandResult } from "@/components/chat/slash-commands/types"
import type { AgentSummaryForSidebar, PendingMessageQuote } from "@/types/chat"

const hideWindow = () => getCurrentWindow().hide()
const QUICK_CHAT_EMPTY_HEIGHT = 460
const QUICK_CHAT_MESSAGES_HEIGHT = 500

export default function QuickChatWindow() {
  const session = useQuickChatSession(true)
  const quickStreamSeqRef = useRef<Map<string, number>>(new Map())
  const quickEndedStreamIdsRef = useRef<Map<string, string>>(new Map())
  const [composerFocusSignal, setComposerFocusSignal] = useState<number | undefined>(undefined)

  // Effective incognito = persisted session.incognito (continued chat) or
  // draft toggle (new chat). Mirrors `ChatScreen` semantics so the toggle
  // and `useChatStream` see the same value.
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

  // Draft-only incognito toggle handler: ignored once a session exists, just
  // like the main chat (post-create switching is policed by the backend).
  // No useCallback — React Compiler handles memoization; manual useCallback
  // here trips `react-hooks/preserve-manual-memoization` because the inferred
  // dep (whole `session`) is broader than what we close over.
  const handleIncognitoChange = (enabled: boolean) => {
    if (session.currentSessionId) return
    session.setDraftIncognito(enabled)
  }

  useEffect(() => {
    initLanguageFromConfig()
  }, [])

  // Transparent html/body so CSS border-radius shows rounded corners on macOS.
  // Linux 窗口本身不透明（shortcuts.rs 关了 transparent，#547），保持默认背景，
  // 圆角与 clip-path 一并退化为方角。
  useEffect(() => {
    if (isLinuxDesktop()) return
    document.documentElement.style.background = "transparent"
    document.body.style.background = "transparent"
  }, [])

  const hasMessages = session.messages.length > 0

  // Dynamic window height: compact when empty, expanded when has messages
  useEffect(() => {
    const win = getCurrentWindow()
    if (hasMessages) {
      win.setSize(new LogicalSize(680, QUICK_CHAT_MESSAGES_HEIGHT))
    } else {
      win.setSize(new LogicalSize(680, QUICK_CHAT_EMPTY_HEIGHT))
    }
  }, [hasMessages])

  // Escape → hide window
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault()
        hideWindow()
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [])

  // Window blur (click outside) → hide window
  useEffect(() => {
    const win = getCurrentWindow()
    let unlisten: (() => void) | undefined
    win
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) hideWindow()
      })
      .then((fn) => {
        unlisten = fn
      })
    return () => {
      unlisten?.()
    }
  }, [])

  const handleCommandAction = useCallback(
    (result: CommandResult) => {
      const action = result.action
      if (!action) return
      if (action.type === "switchAgent") {
        session.handleSwitchAgent(action.agentId)
      } else if (action.type === "newSession") {
        session.handleNewChat()
      }
    },
    [session],
  )

  const { t } = useTranslation()
  const currentAgent = session.agents.find((a) => a.id === session.currentAgentId)
  const agentMenuRef = useRef<HTMLDivElement>(null)

  return (
    <TooltipProvider>
      <div
        className={cn(
          "flex flex-col h-screen border border-border/60 bg-background/95 shadow-2xl",
          !isLinuxDesktop() && "rounded-2xl [clip-path:inset(0_round_16px)]",
        )}
      >
        {/* ── Title bar (draggable) ─────────────── */}
        <div
          className="flex items-center gap-2 px-4 py-2 shrink-0 select-none"
          data-tauri-drag-region
        >
          <AgentSelector
            agents={session.agents}
            currentAgent={currentAgent}
            onSelect={session.handleSwitchAgent}
            menuRef={agentMenuRef}
          />

          <div className="flex-1" data-tauri-drag-region />

          {session.currentSessionId && hasMessages && (
            <span className="text-[11px] text-muted-foreground/70">
              {t("quickChat.continueSession")}
            </span>
          )}

          {/* Incognito toggle — only meaningful in draft state (no session
           * yet); once the session materializes the backend owns the
           * `incognito` flag and we hide the toggle (mirrors main chat). */}
          {!session.currentSessionId && (
            <IncognitoToggle
              sessionId={null}
              enabled={incognitoEnabled}
              onChange={handleIncognitoChange}
            />
          )}

          <button
            onClick={session.handleNewChat}
            className="h-7 px-2 text-xs gap-1 inline-flex items-center rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/60 transition-colors"
          >
            <Plus className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={hideWindow}
            className="h-7 w-7 inline-flex items-center justify-center rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/60 transition-colors"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* ── Messages ─────────────────────────────
         * MessageList handles its own empty state ("How can I help") and
         * always claims `flex-1 min-h-0` — no parent gating or spacer needed.
         */}
        <MessageList
          messages={session.messages}
          loading={session.loading}
          agents={session.agents}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
          sessionId={session.currentSessionId}
          incognito={incognitoEnabled}
          onAddMessageQuote={handleMessageQuote}
        />

        {/* ── Approval Dialog ────────────────────── */}
        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        {/* ── Input ──────────────────────────────── */}
        <div className="shrink-0">
          <ChatInput
            input={stream.input}
            onInputChange={stream.setInput}
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
            maxAttachmentBytes={stream.maxChatAttachmentBytes}
            onAttachFiles={stream.setAttachedFiles}
            onRemoveFile={(i) =>
              stream.setAttachedFiles((prev) => prev.filter((_, idx) => idx !== i))
            }
            onUpdateFile={(index, file) =>
              stream.setAttachedFiles((prev) =>
                prev.map((existing, idx) =>
                  idx === index
                    ? { ...existing, file, status: "ready", error: undefined }
                    : existing,
                ),
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
            onCommandAction={handleCommandAction}
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionModeByUser}
            sandboxMode={stream.sandboxMode}
            onSandboxModeChange={stream.setSandboxModeByUser}
          />
        </div>
      </div>
    </TooltipProvider>
  )
}

// ── Agent Selector ──────────────────────────────

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
  const [menuOpen, setMenuOpen] = React.useState(false)

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
          "hover:bg-muted/60 transition-colors",
          menuOpen && "bg-muted/60",
        )}
      >
        <AgentAvatarIcon agent={currentAgent} />
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
              <AgentAvatarIcon agent={agent} />
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

function AgentAvatarIcon({ agent }: { agent?: AgentSummaryForSidebar }) {
  return (
    <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
      {agent?.avatar ? (
        <img
          src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
          className="w-full h-full object-cover"
          alt=""
        />
      ) : agent?.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className="h-3 w-3" />
      )}
    </div>
  )
}
