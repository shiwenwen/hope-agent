import {
  Bot,
  Cpu,
  Folder,
  FolderKanban,
  GitBranch,
  GitCompare,
  HardDrive,
  Loader2,
  MessageSquare,
  Network,
  Timer,
  Trees,
} from "lucide-react"
import { useTranslation } from "react-i18next"

import ChannelIcon from "@/components/common/ChannelIcon"
import { useSessionGitControl } from "@/components/chat/workspace/useSessionGitControl"
import { useWorkspaceEnvironment } from "@/components/chat/workspace/useWorkspaceEnvironment"
import { TooltipContent } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { basename } from "@/lib/path"
import type { AgentSummaryForSidebar, SessionMeta } from "@/types/chat"
import type { ProjectMeta } from "@/types/project"

interface SessionHoverCardProps {
  session: SessionMeta
  agent?: AgentSummaryForSidebar
  parentSession?: SessionMeta
  parentAgent?: AgentSummaryForSidebar
  project?: ProjectMeta
  formatRelativeTime: (dateStr: string) => string
}

function shortCheckoutPath(root: string, worktree: boolean): string {
  const parts = root
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/)
    .filter(Boolean)
  const visibleParts = worktree ? parts.slice(-2) : parts.slice(-1)
  return visibleParts.join("/") || root
}

function AgentAvatar({ agent }: { agent?: AgentSummaryForSidebar }) {
  return (
    <span className="flex h-5 w-5 shrink-0 items-center justify-center overflow-hidden rounded-full bg-primary/10 text-[10px] text-primary">
      {agent?.avatar ? (
        <img
          src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
          className="h-full w-full object-cover"
          alt=""
        />
      ) : agent?.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className="h-3 w-3" />
      )}
    </span>
  )
}

