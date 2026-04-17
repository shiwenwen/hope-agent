import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { AgentSummary } from "@/components/settings/types"

interface AgentSelectorProps {
  value: string
  onChange: (agentId: string) => void
  agents: AgentSummary[]
  loading?: boolean
  disabled?: boolean
}

export default function AgentSelector({
  value,
  onChange,
  agents,
  loading,
  disabled,
}: AgentSelectorProps) {
  return (
    <Select value={value} onValueChange={onChange} disabled={disabled || loading}>
      <SelectTrigger className="h-8 text-xs bg-secondary/40">
        <SelectValue placeholder={loading ? "…" : "Select agent"} />
      </SelectTrigger>
      <SelectContent>
        {agents.map((a) => (
          <SelectItem key={a.id} value={a.id}>
            <span className="inline-flex items-center gap-1.5">
              <span>{a.emoji ?? "🤖"}</span>
              <span>{a.name}</span>
              <span className="text-muted-foreground font-mono text-[10px]">({a.id})</span>
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
