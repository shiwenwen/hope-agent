import { useRef } from "react"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  arrayMove,
  rectSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
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
  Bot,
  Ghost,
  GripVertical,
  MessageSquarePlus,
  Settings,
} from "lucide-react"
import type { AgentSummaryForSidebar } from "@/types/chat"
import SidebarSectionHeader from "./SidebarSectionHeader"

interface AgentSectionProps {
  agents: AgentSummaryForSidebar[]
  agentsExpanded: boolean
  setAgentsExpanded: (expanded: boolean) => void
  selectedAgentId: string | null
  toggleAgentFilter: (agentId: string) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onEditAgent?: (agentId: string) => void
  onReorderAgents?: (agentIds: string[]) => void
  panelWidth: number
}

interface SortableAgentCardProps {
  agent: AgentSummaryForSidebar
  isSelected: boolean
  canReorder: boolean
  clickTimerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | null>
  toggleAgentFilter: (agentId: string) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onEditAgent?: (agentId: string) => void
}

function SortableAgentCard({
  agent,
  isSelected,
  canReorder,
  clickTimerRef,
  toggleAgentFilter,
  onNewChat,
  onEditAgent,
}: SortableAgentCardProps) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: agent.id,
    disabled: !canReorder,
  })

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.45 : 1,
    zIndex: isDragging ? 50 : undefined,
  }

  return (
    <div ref={setNodeRef} style={style}>
      <ContextMenu>
        <Tooltip>
          <ContextMenuTrigger asChild>
            <TooltipTrigger asChild>
              <div
                className={cn(
                  "flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs transition-colors truncate group/agent",
                  isSelected ? "bg-primary/10" : "hover:bg-secondary/60",
                )}
              >
                {canReorder && (
                  <span
                    className="shrink-0 cursor-grab active:cursor-grabbing text-muted-foreground/0 group-hover/agent:text-muted-foreground/50 hover:!text-muted-foreground/80 touch-none transition-colors"
                    onClick={(e) => e.stopPropagation()}
                    {...attributes}
                    {...listeners}
                  >
                    <GripVertical className="h-3 w-3" />
                  </span>
                )}
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
                {onEditAgent && (
                  <IconTip label={t("common.settings")}>
                    <button
                      className="shrink-0 p-0.5 rounded text-muted-foreground/0 group-hover/agent:text-muted-foreground/60 hover:!text-primary transition-colors"
                      onClick={(e) => {
                        e.stopPropagation()
                        onEditAgent(agent.id)
                      }}
                    >
                      <Settings className="h-3 w-3" />
                    </button>
                  </IconTip>
                )}
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
    </div>
  )
}

export default function AgentSection({
  agents,
  agentsExpanded,
  setAgentsExpanded,
  selectedAgentId,
  toggleAgentFilter,
  onNewChat,
  onEditAgent,
  onReorderAgents,
  panelWidth,
}: AgentSectionProps) {
  const { t } = useTranslation()
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))

  const handleDragEnd = (event: DragEndEvent) => {
    if (!onReorderAgents) return
    const { active, over } = event
    if (!over || active.id === over.id) return
    const oldIndex = agents.findIndex((agent) => agent.id === active.id)
    const newIndex = agents.findIndex((agent) => agent.id === over.id)
    if (oldIndex === -1 || newIndex === -1) return
    onReorderAgents(arrayMove(agents, oldIndex, newIndex).map((agent) => agent.id))
  }

  return (
    <div className="border-b border-border/50 px-3 pt-3 pb-1">
      <SidebarSectionHeader
        title={t("settings.agents")}
        count={agents.length}
        expanded={agentsExpanded}
        onToggle={() => setAgentsExpanded(!agentsExpanded)}
      />
      <div
        className={cn(
          "overflow-hidden transition-all duration-200 ease-out",
          agentsExpanded ? "max-h-[500px] opacity-100" : "max-h-0 opacity-0",
        )}
      >
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={handleDragEnd}
        >
          <SortableContext items={agents.map((agent) => agent.id)} strategy={rectSortingStrategy}>
            <div
              className={cn(
                "pb-2 grid gap-1",
                panelWidth >= 280 ? "grid-cols-2" : "grid-cols-1",
              )}
            >
              {agents.map((agent) => (
                <SortableAgentCard
                  key={agent.id}
                  agent={agent}
                  isSelected={selectedAgentId === agent.id}
                  canReorder={!!onReorderAgents}
                  clickTimerRef={clickTimerRef}
                  toggleAgentFilter={toggleAgentFilter}
                  onNewChat={onNewChat}
                  onEditAgent={onEditAgent}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      </div>
    </div>
  )
}
