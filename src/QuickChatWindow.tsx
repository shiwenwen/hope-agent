/**
 * QuickChatWindow — root component for the independent quick-chat Tauri window.
 * Rendered when `?window=quickchat` is in the URL (see main.tsx).
 */
import React, { useEffect, useCallback, useRef } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window"
import { useTranslation } from "react-i18next"
import { initLanguageFromConfig } from "@/i18n/i18n"
import { Plus, ChevronDown, Bot, X } from "lucide-react"
import { cn } from "@/lib/utils"
import { TooltipProvider } from "@/components/ui/tooltip"
import ChatInput from "@/components/chat/ChatInput"
import QuickChatMessages from "@/components/chat/QuickChatMessages"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import { useQuickChatSession } from "@/components/chat/useQuickChatSession"
import { useChatStream } from "@/components/chat/useChatStream"
import type { CommandResult } from "@/components/chat/slash-commands/types"
import type { AgentSummaryForSidebar } from "@/types/chat"

const hideWindow = () => getCurrentWindow().hide()

export default function QuickChatWindow() {
  const session = useQuickChatSession(true)

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
    activeModel: session.activeModel,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
  })

  useEffect(() => { initLanguageFromConfig() }, [])

  // Transparent html/body so CSS border-radius shows rounded corners on macOS
  useEffect(() => {
    document.documentElement.style.background = "transparent"
    document.body.style.background = "transparent"
  }, [])

  const hasMessages = session.messages.length > 0

  // Dynamic window height: compact when empty, expanded when has messages
  useEffect(() => {
    const win = getCurrentWindow()
    if (hasMessages) {
      win.setSize(new LogicalSize(680, 500))
    } else {
      win.setSize(new LogicalSize(680, 180))
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
    win.onFocusChanged(({ payload: focused }) => {
      if (!focused) hideWindow()
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
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
      <div className="flex flex-col h-screen rounded-2xl bg-background/70 dark:bg-background/60 backdrop-blur-2xl backdrop-saturate-150 shadow-2xl">
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

        {/* ── Messages (only when present) ──────── */}
        {hasMessages && (
          <QuickChatMessages
            messages={session.messages}
            loading={session.loading}
            sessionId={session.currentSessionId}
          />
        )}

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
            reasoningEffort={session.reasoningEffort}
            onModelChange={session.handleModelChange}
            onEffortChange={session.handleEffortChange}
            attachedFiles={stream.attachedFiles}
            onAttachFiles={stream.setAttachedFiles}
            onRemoveFile={(i) =>
              stream.setAttachedFiles((prev) => prev.filter((_, idx) => idx !== i))
            }
            pendingMessage={stream.pendingMessage}
            onCancelPending={() => stream.setPendingMessage(null)}
            onStop={stream.handleStop}
            currentSessionId={session.currentSessionId}
            currentAgentId={session.currentAgentId}
            onCommandAction={handleCommandAction}
            toolPermissionMode={stream.toolPermissionMode}
            onToolPermissionChange={stream.setToolPermissionMode}
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
        <span className="font-medium">
          {currentAgent?.name || t("chat.mainAgent")}
        </span>
        <ChevronDown className="h-3 w-3 text-muted-foreground" />
      </button>

      {menuOpen && agents.length > 0 && (
        <div className="absolute top-full left-0 mt-1 min-w-[200px] max-h-[240px] overflow-y-auto bg-popover border border-border rounded-lg shadow-lg py-1 z-10">
          {agents.map((agent) => (
            <button
              key={agent.id}
              onClick={() => { onSelect(agent.id); setMenuOpen(false) }}
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
        </div>
      )}
    </div>
  )
}

// ── Agent Avatar Icon ───────────────────────────

function AgentAvatarIcon({ agent }: { agent?: AgentSummaryForSidebar }) {
  return (
    <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
      {agent?.avatar ? (
        <img
          src={agent.avatar.startsWith("/") ? convertFileSrc(agent.avatar) : agent.avatar}
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