export default function SessionHoverCard({
  session,
  agent,
  parentSession,
  parentAgent,
  project,
  formatRelativeTime,
}: SessionHoverCardProps) {
  const { t } = useTranslation()
  const environmentState = useWorkspaceEnvironment(session.id)
  const workingDir = environmentState.snapshot?.workingDir.path?.trim() || null
  const workingDirName = workingDir
    ? environmentState.snapshot?.workingDir.name || basename(workingDir)
    : null
  const modelLabel = [session.providerName || session.providerId, session.modelId]
    .filter((value): value is string => Boolean(value))
    .join(" · ")
  const channelLabel = session.channelInfo
    ? `${session.channelInfo.channelId} · ${session.channelInfo.senderName || session.channelInfo.chatId}`
    : null
  const gitAvailable = Boolean(environmentState.snapshot?.git)
  const gitState = useSessionGitControl(gitAvailable ? session.id : null)
  const gitSnapshot = gitState.snapshot
  const gitUnavailable = Boolean(
    environmentState.error ||
    (environmentState.snapshot && workingDir && !environmentState.snapshot.git) ||
    (workingDir && gitState.error),
  )

  return (
    <TooltipContent
      side="right"
      align="start"
      sideOffset={10}
      collisionPadding={12}
      className="w-[min(340px,calc(100vw-24px))] p-3.5 text-sm"
    >
      <div className="mb-3 flex items-start gap-3">
        <p className="line-clamp-2 min-w-0 flex-1 break-words text-[15px] font-semibold leading-5 text-foreground">
          {session.title || t("chat.newChat")}
        </p>
        <span className="shrink-0 text-xs font-normal text-muted-foreground">
          {formatRelativeTime(session.updatedAt)}
        </span>
      </div>

      <div className="space-y-2.5 text-[13px] text-foreground/90">
        {project && (
          <div className="flex min-w-0 items-center gap-2.5">
            <FolderKanban className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{project.name}</span>
          </div>
        )}

        {workingDir && (
          <div className="flex min-w-0 items-start gap-2.5">
            <Folder className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0">
              <p className="truncate">{workingDirName}</p>
              {workingDirName !== workingDir && (
                <p className="truncate text-[11px] text-muted-foreground">{workingDir}</p>
              )}
            </div>
          </div>
        )}

        {environmentState.loading && !environmentState.snapshot && (
          <div className="flex min-w-0 items-center gap-2.5 text-muted-foreground">
            <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
            <span className="truncate">{t("common.loading")}</span>
          </div>
        )}

        {workingDir && gitAvailable && !gitSnapshot && !gitState.error && (
          <div className="flex min-w-0 items-center gap-2.5 text-muted-foreground">
            <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
            <span className="truncate">{t("common.loading")}</span>
          </div>
        )}

        {gitSnapshot && (
          <>
            <div className="flex min-w-0 items-center gap-2.5">
              {gitSnapshot.activeLocation === "worktree" ? (
                <Trees className="h-4 w-4 shrink-0 text-muted-foreground" />
              ) : (
                <HardDrive className="h-4 w-4 shrink-0 text-muted-foreground" />
              )}
              <span className="shrink-0">
                {gitSnapshot.activeLocation === "worktree"
                  ? t("workspace.git.worktree")
                  : t("workspace.git.localCheckout")}
              </span>
              <span className="truncate text-[11px] text-muted-foreground">
                · {shortCheckoutPath(gitSnapshot.root, gitSnapshot.activeLocation === "worktree")}
              </span>
            </div>

            <div className="flex min-w-0 items-center gap-2.5">
              <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate">
                {gitSnapshot.branch || t("workspace.environment.status.detached")}
              </span>
            </div>

            <div className="flex min-w-0 items-center gap-2.5">
              <GitCompare className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className={gitSnapshot.status.clean ? "text-emerald-500" : "text-amber-500"}>
                {gitSnapshot.status.clean
                  ? t("workspace.environment.clean")
                  : t("workspace.environment.changedFiles", {
                      count: gitSnapshot.status.changedFiles,
                    })}
              </span>
              {(gitSnapshot.status.linesAdded > 0 || gitSnapshot.status.linesRemoved > 0) && (
                <span className="truncate text-[11px] tabular-nums">
                  <span className="text-emerald-500">+{gitSnapshot.status.linesAdded}</span>
                  <span className="ml-1 text-red-500">−{gitSnapshot.status.linesRemoved}</span>
                </span>
              )}
              {gitSnapshot.status.conflictedFiles > 0 && (
                <span className="truncate text-[11px] text-red-500">
                  {t("workspace.environment.conflictCount", {
                    count: gitSnapshot.status.conflictedFiles,
                  })}
                </span>
              )}
            </div>
          </>
        )}

        {gitUnavailable && (
          <div className="flex min-w-0 items-center gap-2.5 text-muted-foreground">
            <GitCompare className="h-4 w-4 shrink-0" />
            <span className="truncate">{t("workspace.environment.unavailable")}</span>
          </div>
        )}

        <div className="flex min-w-0 items-center gap-2.5">
          <AgentAvatar agent={agent} />
          <span className="truncate">{agent?.name || session.agentId}</span>
        </div>

        {modelLabel && (
          <div className="flex min-w-0 items-center gap-2.5">
            <Cpu className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{modelLabel}</span>
          </div>
        )}

        {session.parentSessionId && (
          <div className="flex min-w-0 items-center gap-2.5">
            <Network className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">
              {t("chat.subagentFrom", {
                agent: parentAgent?.name || parentSession?.agentId || session.parentSessionId,
              })}
            </span>
          </div>
        )}

        {channelLabel && session.channelInfo && (
          <div className="flex min-w-0 items-center gap-2.5">
            <ChannelIcon
              channelId={session.channelInfo.channelId}
              className="h-4 w-4 shrink-0 text-muted-foreground"
            />
            <span className="truncate">{channelLabel}</span>
          </div>
        )}

        {session.isCron && (
          <div className="flex min-w-0 items-center gap-2.5">
            <Timer className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate">{t("chat.cronTrigger")}</span>
          </div>
        )}

        <div className="flex min-w-0 items-center gap-2.5 text-muted-foreground">
          <MessageSquare className="h-4 w-4 shrink-0" />
          <span>{t("project.overview.messageCount", { count: session.messageCount })}</span>
          {session.incognito && (
            <>
              <span aria-hidden="true">·</span>
              <span>{t("chat.incognito")}</span>
            </>
          )}
        </div>
      </div>
    </TooltipContent>
  )
}
