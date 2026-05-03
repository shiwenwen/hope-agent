import { useState, useRef, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { useAppVersion } from "@/lib/appMeta"
import { basename } from "@/lib/path"
import { IconTip } from "@/components/ui/tooltip"
import {
  Settings,
  Copy,
  BarChart3,
  Pencil,
  Zap,
  Check,
  X,
  FileText,
  FolderCheck,
  Loader2,
  Search,
  Ghost,
} from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import { formatCacheUsageDisplay, formatCompactTokenCount } from "./cacheUsageDisplay"
import { formatMessageTime, getContextUsageTokens } from "./chatUtils"
import { INCOGNITO_BADGE_LABEL_CLASSES } from "./input/incognitoStyles"
import { logger } from "@/lib/logger"
import AgentSwitcher from "./AgentSwitcher"
import ProjectIcon from "./project/ProjectIcon"
import type {
  Message,
  AvailableModel,
  ActiveModel,
  SessionMeta,
  AgentSummaryForSidebar,
} from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

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
  onCommandAction?: (result: import("@/components/chat/slash-commands/types").CommandResult) => void
  /** Toggles the in-session "find in page" search bar. */
  onToggleSearch?: () => void
  /** Whether the in-session search bar is currently open (controls active styling). */
  searchOpen?: boolean
  /**
   * Currently effective working directory for this session — session-level
   * value if set, otherwise the parent project's default. `null` when neither
   * is set, in which case the chip is hidden.
   */
  effectiveWorkingDir?: string | null
  /** Source of the effective path; only read when `effectiveWorkingDir` is set. */
  workingDirSource?: "session" | "project"
  /** Project this session belongs to. Surface a chip linking to its settings. */
  project?: ProjectMeta | null
  /** Triggered by the project chip click — opens the settings sheet. */
  onOpenProjectSettings?: (project: ProjectMeta) => void
  /** Available agents for the title-bar agent switcher. */
  agents?: AgentSummaryForSidebar[]
  /**
   * Triggered when the user picks a different agent from the title-bar
   * dropdown. Only invoked while `messages.length === 0` — once messages
   * exist, the dropdown is hidden.
   */
  onChangeAgent?: (agentId: string) => void
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
  onToggleSearch,
  searchOpen,
  effectiveWorkingDir,
  workingDirSource,
  project,
  onOpenProjectSettings,
  agents = [],
  onChangeAgent,
}: ChatTitleBarProps) {
  const { t } = useTranslation()
  const appVersion = useAppVersion()
  const [showStatus, setShowStatus] = useState(false)
  const statusRef = useRef<HTMLDivElement>(null)

  // Compact result toast
  const [compactToast, setCompactToast] = useState<{ success: boolean; message: string } | null>(
    null,
  )
  const compactToastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Session ID copy feedback
  const [sessionIdCopied, setSessionIdCopied] = useState(false)
  const sessionIdCopiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

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

  useEffect(() => {
    return () => {
      if (sessionIdCopiedTimer.current) clearTimeout(sessionIdCopiedTimer.current)
    }
  }, [])

  const handleCopySessionId = useCallback(async () => {
    if (!currentSessionId) return
    try {
      await navigator.clipboard.writeText(currentSessionId)
    } catch (e) {
      logger.error("ui", "ChatTitleBar::copySessionId", "Copy failed", e)
      return
    }
    setSessionIdCopied(true)
    if (sessionIdCopiedTimer.current) clearTimeout(sessionIdCopiedTimer.current)
    sessionIdCopiedTimer.current = setTimeout(() => setSessionIdCopied(false), 1500)
  }, [currentSessionId])

  const currentModel = activeModel
    ? availableModels.find(
        (x) => x.providerId === activeModel.providerId && x.modelId === activeModel.modelId,
      )
    : null
  const workingDirChip = effectiveWorkingDir ? (
    <IconTip
      label={
        workingDirSource === "project"
          ? `${t("chat.workingDir.titleBarInherited")}: ${effectiveWorkingDir}`
          : `${t("chat.workingDir.titleBarSession")}: ${effectiveWorkingDir}`
      }
    >
      <span
        className={cn(
          "inline-flex items-center gap-1 shrink-0 text-[11px] px-1.5 py-0.5 rounded font-mono max-w-[260px]",
          workingDirSource === "project"
            ? "text-muted-foreground bg-muted/60"
            : "text-primary bg-primary/10",
        )}
      >
        <FolderCheck className="h-3 w-3 shrink-0" />
        <span className="truncate">{basename(effectiveWorkingDir)}</span>
      </span>
    </IconTip>
  ) : null

  return (
    <div
      className="h-10 flex items-end justify-between px-4 bg-background shrink-0"
      data-tauri-drag-region
    >
      <div className="flex items-end gap-2 min-w-0 pb-1.5">
        {project && (
          <>
            <div className="inline-flex items-center gap-1 shrink-0 min-w-0">
              <button
                onClick={() => onOpenProjectSettings?.(project)}
                className="inline-flex items-center gap-1 shrink-0 text-[12px] px-1.5 py-0.5 rounded hover:bg-accent/40 transition-colors"
                title={project.description ?? project.name}
              >
                <ProjectIcon project={project} size="xs" />
                <span className="truncate max-w-[140px] text-foreground/80">{project.name}</span>
              </button>
              {workingDirChip}
            </div>
            <span className="text-muted-foreground/40 text-sm shrink-0">/</span>
          </>
        )}
        <AgentSwitcher
          agents={agents}
          currentAgentId={currentAgentId}
          agentName={agentName || t("chat.mainAgent")}
          // Allow switching only before any messages exist — system prompt
          // and history are pinned to the agent once a message is sent.
          disabled={messages.length > 0 || !onChangeAgent}
          onSelect={(agentId) => onChangeAgent?.(agentId)}
        />
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
                <span className="truncate max-w-[300px]">{sessionTitle || t("chat.newChat")}</span>
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
            {currentSession?.incognito && (
              <span className={INCOGNITO_BADGE_LABEL_CLASSES}>
                <Ghost className="h-3 w-3" />
                {t("chat.incognito")}
              </span>
            )}
            {!project && workingDirChip}
          </>
        )}
      </div>
      <div className="flex items-end gap-1">
        {/* In-session Search Button */}
        {currentSessionId && onToggleSearch && (
          <IconTip label={t("chat.sessionSearch")}>
            <button
              className={cn(
                "pb-1.5 text-muted-foreground hover:text-foreground transition-colors",
                searchOpen && "text-foreground",
              )}
              onClick={onToggleSearch}
            >
              <Search className="h-4 w-4" />
            </button>
          </IconTip>
        )}
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
                    const msg =
                      result.messagesAffected > 0
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
              <div
                className={cn(
                  "absolute top-full right-0 mt-1.5 z-50 whitespace-nowrap rounded-lg border px-2.5 py-1.5 text-xs shadow-lg animate-in fade-in slide-in-from-top-1 duration-200",
                  compactToast.success
                    ? "border-border bg-popover text-popover-foreground"
                    : "border-destructive/30 bg-destructive/10 text-destructive",
                )}
              >
                <div className="flex items-center gap-1.5">
                  {compactToast.success ? (
                    <Check className="h-3 w-3 text-green-500" />
                  ) : (
                    <X className="h-3 w-3" />
                  )}
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
                <span className="text-muted-foreground">🖥️ Hope Agent</span>
                <span className="font-medium text-foreground tabular-nums">v{appVersion}</span>
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
              {/* Context window usage. See `getContextUsageTokens` for the
               *  cumulative-vs-last-round rule. */}
              {(() => {
                if (!currentModel) return null
                const ctxK = Math.round(currentModel.contextWindow / 1000)
                const lastAssistantWithUsage = [...messages]
                  .reverse()
                  .find((msg) => msg.role === "assistant" && getContextUsageTokens(msg.usage))
                const usedTokens = getContextUsageTokens(lastAssistantWithUsage?.usage) ?? 0
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
                        {usedK}k/{ctxK}k ({pct}%)
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
                            const msg =
                              result.messagesAffected > 0
                                ? t("chat.compactDone", {
                                    saved,
                                    affected: result.messagesAffected,
                                  })
                                : t("chat.compactNoChange")
                            if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                            setCompactToast({ success: true, message: msg })
                            compactToastTimer.current = setTimeout(
                              () => setCompactToast(null),
                              3000,
                            )
                            if (result.messagesAffected > 0) {
                              setShowStatus(false)
                            }
                          } catch (e) {
                            logger.error("ui", "ChatTitleBar::compact", "Compact failed", e)
                            if (compactToastTimer.current) clearTimeout(compactToastTimer.current)
                            setCompactToast({ success: false, message: t("chat.compactFailed") })
                            compactToastTimer.current = setTimeout(
                              () => setCompactToast(null),
                              3000,
                            )
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
                          onCommandAction?.(result)
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
                if (!u || (u.cacheCreationInputTokens == null && u.cacheReadInputTokens == null))
                  return null
                const created = u.cacheCreationInputTokens || 0
                const read = u.cacheReadInputTokens || 0
                const lastInput = u.lastInputTokens
                return (
                  <div className="space-y-1">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-muted-foreground">
                        🗄️ {t("chat.statusCache")} · {t("chat.statusCumulative")}
                      </span>
                      <span className="font-medium text-foreground tabular-nums">
                        {formatCacheUsageDisplay({
                          created,
                          read,
                          writeLabel: t("chat.statusCacheWrite"),
                          hitLabel: t("chat.statusCacheHit"),
                        })}
                      </span>
                    </div>
                    {lastInput != null && (
                      <div className="flex items-center justify-between gap-2 text-[11px]">
                        <span className="text-muted-foreground">
                          {t("chat.lastRoundInputTokens")}
                        </span>
                        <span className="font-medium text-foreground tabular-nums">
                          {formatCompactTokenCount(lastInput)}
                        </span>
                      </div>
                    )}
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
                <span className="text-muted-foreground shrink-0">🧵 {t("chat.statusSession")}</span>
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
                  <IconTip label={sessionIdCopied ? t("chat.copied") : t("chat.copy")}>
                    <div
                      role="button"
                      tabIndex={0}
                      className={cn(
                        "flex items-center gap-1.5 ml-auto overflow-hidden cursor-pointer transition-colors group",
                        sessionIdCopied
                          ? "text-green-600 dark:text-green-500"
                          : "text-muted-foreground/80 hover:text-foreground",
                      )}
                      onClick={handleCopySessionId}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault()
                          handleCopySessionId()
                        }
                      }}
                    >
                      <span className="font-mono text-[11px] truncate select-all">
                        {currentSessionId}
                      </span>
                      {sessionIdCopied ? (
                        <Check className="h-3.5 w-3.5 shrink-0" />
                      ) : (
                        <Copy className="h-3.5 w-3.5 shrink-0 opacity-70 group-hover:opacity-100 transition-opacity" />
                      )}
                    </div>
                  </IconTip>
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
                      <span className="text-muted-foreground">🕒 {t("chat.statusUpdated")}</span>
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
