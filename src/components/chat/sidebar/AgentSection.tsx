import { useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  ChevronRight,
  Bot,
  Ghost,
  MessageSquarePlus,
  Settings,
} from "lucide-react"
import type { AgentSummaryForSidebar } from "@/types/chat"

interface AgentSectionProps {
  agents: AgentSummaryForSidebar[]
  agentsExpanded: boolean
  setAgentsExpanded: (expanded: boolean) => void
  selectedAgentId: string | null
  toggleAgentFilter: (agentId: string) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onEditAgent?: (agentId: string) => void
  panelWidth: number
}

export default function AgentSection({
  agents,
  agentsExpanded,
  setAgentsExpanded,
  selectedAgentId,
  toggleAgentFilter,
  onNewChat,
  onEditAgent,
  panelWidth,
}: AgentSectionProps) {
  const { t } = useTranslation()
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  return (
    <div className="border-b border-border/50">
      <div className="flex items-center">
        <button
          className="flex items-center gap-1.5 flex-1 px-4 py-2 text-[11px] font-semibold text-muted-foreground uppercase tracking-wider hover:text-foreground transition-colors"
          onClick={() => setAgentsExpanded(!agentsExpanded)}
        >
          <ChevronRight
            className={cn(
              "h-3 w-3 transition-transform duration-200",
              agentsExpanded && "rotate-90",
            )}
          />
          <span>Agents</span>
          <span className="font-normal normal-case text-muted-foreground/60 ml-0.5">
            ({agents.length})
          </span>
        </button>
      </div>
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          agentsExpanded ? "max-h-[500px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <div
          className={cn(
            "px-2 pb-2 grid gap-1",
            panelWidth >= 280 ? "grid-cols-2" : "grid-cols-1",
          )}
        >
          {agents.map((agent) => {
            const isSelected = selectedAgentId === agent.id
            return (
              <ContextMenu key={agent.id}>
                <Tooltip>
                <ContextMenuTrigger asChild>
                  <TooltipTrigger asChild>
                  <div
                    className={cn(
                      "flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs transition-colors truncate group/agent",
                      isSelected ? "bg-primary/10" : "hover:bg-secondary/60",
                    )}
                  >
                    {/* Clickable area: single click = toggle filter, double click = new chat */}
                    <button
                      className="flex items-center gap-2 flex-1 min-w-0"
                      onClick={() => {
                        if (clickTimerRef.current) {
                          clearTimeout(clickTimerRef.current)
                          clickTimerRef.current = null
                        }
                        clickTimerRef.current = setTimeout(() => {
                          toggleAgentFilter(agent.id)
                          clickTimerRef.current = null
                        }, 250)
                      }}
                      onDoubleClick={() => {
                        if (clickTimerRef.current) {
                          clearTimeout(clickTimerRef.current)
                          clickTimerRef.current = null
                        }
                        onNewChat(agent.id)
                      }}
                    >
                      <div
                        className={cn(
                          "w-6 h-6 rounded-full flex items-center justify-center shrink-0 text-[10px] overflow-hidden",
                          isSelected
                            ? "bg-primary/25 text-primary"
                            : "bg-primary/15 text-primary",
                        )}
                      >
                        {agent.avatar ? (
                          <img
                            src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
                            className="w-full h-full object-cover"
                            alt=""
                          />
                        ) : agent.emoji ? (
                          <span>{agent.emoji}</span>
                        ) : (
                          <Bot className="h-3 w-3" />
                        )}
                      </div>
                      <span
                        className={cn(
                          "truncate",
                          isSelected ? "text-primary font-medium" : "text-foreground/80",
                        )}
                      >
                        {agent.name}
                      </span>
                    </button>
                    {/* New chat button */}
                    <IconTip label={t("chat.newChat")}>
                      <button
                        className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/agent:text-muted-foreground/60 hover:!text-primary transition-colors"
                        onClick={(e) => {
                          e.stopPropagation()
                          onNewChat(agent.id)
                        }}
                      >
                        <MessageSquarePlus className="h-3 w-3" />
                      </button>
                    </IconTip>
                  </div>
                  </TooltipTrigger>
                </ContextMenuTrigger>
                <TooltipContent>{agent.description || agent.name}</TooltipContent>
                </Tooltip>
                <ContextMenuContent>
                  <ContextMenuItem onClick={() => onNewChat(agent.id, { incognito: true })}>
                    <Ghost className="h-3 w-3 mr-2" />
                    {t("chat.newIncognitoChat")}
                  </ContextMenuItem>
                  {onEditAgent && (
                    <ContextMenuItem onClick={() => onEditAgent(agent.id)}>
                      <Settings className="h-3 w-3 mr-2" />
                      {t("common.settings")}
                    </ContextMenuItem>
                  )}
                </ContextMenuContent>
              </ContextMenu>
            )
          })}
        </div>
      </div>
    </div>
  )
}
