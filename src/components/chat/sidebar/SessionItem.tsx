import { useCallback, useEffect, useRef, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { desktopUnreadCount, channelUnreadCount } from "@/lib/unread"
import { IconTip } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
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
  BellRing,
  FolderInput,
  FolderMinus,
  Check,
  Ghost,
  CircleAlert,
  Download,
  Pin,
  PinOff,
} from "lucide-react"
import { ExportSessionDialog } from "@/components/chat/export/ExportSessionDialog"
import type { SessionMeta, AgentSummaryForSidebar } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"
import ChannelIcon from "@/components/common/ChannelIcon"
import { INCOGNITO_BADGE_ICON_CLASSES } from "@/components/chat/input/incognitoStyles"
import PendingCountdownRing from "./PendingCountdownRing"
import type { SidebarDisplayMode } from "./types"

interface SessionItemProps {
  session: SessionMeta
  sessions: SessionMeta[]
  agent: AgentSummaryForSidebar | undefined
  /** Projects visible in the sidebar — used by the "Move to project" submenu. */
  projects?: ProjectMeta[]
  isActive: boolean
  isReadable: boolean
  isLoading: boolean
  renamingSessionId: string | null
  renameValue: string
  renameInputRef: React.RefObject<HTMLInputElement | null>
  onSwitchSession: (sessionId: string, opts?: { targetMessageId?: number }) => void
  onDeleteClick: (sessionId: string, e: React.MouseEvent) => void
  onStartRename: (sessionId: string, currentTitle: string) => void
  onRenameValueChange: (value: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  onMarkAllRead?: () => void
  onTogglePinned?: (sessionId: string, pinned: boolean) => void
  /**
   * Move this session to a project (or remove from current project when
   * `projectId` is `null`). Only rendered when this callback is provided.
   */
  onMoveToProject?: (sessionId: string, projectId: string | null) => void
  getAgentInfo: (agentId: string) => AgentSummaryForSidebar | undefined
  formatRelativeTime: (dateStr: string) => string
  displayMode: SidebarDisplayMode
  /** Monotonic signal that reveals and briefly highlights this row. */
  revealSignal?: number
}

export default function SessionItem({
  session,
  sessions,
  agent,
  projects = [],
  isActive,
  isReadable,
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
  onTogglePinned,
  onMoveToProject,
  getAgentInfo,
  formatRelativeTime,
  displayMode,
  revealSignal,
}: SessionItemProps) {
  const { t } = useTranslation()
  const [exportOpen, setExportOpen] = useState(false)
  const [revealHighlight, setRevealHighlight] = useState(false)
  const rowRef = useRef<HTMLDivElement>(null)
  // Rename launches from the context menu below. Radix restores focus to the
  // trigger when the menu closes, which would immediately blur the freshly
  // opened rename input → onBlur commits → the box vanishes. This flag lets the
  // rename path suppress that one focus-restore (see ContextMenuContent's
  // onCloseAutoFocus); other menu items keep normal focus behaviour.
  const renameTriggeredRef = useRef(false)
  const isCompact = displayMode === "compact"

  const pendingInteractionCount = session.pendingInteractionCount ?? 0
  const hasPending = !isActive && !session.channelInfo && pendingInteractionCount > 0
  // Countdown ring next to the badge — only when a pending interaction has a
  // timeout configured; timeout-free pendings keep the plain badge.
  const pendingCountdown = hasPending ? (session.pendingCountdown ?? null) : null
  // Both counts share the single-source rules in `@/lib/unread`. Passing the
  // session's own id as the active id when `isActive` makes them read as 0 for
  // the open session (so the badges and the "mark as read" menu agree without a
  // separate `!isActive` gate). They're mutually exclusive — a session is
  // either channel-attached (sky IM badge) or not (red desktop badge).
  const activeId = isReadable ? session.id : null
  const displayUnreadCount = desktopUnreadCount(session, activeId)
  const displayChannelUnreadCount = channelUnreadCount(session, activeId)
  const channelLabel = session.channelInfo
    ? `${session.channelInfo.channelId} · ${session.channelInfo.senderName || session.channelInfo.chatId}`
    : null

  useEffect(() => {
    if (!revealSignal) return
    const frame = window.requestAnimationFrame(() => {
      rowRef.current?.scrollIntoView({
        block: "center",
        behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      })
      setRevealHighlight(true)
    })
    const timer = window.setTimeout(() => setRevealHighlight(false), 1600)
    return () => {
      window.cancelAnimationFrame(frame)
      window.clearTimeout(timer)
    }
  }, [revealSignal])

  const handleMarkAsRead = useCallback(async () => {
    if (displayUnreadCount === 0 && displayChannelUnreadCount === 0) return
    try {
      await getTransport().call("mark_session_read_cmd", {
        sessionId: session.id,
      })
      if (onMarkAllRead) onMarkAllRead()
    } catch (err) {
      logger.error("chat", "ChatSidebar::markSessionRead", "Failed to mark session as read", err)
    }
  }, [session.id, displayUnreadCount, displayChannelUnreadCount, onMarkAllRead])

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={rowRef}
          data-session-id={session.id}
          role="button"
          tabIndex={0}
          className={cn(
            "relative flex items-center gap-2.5 w-full px-2.5 py-2 rounded-lg text-left group cursor-pointer",
            isCompact && "gap-1.5 px-2 py-[7px] rounded-md",
            revealHighlight && "bg-destructive/10",
            isActive
              ? "bg-secondary/70"
              : hasPending
                ? "bg-amber-500/10 hover:bg-amber-500/15 border-l-2 border-l-amber-500 pl-[8px]"
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
          {!isCompact && (
            <div className="relative shrink-0">
              <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary text-[10px] overflow-hidden">
                {isLoading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
                ) : agent?.avatar ? (
                  <img
                    src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                    className="w-full h-full object-cover"
                    alt=""
                  />
                ) : agent?.emoji ? (
                  <span>{agent.emoji}</span>
                ) : (
                  <Bot className="h-3.5 w-3.5" />
                )}
              </div>
              {displayUnreadCount > 0 && (
                <span
                  aria-hidden="true"
                  className="absolute -right-0.5 -top-0.5 z-10 h-2.5 w-2.5 rounded-full border-2 border-background bg-destructive pointer-events-none"
                />
              )}
              {displayChannelUnreadCount > 0 && (
                <span
                  aria-hidden="true"
                  className="absolute -right-0.5 -top-0.5 z-10 h-2.5 w-2.5 rounded-full border-2 border-background bg-sky-500 pointer-events-none"
                />
              )}
              {hasPending && (
                <IconTip label={t("chat.pendingInteractionHint")}>
                  <span
                    className="absolute -bottom-1 -left-1.5 z-10 min-w-[16px] h-[16px] px-0.5 rounded-full text-white text-[9px] font-bold flex items-center justify-center border border-background leading-none animate-pulse cursor-pointer"
                    style={{
                      background: "linear-gradient(135deg, #fbbf24 0%, #f59e0b 50%, #d97706 100%)",
                      boxShadow:
                        "0 2px 6px rgba(217, 119, 6, 0.5), inset 0 1px 1px rgba(255, 255, 255, 0.3)",
                    }}
                  >
                    {pendingInteractionCount > 99 ? "99+" : pendingInteractionCount}
                  </span>
                </IconTip>
              )}
            </div>
          )}

          {/* Title + meta */}
          <div className="flex-1 min-w-0">
            <div
              className={cn(
                "text-[13px] font-medium text-foreground truncate flex items-center gap-1",
                isCompact && "text-[12.5px] leading-5",
              )}
            >
              {isCompact && isLoading && (
                <Loader2 className="h-3 w-3 shrink-0 animate-spin text-primary" />
              )}
              {session.isCron && (
                <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-orange-500/15 text-orange-500">
                  <Timer className="w-2.5 h-2.5" />
                </span>
              )}
              {session.parentSessionId &&
                (() => {
                  const parentSession = sessions.find((s) => s.id === session.parentSessionId)
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
              {!isCompact && session.channelInfo && channelLabel && (
                <IconTip label={channelLabel}>
                  <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-blue-500/15 text-blue-500">
                    <ChannelIcon
                      channelId={session.channelInfo.channelId}
                      className="w-2.5 h-2.5"
                    />
                  </span>
                </IconTip>
              )}
              {session.incognito && (
                <IconTip label={t("chat.incognito")}>
                  <span className={INCOGNITO_BADGE_ICON_CLASSES}>
                    <Ghost className="w-2.5 h-2.5" strokeWidth={1.75} />
                  </span>
                </IconTip>
              )}
              {!isActive && session.hasError && (
                <IconTip label={t("chat.sessionFailed")}>
                  <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-red-500/15 text-red-500">
                    <CircleAlert className="w-2.5 h-2.5" />
                  </span>
                </IconTip>
              )}
              {session.pinnedAt && (
                <IconTip label={t("chat.pinSession")}>
                  <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-primary/10 text-primary">
                    <Pin className="w-2.5 h-2.5" />
                  </span>
                </IconTip>
              )}
              {renamingSessionId === session.id ? (
                <Input
                  ref={renameInputRef}
                  surface="embedded"
                  className="h-auto w-auto min-w-0 flex-1 border-b border-primary px-0 py-0 text-[13px] font-medium"
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
                <span className={cn("truncate", isCompact && "min-w-0 flex-1")}>
                  {session.title || t("chat.newChat")}
                </span>
              )}
              {isCompact && renamingSessionId !== session.id && (
                <span className="ml-auto flex shrink-0 items-center justify-end gap-1 pl-2 group-hover:pr-5">
                  {displayUnreadCount > 0 && (
                    <span
                      aria-hidden="true"
                      className="inline-flex h-2.5 w-2.5 rounded-full bg-destructive"
                    />
                  )}
                  {displayChannelUnreadCount > 0 && (
                    <span
                      aria-hidden="true"
                      className="inline-flex h-2.5 w-2.5 rounded-full bg-sky-500"
                    />
                  )}
                  {pendingCountdown && (
                    <PendingCountdownRing
                      key={`${pendingCountdown.deadlineAtMs}:${pendingCountdown.serverNowMs}`}
                      countdown={pendingCountdown}
                    />
                  )}
                  {hasPending && (
                    <IconTip
                      label={t("chat.pendingInteractionInline", {
                        count: pendingInteractionCount,
                      })}
                    >
                      <span className="inline-flex h-[15px] min-w-[15px] items-center justify-center rounded-full bg-amber-500 px-1 text-[9px] font-bold leading-none text-white tabular-nums animate-pulse">
                        {pendingInteractionCount > 99 ? "99+" : pendingInteractionCount}
                      </span>
                    </IconTip>
                  )}
                  {!isLoading && !hasPending && (
                    <>
                      {session.channelInfo && channelLabel && (
                        <IconTip label={channelLabel}>
                          <span className="inline-flex h-3 w-3 shrink-0 items-center justify-center text-blue-500/70 group-hover:hidden">
                            <ChannelIcon
                              channelId={session.channelInfo.channelId}
                              className="h-2.5 w-2.5"
                            />
                          </span>
                        </IconTip>
                      )}
                      {/* hover 时在原行右侧就地显示 agent 头像 + 名称（替换时间），不弹浮层 */}
                      <span className="hidden min-w-0 items-center gap-1 group-hover:flex">
                        <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center overflow-hidden rounded-full bg-primary/10 text-[8px] text-primary">
                          {agent?.avatar ? (
                            <img
                              src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                              className="h-full w-full object-cover"
                              alt=""
                            />
                          ) : agent?.emoji ? (
                            <span>{agent.emoji}</span>
                          ) : (
                            <Bot className="h-2 w-2" />
                          )}
                        </span>
                        <span className="max-w-[88px] truncate text-[10px] font-normal text-muted-foreground/70">
                          {agent?.name || session.agentId}
                        </span>
                      </span>
                      <span className="text-right text-[10px] font-normal text-muted-foreground/60 group-hover:hidden">
                        {formatRelativeTime(session.updatedAt)}
                      </span>
                    </>
                  )}
                </span>
              )}
              {displayUnreadCount > 0 && <span className="sr-only">{t("chat.unreadStatus")}</span>}
              {displayChannelUnreadCount > 0 && (
                <span className="sr-only">{t("chat.channelUnreadStatus")}</span>
              )}
            </div>
            {!isCompact && (
              <div className="text-[11px] text-muted-foreground truncate">
                {isLoading ? (
                  <>
                    {agent?.name || session.agentId}
                    <span className="mx-1">·</span>
                    <span className="text-primary animate-pulse">
                      {t("chat.thinking")}
                    </span>
                  </>
                ) : hasPending ? (
                  <span className="flex items-center gap-1 text-amber-500 font-medium">
                    {pendingCountdown ? (
                      <PendingCountdownRing
                        key={`${pendingCountdown.deadlineAtMs}:${pendingCountdown.serverNowMs}`}
                        countdown={pendingCountdown}
                      />
                    ) : (
                      <BellRing className="h-3 w-3 shrink-0" />
                    )}
                    <span className="truncate">
                      {t("chat.pendingInteractionInline", {
                        count: pendingInteractionCount,
                      })}
                    </span>
                  </span>
                ) : (
                  <>
                    {agent?.name || session.agentId}
                    <span className="mx-1">·</span>
                    {formatRelativeTime(session.updatedAt)}
                  </>
                )}
              </div>
            )}
          </div>

          {/* Delete button (hover) */}
          <IconTip label={t("common.delete")}>
            <button
              className={cn(
                "shrink-0 transition-colors p-0.5",
                isCompact
                  ? "absolute right-2 top-1/2 hidden -translate-y-1/2 text-muted-foreground/50 hover:!text-destructive group-hover:block"
                  : "text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-destructive",
              )}
              onClick={(e) => onDeleteClick(session.id, e)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </IconTip>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent
        variant="floating"
        onCloseAutoFocus={(e) => {
          if (renameTriggeredRef.current) {
            e.preventDefault()
            renameTriggeredRef.current = false
          }
        }}
      >
        {onTogglePinned && (
          <ContextMenuItem onClick={() => onTogglePinned(session.id, !session.pinnedAt)}>
            {session.pinnedAt ? (
              <PinOff className="h-4 w-4 mr-2" />
            ) : (
              <Pin className="h-4 w-4 mr-2" />
            )}
            {session.pinnedAt ? t("chat.unpinSession") : t("chat.pinSession")}
          </ContextMenuItem>
        )}
        <ContextMenuItem
          onClick={() => {
            renameTriggeredRef.current = true
            onStartRename(session.id, session.title || t("chat.newChat") || "New Chat")
          }}
        >
          <Pencil className="h-4 w-4 mr-2" />
          {t("chat.renameSession")}
        </ContextMenuItem>
        <ContextMenuItem
          onClick={handleMarkAsRead}
          disabled={displayUnreadCount === 0 && displayChannelUnreadCount === 0}
        >
          <CheckCheck className="h-4 w-4 mr-2" />
          {t("chat.markAsRead")}
        </ContextMenuItem>
        <ContextMenuItem onClick={() => setExportOpen(true)}>
          <Download className="h-4 w-4 mr-2" />
          {t("chat.exportSession.menuItem")}
        </ContextMenuItem>
        {/* Project binding — only when a mover is wired AND this session is
            a regular chat (not a sub-agent / cron / channel session, which
            shouldn't be arbitrarily relocated). Channel sessions are filtered
            here because their lifecycle is tied to the IM conversation. */}
        {onMoveToProject && !session.channelInfo && !session.parentSessionId && !session.isCron && (
            <>
              <ContextMenuSeparator />
              <ContextMenuSub>
                <ContextMenuSubTrigger>
                  <FolderInput className="h-4 w-4 mr-2" />
                  {t("project.moveToProject")}
                </ContextMenuSubTrigger>
                <ContextMenuSubContent>
                  {projects.filter((p) => !p.archived).length === 0 ? (
                  <ContextMenuItem disabled>{t("project.noProjects")}</ContextMenuItem>
                  ) : (
                    projects
                      .filter((p) => !p.archived)
                      .map((p) => (
                        <ContextMenuItem
                          key={p.id}
                          disabled={session.projectId === p.id}
                          onClick={() => onMoveToProject(session.id, p.id)}
                        >
                          {session.projectId === p.id ? (
                            <Check className="h-3.5 w-3.5 mr-2 text-primary" />
                          ) : null}
                          <span className="truncate">{p.name}</span>
                        </ContextMenuItem>
                      ))
                  )}
                  {session.projectId && (
                    <>
                      <ContextMenuSeparator />
                    <ContextMenuItem onClick={() => onMoveToProject(session.id, null)}>
                        <FolderMinus className="h-4 w-4 mr-2" />
                        {t("project.removeFromProject")}
                      </ContextMenuItem>
                    </>
                  )}
                </ContextMenuSubContent>
              </ContextMenuSub>
            </>
          )}
        <ContextMenuSeparator />
        <ContextMenuItem
          onClick={(e) => onDeleteClick(session.id, e)}
          className="text-destructive focus:text-destructive"
        >
          <Trash2 className="h-4 w-4 mr-2" />
          {t("common.delete")}
        </ContextMenuItem>
      </ContextMenuContent>
      {exportOpen && (
        <ExportSessionDialog
          open={exportOpen}
          onOpenChange={setExportOpen}
          sessionId={session.id}
          sessionTitle={session.title ?? null}
        />
      )}
    </ContextMenu>
  )
}
