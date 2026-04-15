import { convertFileSrc } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { renderHighlightedSnippet } from "@/lib/highlight"
import { IconTip } from "@/components/ui/tooltip"
import { Bot, Timer, Network, MessageSquare } from "lucide-react"
import ChannelIcon from "@/components/common/ChannelIcon"
import type {
  AgentSummaryForSidebar,
  SessionMeta,
  SessionSearchResult,
} from "@/types/chat"

interface SearchResultItemProps {
  result: SessionSearchResult
  isActive: boolean
  agent: AgentSummaryForSidebar | undefined
  agents: AgentSummaryForSidebar[]
  sessionMeta: SessionMeta | undefined
  onSwitch: () => void
  formatRelativeTime: (dateStr: string) => string
}

export default function SearchResultItem({
  result,
  isActive,
  agent,
  agents,
  sessionMeta,
  onSwitch,
  formatRelativeTime,
}: SearchResultItemProps) {
  const { t } = useTranslation()

  const title =
    result.sessionTitle?.trim() ||
    sessionMeta?.title?.trim() ||
    t("chat.untitledSession") ||
    "Untitled"

  const typeChip = (() => {
    if (result.channelType) {
      return (
        <IconTip label={result.channelType}>
          <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-blue-500/15 text-blue-500">
            <ChannelIcon channelId={result.channelType} className="w-2.5 h-2.5" />
          </span>
        </IconTip>
      )
    }
    if (result.isCron) {
      return (
        <IconTip label={t("chat.filterCron")}>
          <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-orange-500/15 text-orange-500">
            <Timer className="w-2.5 h-2.5" />
          </span>
        </IconTip>
      )
    }
    if (result.parentSessionId) {
      return (
        <IconTip label={t("chat.filterSubagent")}>
          <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-purple-500/15 text-purple-500">
            <Network className="w-2.5 h-2.5" />
          </span>
        </IconTip>
      )
    }
    return (
      <span className="inline-flex items-center justify-center shrink-0 w-4 h-4 rounded bg-muted text-muted-foreground">
        <MessageSquare className="w-2.5 h-2.5" />
      </span>
    )
  })()

  const agentLabel = agent?.name ?? result.agentId
  // Locate agent avatar even if not in sidebar agents (e.g. subagent)
  const resolvedAgent = agent ?? agents.find((a) => a.id === result.agentId)

  return (
    <div
      role="button"
      tabIndex={0}
      className={cn(
        "flex items-start gap-2.5 w-full px-2.5 py-2 rounded-lg text-left transition-colors group cursor-pointer",
        isActive
          ? "bg-secondary/70 border border-border/50"
          : "hover:bg-secondary/40",
      )}
      onClick={onSwitch}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault()
          onSwitch()
        }
      }}
    >
      {/* Agent avatar */}
      <div className="relative shrink-0 mt-0.5">
        <div className="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center text-primary text-[10px] overflow-hidden">
          {resolvedAgent?.avatar ? (
            <img
              src={
                resolvedAgent.avatar.startsWith("/")
                  ? convertFileSrc(resolvedAgent.avatar)
                  : resolvedAgent.avatar
              }
              className="w-full h-full object-cover"
              alt=""
            />
          ) : resolvedAgent?.emoji ? (
            <span>{resolvedAgent.emoji}</span>
          ) : (
            <Bot className="h-3.5 w-3.5" />
          )}
        </div>
      </div>

      {/* Title + meta + snippet */}
      <div className="flex-1 min-w-0">
        <div className="text-[13px] font-medium text-foreground truncate flex items-center gap-1">
          {typeChip}
          <span className="truncate">{title}</span>
        </div>
        <div className="text-[10px] text-muted-foreground/70 mt-0.5 flex items-center gap-1 truncate">
          <span className="truncate">{agentLabel}</span>
          <span>·</span>
          <span className="shrink-0">{formatRelativeTime(result.timestamp)}</span>
        </div>
        <div
          className="text-[11px] text-muted-foreground mt-1 line-clamp-2 leading-snug break-words"
          // eslint-disable-next-line react/no-danger
          dangerouslySetInnerHTML={{
            __html: renderHighlightedSnippet(result.contentSnippet),
          }}
        />
      </div>
    </div>
  )
}
