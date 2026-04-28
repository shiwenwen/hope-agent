/**
 * Agent switcher used in `ChatTitleBar`.
 *
 * Trigger: shows the current agent name. Click to open a popover listing all
 * available agents; pick one to call `onSelect(agentId)`.
 *
 * When `disabled` is true the trigger is rendered as a static label — used
 * after a session has already exchanged messages, since the agent_id is
 * baked into the system prompt and history at that point.
 *
 * Visual style mirrors the new-chat agent picker in
 * [src/components/chat/sidebar/ChatSidebar.tsx](sidebar/ChatSidebar.tsx) so
 * the two pickers feel consistent.
 */

import { useEffect, useRef, useState } from "react"
import { Bot, ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"
import { getTransport } from "@/lib/transport-provider"
import type { AgentSummaryForSidebar } from "@/types/chat"

interface AgentSwitcherProps {
  agents: AgentSummaryForSidebar[]
  currentAgentId: string
  agentName: string
  disabled?: boolean
  onSelect: (agentId: string) => void
}

export default function AgentSwitcher({
  agents,
  currentAgentId,
  agentName,
  disabled,
  onSelect,
}: AgentSwitcherProps) {
  const [open, setOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    function onClickOutside(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false)
      }
    }
    document.addEventListener("mousedown", onClickOutside)
    return () => document.removeEventListener("mousedown", onClickOutside)
  }, [open])

  if (disabled) {
    return <span className="text-sm font-medium text-foreground shrink-0">{agentName}</span>
  }

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "inline-flex items-center gap-0.5 text-sm font-medium text-foreground transition-colors",
          "hover:text-primary",
        )}
      >
        <span>{agentName}</span>
        <ChevronDown
          className={cn("h-3 w-3 text-muted-foreground transition-transform", open && "rotate-180")}
        />
      </button>
      {open && (
        <div className="absolute left-0 top-full mt-1 bg-popover/95 backdrop-blur-xl border border-border/60 rounded-xl shadow-lg z-50 min-w-[200px] p-1.5 animate-in fade-in-0 zoom-in-95 duration-150">
          {agents.length === 0 ? (
            <div className="px-2 py-1.5 text-[12px] text-muted-foreground italic">No agents</div>
          ) : (
            agents.map((agent) => {
              const isCurrent = agent.id === currentAgentId
              return (
                <button
                  key={agent.id}
                  className={cn(
                    "flex items-center gap-2 w-full px-2.5 py-1.5 text-[13px] rounded-md transition-colors",
                    isCurrent
                      ? "bg-primary/10 text-primary"
                      : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                  )}
                  onClick={() => {
                    onSelect(agent.id)
                    setOpen(false)
                  }}
                >
                  <div className="w-5 h-5 rounded-full bg-primary/15 flex items-center justify-center text-primary shrink-0 text-[10px] overflow-hidden">
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
                  <span className="truncate">{agent.name}</span>
                </button>
              )
            })
          )}
        </div>
      )}
    </div>
  )
}
