/**
 * QuickChatWindow — root component for the independent quick-chat Tauri window.
 * Rendered when `?window=quickchat` is in the URL (see main.tsx).
 */
import { useEffect, useCallback, useRef } from "react"
import { convertFileSrc } from "@tauri-apps/api/core"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { useTranslation } from "react-i18next"
import { initLanguageFromConfig } from "@/i18n/i18n"
import { Plus, ChevronDown, Bot, Minus, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { TooltipProvider } from "@/components/ui/tooltip"
import ChatInput from "@/components/chat/ChatInput"
import QuickChatMessages from "@/components/chat/QuickChatMessages"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import { useQuickChatSession } from "@/components/chat/useQuickChatSession"
import { useChatStream } from "@/components/chat/useChatStream"
import type { CommandResult } from "@/components/chat/slash-commands/types"
import type { AgentSummaryForSidebar } from "@/types/chat"

export default function QuickChatWindow() {
  // Always active — this is a standalone window
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

  // Init language
  useEffect(() => { initLanguageFromConfig() }, [])

  // Escape → hide window (not close — keep alive for next shortcut toggle)
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault()
        getCurrentWindow().hide()
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
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
      <div className="flex flex-col h-screen bg-background rounded-2xl overflow-hidden">
        {/* ── Title bar (draggable) ─────────────── */}
        <div
          data-tauri-drag-region
          className="flex items-center gap-2 px-4 py-2.5 border-b border-border shrink-0 select-none"
        >
          {/* Agent selector */}
          <AgentSelector
            agents={session.agents}
            currentAgent={currentAgent}
            onSelect={session.handleSwitchAgent}
            menuRef={agentMenuRef}
          />

          <div className="flex-1" data-tauri-drag-region />

          {/* Continuing session hint */}
          {session.currentSessionId && session.messages.length > 0 && (
            <span className="text-xs text-muted-foreground">
              {t("quickChat.continueSession")}
            </span>
          )}

          {/* New chat */}
          <Button
            variant="ghost"
            size="sm"
            onClick={session.handleNewChat}
            className="h-7 px-2 text-xs gap-1"
          >
            <Plus className="h-3.5 w-3.5" />
            {t("quickChat.newChat")}
          </Button>

          {/* Minimize */}
          <Button
            variant="ghost"
            size="icon"
            onClick={() => getCurrentWindow().hide()}
            className="h-7 w-7"
          >
            <Minus className="h-3.5 w-3.5" />
          </Button>

          {/* Close */}
          <Button
            variant="ghost"
            size="icon"
            onClick={() => getCurrentWindow().hide()}
            className="h-7 w-7"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* ── Messages ──────────────────────────── */}
        <QuickChatMessages
          messages={session.messages}
          loading={session.loading}
          sessionId={session.currentSessionId}
        />

        {/* ── Approval Dialog ────────────────────── */}
        <ApprovalDialog
          requests={stream.approvalRequests}
          onRespond={stream.handleApprovalResponse}
        />

        {/* ── Input ──────────────────────────────── */}
        <div className="border-t border-border px-3 py-2 shrink-0">
          <ChatInput
            input={stream.input}
            onInputChange={stream.setInput}
            onSend={stream.handleSend}
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
          "hover:bg-muted transition-colors",
          menuOpen && "bg-muted",
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
