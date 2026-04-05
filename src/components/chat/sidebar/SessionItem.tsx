import { useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { convertFileSrc } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  Bot,
  Trash2,
  Loader2,
  Timer,
  Pencil,
  Network,
  CheckCheck,
} from "lucide-react"
import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import ChannelIcon from "@/components/common/ChannelIcon"

interface SessionItemProps {
  session: SessionMeta
  sessions: SessionMeta[]
  agent: AgentSummaryForSidebar | undefined
  isActive: boolean
  isLoading: boolean
  renamingSessionId: string | null
  renameValue: string
  renameInputRef: React.RefObject<HTMLInputElement | null>
  onSwitchSession: (sessionId: string) => void
  onDeleteClick: (sessionId: string, e: React.MouseEvent) => void
  onStartRename: (sessionId: string, currentTitle: string) => void
  onRenameValueChange: (value: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  onMarkAllRead?: () => void
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
}

export default function SessionItem({
  session,
  sessions,
  agent,
  isActive,
  isLoading,
  renamingSessionId,
  renameValue,
  renameInputRef,
  onSwitchSession,
  onDeleteClick,
  onStartRename,
  onRenameValueChange,
  onCommitRename,
  onCancelRename,
  onMarkAllRead,
  getAgentInfo,
  formatRelativeTime,
}: SessionItemProps) {
  const { t } = useTranslation()

  const handleMarkAsRead = useCallback(async () => {
    if (session.unreadCount === 0) return
    try {
      await getTransport().call("mark_session_read_cmd", {
        sessionId: session.id,
      })
      if (onMarkAllRead) onMarkAllRead()
    } catch (err) {
      logger.error("chat", "ChatSidebar::markSessionRead", "Failed to mark session as read", err)
    }
  }, [session.id, session.unreadCount, onMarkAllRead])

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="button"
          tabIndex={0}
          className={cn(
            "flex items-center gap-2.5 w-full px-2.5 py-2 rounded-lg text-left transition-colors group cursor-pointer",
            isActive
              ? "bg-secondary/70 border border-border/50"
              : "hover:bg-secondary/40",
          )}
          onClick={() => onSwitchSession(session.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault()
              onSwitchSession(session.id)
            }
          }}
        >
          {/* Agent avatar (small) — with loading spinner overlay + unread dot */}
          <div className="relative shrink-0">
            <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary text-[10px] overflow-hidden">
              {isLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
              ) : agent?.avatar ? (
                <img
                  src={
                    agent.avatar.startsWith("/")
                      ? convertFileSrc(agent.avatar)
                      : agent.avatar
                  }
                  className="w-full h-full object-cover"
                  alt=""
                />
              ) : agent?.emoji ? (
                <span>{agent.emoji}</span>
              ) : (
                <Bot className="h-3.5 w-3.5" />
              )}
            </div>
            {!isActive && !session.channelInfo && session.unreadCount > 0 && (
              <span
                className="absolute -top-1 -right-1.5 z-10 min-w-[16px] h-[16px] px-0.5 rounded-full text-white text-[9px] font-bold flex items-center justify-center border border-background pointer-events-none leading-none"
                style={{
                  background:
                    "linear-gradient(135deg, #ff6b6b 0%, #ee3333 50%, #cc1111 100%)",
                  boxShadow:
                    "0 2px 6px rgba(220, 38, 38, 0.45), inset 0 1px 1px rgba(255, 255, 255, 0.25)",
                }}
              >
                {session.unreadCount > 99 ? "99+" : session.unreadCount}
              </span>
            )}
          </div>

          {/* Title + meta */}
          <div className="flex-1 min-w-0">
            <div className="text-[13px] font-medium text-foreground truncate flex items-center gap-1">
              {session.isCron && (
                <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-orange-500/15 text-orange-500">
                  <Timer className="w-2.5 h-2.5" />
                </span>
              )}
              {session.parentSessionId &&
                (() => {
                  const parentSession = sessions.find(
                    (s) => s.id === session.parentSessionId,
                  )
                  const parentAgent = parentSession
                    ? getAgentInfo(parentSession.agentId)
                    : undefined
                  return (
                    <IconTip
                      label={t("chat.subagentFrom", {
                        agent: parentAgent?.name || parentSession?.agentId || "unknown",
                      })}
                    >
                      <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-purple-500/15 text-purple-500">
                        <Network className="w-2.5 h-2.5" />
                      </span>
                    </IconTip>
                  )
                })()}
              {session.channelInfo && (
                <IconTip
                  label={`${session.channelInfo.channelId} · ${session.channelInfo.senderName || session.channelInfo.chatId}`}
                >
                  <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-blue-500/15 text-blue-500">
                    <ChannelIcon channelId={session.channelInfo.channelId} className="w-2.5 h-2.5" />
                  </span>
                </IconTip>
              )}
              {renamingSessionId === session.id ? (
                <input
                  ref={renameInputRef}
                  className="flex-1 min-w-0 bg-transparent border-b border-primary text-[13px] font-medium text-foreground outline-none py-0"
                  value={renameValue}
                  onChange={(e) => onRenameValueChange(e.target.value)}
                  onBlur={onCommitRename}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault()
                      onCommitRename()
                    } else if (e.key === "Escape") {
                      e.preventDefault()
                      onCancelRename()
                    }
                  }}
                  onClick={(e) => e.stopPropagation()}
                  placeholder={t("chat.renameSessionPlaceholder")}
                />
              ) : (
                <span className="truncate">
                  {session.title || t("chat.newChat") || "New Chat"}
                </span>
              )}
            </div>
            <div className="text-[11px] text-muted-foreground truncate">
              {agent?.name || session.agentId}
              <span className="mx-1">·</span>
              {isLoading ? (
                <span className="text-primary animate-pulse">
                  {t("chat.thinking") || "执行中..."}
                </span>
              ) : (
                formatRelativeTime(session.updatedAt)
              )}
            </div>
          </div>

          {/* Delete button (hover) */}
          <IconTip label={t("common.delete")}>
            <button
              className="shrink-0 text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-destructive transition-colors p-0.5"
              onClick={(e) => onDeleteClick(session.id, e)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          onClick={() => onStartRename(session.id, session.title || t("chat.newChat") || "New Chat")}
        >
          <Pencil className="h-4 w-4 mr-2" />
          {t("chat.renameSession")}
        </ContextMenuItem>
        <ContextMenuItem
          onClick={handleMarkAsRead}
          disabled={session.unreadCount === 0}
        >
          <CheckCheck className="h-4 w-4 mr-2" />
          {t("chat.markAsRead")}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
