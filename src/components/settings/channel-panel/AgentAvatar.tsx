import { getTransport } from "@/lib/transport-provider"
import { Bot } from "lucide-react"
import type { AgentInfo } from "./types"

export default function AgentAvatar({ agent, size = "sm" }: { agent: AgentInfo; size?: "sm" | "md" }) {
  const cls = size === "sm" ? "w-5 h-5 text-[10px]" : "w-6 h-6 text-xs"
  const iconCls = size === "sm" ? "h-3 w-3" : "h-3.5 w-3.5"
  return (
    <span className={`${cls} rounded-full bg-primary/15 flex items-center justify-center shrink-0 overflow-hidden`}>
      {agent.avatar ? (
        <img
          src={getTransport().resolveAssetUrl(agent.avatar) ?? agent.avatar}
          className="w-full h-full object-cover"
          alt=""
        />
      ) : agent.emoji ? (
        <span>{agent.emoji}</span>
      ) : (
        <Bot className={`${iconCls} text-muted-foreground`} />
      )}
    </span>
  )
}
