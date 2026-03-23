import { useState, useRef, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { TooltipProvider, IconTip } from "@/components/ui/tooltip"
import { Settings, Copy, BarChart3 } from "lucide-react"
import { formatMessageTime } from "./chatUtils"
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
  onOpenAgentSettings,
}: ChatTitleBarProps) {
  const { t } = useTranslation()
  const [showStatus, setShowStatus] = useState(false)
  const statusRef = useRef<HTMLDivElement>(null)

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
      <span className="text-sm font-medium text-foreground shrink-0 pb-1.5">
        {agentName || t("chat.mainAgent")}
      </span>
      <div className="flex items-end gap-1">
        <TooltipProvider>
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
                              const result = await invoke<{
                                tierApplied: number
                                tokensBefore: number
                                tokensAfter: number
                                messagesAffected: number
                              }>("compact_context_now", {
                                sessionId: currentSessionId,
                              })
                              if (result.messagesAffected > 0) {
                                setShowStatus(false)
                              }
                            } catch (e) {
                              console.error("compact failed", e)
                            } finally {
                              setCompacting(false)
                            }
                          }}
                        >
                          {compacting ? t("chat.compacting") : t("chat.compactNow")}
                        </button>
                      )}
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
        </TooltipProvider>
      </div>
    </div>
  )
}
