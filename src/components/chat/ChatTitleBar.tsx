import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Settings, Copy, BarChart3, Pencil, Zap, Check, X, FileText, Loader2 } from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import { formatMessageTime } from "./chatUtils"
import { logger } from "@/lib/logger"
import type { Message, AvailableModel, ActiveModel, SessionMeta } from "@/types/chat"

interface ChatTitleBarProps {
  agentName: string
  currentAgentId: string
  currentSessionId: string | null
  sessions: SessionMeta[]
  messages: Message[]
  activeModel: ActiveModel | null
  availableModels: AvailableModel[]
  reasoningEffort: string
  loading: boolean
  compacting: boolean
  setCompacting: (v: boolean) => void
  onOpenAgentSettings?: (agentId: string) => void
  onRenameSession?: (sessionId: string, title: string) => void
  onViewSystemPrompt?: () => void
  systemPromptLoading?: boolean
  /**
   * Dispatches a slash command action back to ChatScreen's handler.
   * Used by the "View context" popover button to trigger `/context`
   * without going through the text input.
   */
  onCommandAction?: (action: import("@/components/chat/slash-commands/types").CommandAction) => void
}

export default function ChatTitleBar({
  agentName,
  currentAgentId,
  currentSessionId,
  sessions,
  messages,
  activeModel,
  availableModels,
  reasoningEffort,
  loading,
  compacting,
  setCompacting,
  onRenameSession,
  onOpenAgentSettings,
  onViewSystemPrompt,
  systemPromptLoading,
  onCommandAction,
}: ChatTitleBarProps) {
  const { t } = useTranslation()
  const [showStatus, setShowStatus] = useState(false)
  const statusRef = useRef<HTMLDivElement>(null)

  // Compact result toast
  const [compactToast, setCompactToast] = useState<{ success: boolean; message: string } | null>(null)
  const compactToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Inline title editing
  const [editingTitle, setEditingTitle] = useState(false)
  const [titleValue, setTitleValue] = useState("")
  const titleInputRef = useRef<HTMLInputElement>(null)

  const currentSession = currentSessionId ? sessions.find((s) => s.id === currentSessionId) : null
  const sessionTitle = currentSession?.title || ""

  const startEditTitle = useCallback(() => {
    setTitleValue(sessionTitle || t("chat.newChat") || "")
    setEditingTitle(true)
    setTimeout(() => {
      titleInputRef.current?.focus()
      titleInputRef.current?.select()
    }, 0)
  }, [sessionTitle, t])

  const commitTitle = useCallback(() => {
    if (currentSessionId && titleValue.trim() && onRenameSession) {
      onRenameSession(currentSessionId, titleValue.trim())
    }
    setEditingTitle(false)
  }, [currentSessionId, titleValue, onRenameSession])

  const cancelEditTitle = useCallback(() => {
    setEditingTitle(false)
  }, [])

  // Close status popover on outside click
  useEffect(() => {
    if (!showStatus) return
    const handler = (e: MouseEvent) => {
      if (statusRef.current && !statusRef.current.contains(e.target as Node)) {
        setShowStatus(false)
      }
    }
    document.addEventListener("mousedown", handler)
    return () => document.removeEventListener("mousedown", handler)
  }, [showStatus])

  const currentModel = activeModel
    ? availableModels.find(
        (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId,
      )
    : null

  return (
    <div
      className="h-10 flex items-end justify-between px-4 bg-background shrink-0"
      data-tauri-drag-region
    >
      <div className="flex items-end gap-2 min-w-0 pb-1.5">
        <span className="text-sm font-medium text-foreground shrink-0">
          {agentName || t("chat.mainAgent")}
        </span>
        {currentSessionId && (
          <>
            <span className="text-muted-foreground/40 text-sm shrink-0">/</span>
            {editingTitle ? (
              <div className="flex items-center gap-1 min-w-0">
                <input
                  ref={titleInputRef}
                  className="text-sm text-foreground/80 bg-transparent border-b border-primary outline-none min-w-[80px] max-w-[300px] py-0"
                  value={titleValue}
                  onChange={(e) => setTitleValue(e.target.value)}
                  onBlur={commitTitle}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault()
                      commitTitle()
                    } else if (e.key === "Escape") {
                      e.preventDefault()
                      cancelEditTitle()
                    }
                  }}
                  placeholder={t("chat.renameSessionPlaceholder")}
                />
              </div>
            ) : (
              <button
                className="group flex items-center gap-1 min-w-0 text-sm text-foreground/60 hover:text-foreground transition-colors truncate"
                onClick={startEditTitle}
              >
                <span className="truncate max-w-[300px]">
                  {sessionTitle || t("chat.newChat")}
                </span>
                <Pencil className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-60 transition-opacity" />
              </button>
            )}
            {currentSession?.channelInfo && (
              <span className="inline-flex items-center gap-1 shrink-0 text-[11px] text-blue-500 bg-blue-500/10 px-1.5 py-0.5 rounded">
                <ChannelIcon channelId={currentSession.channelInfo.channelId} />
                {currentSession.channelInfo.channelId}
                {currentSession.channelInfo.senderName && (
                  <span className="text-blue-400">· {currentSession.channelInfo.senderName}</span>
                )}
              </span>
            )}
          </>
        )}
      </div>
      <div className="flex items-end gap-1">
          {/* Compact Context Button */}
          {currentSessionId && (
            <div className="relative">
              <IconTip label={t("chat.compactNow")}>
                <button
                  className={cn(
                    "pb-1.5 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-50",
                    compacting && "text-foreground",
                  )}
                  disabled={compacting || loading}
                  onClick={async () => {
                    setCompacting(true)
                    try {
                      const result = await getTransport().call<{
                        tierApplied: number
                        tokensBefore: number
                        tokensAfter: number
                        messagesAffected: number
                      }>("compact_context_now", {
                        sessionId: currentSessionId,
                      })
                      const saved = result.tokensBefore - result.tokensAfter
                      const msg = result.messagesAffected > 0
                        ? t("chat.compactDone", { saved, affected: result.messagesAffected })
                        : t("chat.compactNoChange")
                      if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                      setCompactToast({ success: true, message: msg })
                      compactToastTimer.current = setTimeout(() => setCompactToast(null), 3000)
                    } catch (e) {
                      logger.error("ui", "ChatTitleBar::compact", "Compact failed", e)
                      if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                      setCompactToast({ success: false, message: t("chat.compactFailed") })
                      compactToastTimer.current = setTimeout(() => setCompactToast(null), 3000)
                    } finally {
                      setCompacting(false)
                    }
                  }}
                >
                  <Zap className={cn("h-4 w-4 pointer-events-none", compacting && "animate-pulse")} />
                </button>
              </IconTip>
              {compactToast && (
                <div className={cn(
                  "absolute top-full right-0 mt-1.5 z-50 whitespace-nowrap rounded-lg border px-2.5 py-1.5 text-xs shadow-lg animate-in fade-in slide-in-from-top-1 duration-200",
                  compactToast.success
                    ? "border-border bg-popover text-popover-foreground"
                    : "border-destructive/30 bg-destructive/10 text-destructive",
                )}>
                  <div className="flex items-center gap-1.5">
                    {compactToast.success ? <Check className="h-3 w-3 text-green-500" /> : <X className="h-3 w-3" />}
                    {compactToast.message}
                  </div>
                </div>
              )}
            </div>
          )}
          {/* Session Status Button */}
          <div className="relative" ref={statusRef}>
            <IconTip label={t("chat.sessionStatus")}>
              <button
                className={cn(
                  "pb-1.5 text-muted-foreground hover:text-foreground transition-colors",
                  showStatus && "text-foreground",
                )}
                onClick={() => setShowStatus((v) => !v)}
              >
                <BarChart3 className="h-4 w-4" />
              </button>
            </IconTip>
            <div
              className={cn(
                "absolute top-full right-0 mt-1.5 z-50 min-w-[260px] rounded-xl border border-border bg-popover p-3.5 shadow-xl transition-all duration-200 origin-top-right",
                showStatus
                  ? "opacity-100 scale-100 pointer-events-auto"
                  : "opacity-0 scale-95 pointer-events-none",
              )}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="space-y-2 text-xs">
                {/* App version */}
                <div className="flex items-center justify-between gap-2">
                  <span className="text-muted-foreground">🖥️ OpenComputer</span>
                  <span className="font-medium text-foreground tabular-nums">v0.1.0</span>
                </div>
                <div className="border-t border-border" />
                {/* Model + Auth */}
                {(() => {
                  const modelLabel = currentModel
                    ? `${currentModel.providerName}/${currentModel.modelId}`
                    : activeModel?.modelId || "—"
                  const apiType = currentModel?.apiType || "—"
                  const authLabel = apiType === "codex" ? "oauth" : "api-key"
                  return (
                    <>
                      <div className="flex items-start gap-2">
                        <span className="text-muted-foreground shrink-0">
                          🧠 {t("chat.statusModel")}
                        </span>
                        <span className="font-medium text-foreground text-right ml-auto">
                          {modelLabel}
                        </span>
                      </div>
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-muted-foreground">🔑 {t("chat.statusAuth")}</span>
                        <span className="font-medium text-foreground">{authLabel}</span>
                      </div>
                    </>
                  )
                })()}
                {/* Context window usage */}
                {(() => {
                  if (!currentModel) return null
                  const ctxK = Math.round(currentModel.contextWindow / 1000)
                  const lastAssistantWithUsage = [...messages]
                    .reverse()
                    .find((msg) => msg.role === "assistant" && msg.usage?.inputTokens)
                  const usedTokens = lastAssistantWithUsage?.usage?.inputTokens || 0
                  const usedK = Math.round(usedTokens / 1000)
                  const pct =
                    currentModel.contextWindow > 0
                      ? Math.round((usedTokens / currentModel.contextWindow) * 100)
                      : 0
                  const barColor =
                    pct < 50 ? "bg-green-500/70" : pct < 80 ? "bg-yellow-500/70" : "bg-red-500/70"
                  return (
                    <div className="space-y-1.5">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-muted-foreground">📚 {t("chat.statusContext")}</span>
                        <span className="font-medium text-foreground tabular-nums">
                          {usedK}/{ctxK}k ({pct}%)
                        </span>
                      </div>
                      <div className="h-1.5 w-full bg-secondary rounded-full overflow-hidden">
                        <div
                          className={`h-full rounded-full transition-all duration-300 ${barColor}`}
                          style={{ width: `${Math.min(pct, 100)}%` }}
                        />
                      </div>
                      {currentSessionId && usedTokens > 0 && (
                        <button
                          className="w-full mt-1 px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors disabled:opacity-50"
                          disabled={compacting || loading}
                          onClick={async () => {
                            setCompacting(true)
                            try {
                              const result = await getTransport().call<{
                                tierApplied: number
                                tokensBefore: number
                                tokensAfter: number
                                messagesAffected: number
                              }>("compact_context_now", {
                                sessionId: currentSessionId,
                              })
                              const saved = result.tokensBefore - result.tokensAfter
                              const msg = result.messagesAffected > 0
                                ? t("chat.compactDone", { saved, affected: result.messagesAffected })
                                : t("chat.compactNoChange")
                              if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                              setCompactToast({ success: true, message: msg })
                              compactToastTimer.current = setTimeout(() => setCompactToast(null), 3000)
                              if (result.messagesAffected > 0) {
                                setShowStatus(false)
                              }
                            } catch (e) {
                              logger.error("ui", "ChatTitleBar::compact", "Compact failed", e)
                              if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                              setCompactToast({ success: false, message: t("chat.compactFailed") })
                              compactToastTimer.current = setTimeout(() => setCompactToast(null), 3000)
                            } finally {
                              setCompacting(false)
                            }
                          }}
                        >
                          {compacting ? t("chat.compacting") : t("chat.compactNow")}
                        </button>
                      )}
                      {/* View context breakdown */}
                      <button
                        className="w-full mt-1 px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors flex items-center justify-center gap-1"
                        onClick={async () => {
                          try {
                            const result = await getTransport().call<{
                              content: string
                              action?: import("@/components/chat/slash-commands/types").CommandAction
                            }>("execute_slash_command", {
                              sessionId: currentSessionId,
                              agentId: currentAgentId,
                              commandText: "/context",
                            })
                            setShowStatus(false)
                            if (result.action) {
                              onCommandAction?.(result.action)
                            }
                          } catch (e) {
                            logger.error("ui", "ChatTitleBar::viewContext", "View context failed", e)
                          }
                        }}
                      >
                        <BarChart3 className="h-3 w-3" />
                        {t("chat.viewContext", "View context")}
                      </button>
                    </div>
                  )
                })()}
                {/* Cache info (Anthropic) */}
                {(() => {
                  const lastAssistantWithUsage = [...messages]
                    .reverse()
                    .find((msg) => msg.role === "assistant" && msg.usage)
                  const u = lastAssistantWithUsage?.usage
                  if (
                    !u ||
                    (u.cacheCreationInputTokens == null && u.cacheReadInputTokens == null)
                  )
                    return null
                  const created = u.cacheCreationInputTokens || 0
                  const read = u.cacheReadInputTokens || 0
                  return (
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">🗄️ {t("chat.statusCache")}</span>
                      <span className="font-medium text-foreground tabular-nums">
                        +{created > 1000 ? `${(created / 1000).toFixed(1)}k` : created}
                        {" / "}⚡{read > 1000 ? `${(read / 1000).toFixed(1)}k` : read}
                      </span>
                    </div>
                  )
                })()}
                <div className="border-t border-border" />
                {/* Agent */}
                <div className="flex items-center justify-between gap-2">
                  <span className="text-muted-foreground">🤖 {t("chat.statusAgent")}</span>
                  <span className="font-medium text-foreground">
                    {agentName || t("chat.mainAgent")}
                  </span>
                </div>
                {/* Session */}
                <div className="flex items-start gap-2">
                  <span className="text-muted-foreground shrink-0">
                    🧵 {t("chat.statusSession")}
                  </span>
                  <span className="font-medium text-foreground text-right ml-auto truncate max-w-[160px]">
                    {currentSessionId
                      ? (() => {
                          const sess = sessions.find((s) => s.id === currentSessionId)
                          return sess?.title || currentSessionId.slice(0, 8)
                        })()
                      : t("chat.statusNewSession")}
                  </span>
                </div>
                {/* Session ID */}
                {currentSessionId && (
                  <div className="flex items-center justify-between gap-2 overflow-hidden">
                    <span className="text-muted-foreground shrink-0">
                      🆔 {t("chat.statusSessionId")}
                    </span>
                    <div
                      className="flex items-center gap-1.5 ml-auto overflow-hidden text-muted-foreground/80 cursor-pointer hover:text-foreground transition-colors group"
                      title={currentSessionId}
                      onClick={() => navigator.clipboard.writeText(currentSessionId)}
                    >
                      <span className="font-mono text-[11px] truncate select-all">
                        {currentSessionId}
                      </span>
                      <Copy className="h-3.5 w-3.5 shrink-0 opacity-70 group-hover:opacity-100 transition-opacity" />
                    </div>
                  </div>
                )}
                {/* Message count */}
                <div className="flex items-center justify-between gap-2">
                  <span className="text-muted-foreground">
                    📊 {t("chat.statusMessages", { count: messages.length })}
                  </span>
                </div>
                <div className="border-t border-border" />
                {/* Runtime: Thinking */}
                <div className="flex items-center justify-between gap-2">
                  <span className="text-muted-foreground">⚙️ {t("chat.statusThinking")}</span>
                  <span className="font-medium text-foreground">
                    {t(`effort.${reasoningEffort}`)}
                  </span>
                </div>
                {/* Updated */}
                {currentSessionId &&
                  (() => {
                    const sess = sessions.find((s) => s.id === currentSessionId)
                    if (!sess) return null
                    return (
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-muted-foreground">
                          🕒 {t("chat.statusUpdated")}
                        </span>
                        <span className="font-medium text-foreground tabular-nums">
                          {formatMessageTime(sess.updatedAt)}
                        </span>
                      </div>
                    )
                  })()}
                {/* View System Prompt */}
                {onViewSystemPrompt && (
                  <>
                    <div className="border-t border-border" />
                    <button
                      className="w-full px-2 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors disabled:opacity-50 flex items-center justify-center gap-1.5"
                      disabled={systemPromptLoading}
                      onClick={() => {
                        onViewSystemPrompt()
                        setShowStatus(false)
                      }}
                    >
                      {systemPromptLoading ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <FileText className="h-3 w-3" />
                      )}
                      {t("chat.viewSystemPrompt")}
                    </button>
                  </>
                )}
              </div>
            </div>
          </div>
          {/* Settings Button */}
          {onOpenAgentSettings && (
            <IconTip label={t("settings.agentSettings")}>
              <button
                className="pb-1.5 text-muted-foreground hover:text-foreground transition-colors"
                onClick={() => onOpenAgentSettings(currentAgentId)}
              >
                <Settings className="h-4 w-4" />
              </button>
            </IconTip>
          )}
      </div>
    </div>
  )
}
