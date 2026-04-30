import React, { useState, useEffect, useCallback, useRef } from "react"
import { createPortal } from "react-dom"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { X, Plus, ChevronDown, Bot } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import ChatInput from "@/components/chat/ChatInput"
import QuickChatMessages from "@/components/chat/QuickChatMessages"
import ApprovalDialog from "@/components/chat/ApprovalDialog"
import { useQuickChatSession } from "./useQuickChatSession"
import { useChatStream } from "./useChatStream"
import type { CommandResult } from "./slash-commands/types"
import type { AgentSummaryForSidebar } from "@/types/chat"

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
    activeModel: session.activeModel,
    reloadSessions: session.reloadSessions,
    updateSessionMessages: session.updateSessionMessages,
    lastSeqRef: quickStreamSeqRef,
    endedStreamIdsRef: quickEndedStreamIdsRef,
  })

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
          getTransport().call("delete_session_cmd", { sessionId: action.sessionId }).catch(() => {})
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
            <span className="text-xs text-muted-foreground">
              {t("quickChat.continueSession")}
            </span>
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
        <QuickChatMessages
          messages={session.messages}
          loading={session.loading}
          sessionId={session.currentSessionId}
          onNavigateToSession={handleNavigate}
          hasMore={session.hasMore}
          loadingMore={session.loadingMore}
          onLoadMore={session.handleLoadMore}
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
            permissionMode={stream.permissionMode}
            onPermissionModeChange={stream.setPermissionMode}
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
        </div>
      )}
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
    <div className={cn(dim, "rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden")}>
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
