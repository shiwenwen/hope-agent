import { useRef } from "react"
import {
  DndContext,
  closestCenter,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import { SortableContext, arrayMove, rectSortingStrategy, useSortable } from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { AnimatedCollapse } from "@/components/ui/animated-presence"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { Bot, Ghost, GripVertical, MessageSquarePlus, Settings } from "lucide-react"
import type { AgentSummaryForSidebar } from "@/types/chat"
import SidebarSectionHeader from "./SidebarSectionHeader"
import type { SidebarDisplayMode } from "./types"

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
  displayMode: SidebarDisplayMode
}

const AGENT_CARD_MIN_WIDTH_PX = 156
const AGENT_COMPACT_CARD_MIN_WIDTH_PX = 104
const AGENT_GRID_GAP_PX = 4
const AGENT_SECTION_HORIZONTAL_PADDING_PX = 24
const AGENT_COMPACT_SECTION_HORIZONTAL_PADDING_PX = 16

interface SortableAgentCardProps {
  agent: AgentSummaryForSidebar
  isSelected: boolean
  canReorder: boolean
  clickTimerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | null>
  toggleAgentFilter: (agentId: string) => void
  onNewChat: (agentId: string, opts?: { incognito?: boolean }) => void
  onEditAgent?: (agentId: string) => void
  displayMode: SidebarDisplayMode
}

function SortableAgentCard({
  agent,
  isSelected,
  canReorder,
  clickTimerRef,
  toggleAgentFilter,
  onNewChat,
  onEditAgent,
  displayMode,
}: SortableAgentCardProps) {
  const { t } = useTranslation()
  const isCompact = displayMode === "compact"
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
    <div ref={setNodeRef} style={style} className="min-w-0">
      <ContextMenu>
        <Tooltip>
          <ContextMenuTrigger asChild>
            <TooltipTrigger asChild>
              <div
                className={cn(
                  "group/agent relative flex min-w-0 items-center text-xs transition-colors",
                  isCompact
                    ? "gap-1 rounded-md px-1.5 py-0.5"
                    : "gap-1.5 rounded-lg px-2 py-1.5",
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
                  className={cn(
                    "flex min-w-0 flex-1 items-center text-left transition-[padding]",
                    isCompact
                      ? "gap-1.5 group-hover/agent:pr-7"
                      : "gap-2 group-hover/agent:pr-9",
                  )}
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
                  {displayMode === "detailed" && (
                    <div
                      className={cn(
                        "w-6 h-6 rounded-full flex items-center justify-center shrink-0 text-[10px] overflow-hidden",
                        isSelected ? "bg-primary/25 text-primary" : "bg-primary/15 text-primary",
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
                  )}
                  <span
                    className={cn(
                      "min-w-0 flex-1 truncate",
                      isCompact && "text-[12px] leading-5",
                      isSelected ? "text-primary font-medium" : "text-foreground/80",
                    )}
                  >
                    {agent.name}
                  </span>
                </button>
                <div
                  className={cn(
                    "pointer-events-none absolute top-1/2 flex -translate-y-1/2 items-center gap-0.5 opacity-0 transition-opacity group-hover/agent:pointer-events-auto group-hover/agent:opacity-100",
                    isCompact ? "right-1" : "right-1.5",
                  )}
                >
                  {onEditAgent && (
                    <IconTip label={t("common.settings")}>
                      <button
                        className="shrink-0 rounded p-0.5 text-muted-foreground/60 transition-colors hover:!text-primary"
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
                      className="shrink-0 rounded p-0.5 text-muted-foreground/60 transition-colors hover:!text-primary"
                      onClick={(e) => {
                        e.stopPropagation()
                        onNewChat(agent.id)
                      }}
                    >
                      <MessageSquarePlus className="h-3 w-3" />
                    </button>
                  </IconTip>
                </div>
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
  displayMode,
}: AgentSectionProps) {
  const { t } = useTranslation()
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }))
  const agentGridWidth = Math.max(
    panelWidth -
      (displayMode === "compact"
        ? AGENT_COMPACT_SECTION_HORIZONTAL_PADDING_PX
        : AGENT_SECTION_HORIZONTAL_PADDING_PX),
    0,
  )
  const agentCardMinWidth =
    displayMode === "compact" ? AGENT_COMPACT_CARD_MIN_WIDTH_PX : AGENT_CARD_MIN_WIDTH_PX
  const agentColumnCount = Math.max(
    1,
    Math.min(
      agents.length || 1,
      Math.floor((agentGridWidth + AGENT_GRID_GAP_PX) / (agentCardMinWidth + AGENT_GRID_GAP_PX)),
    ),
  )

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
    <div className="contents">
      <SidebarSectionHeader
        title={t("settings.agents")}
        count={agents.length}
        expanded={agentsExpanded}
        onToggle={() => setAgentsExpanded(!agentsExpanded)}
        className="sticky top-0 z-20 mb-0 flex h-8 items-center border-b border-border/50 bg-surface-panel px-3"
      />
      <AnimatedCollapse open={agentsExpanded} unmountOnExit={false}>
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
          <SortableContext items={agents.map((agent) => agent.id)} strategy={rectSortingStrategy}>
            <div
              className={cn(
                "grid",
                displayMode === "compact"
                  ? "gap-0.5 px-2 pb-1.5 pt-0.5"
                  : "gap-1 px-3 pb-2 pt-1",
              )}
              style={{
                gridTemplateColumns: `repeat(${agentColumnCount}, minmax(0, 1fr))`,
              }}
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
                  displayMode={displayMode}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      </AnimatedCollapse>
    </div>
  )
}
