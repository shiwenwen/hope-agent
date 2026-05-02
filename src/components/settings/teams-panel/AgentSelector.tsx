import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { AgentSelectDisplay } from "@/components/common/AgentSelectDisplay"
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
  const selectedAgent = agents.find((agent) => agent.id === value)

  return (
    <Select value={value} onValueChange={onChange} disabled={disabled || loading}>
      <SelectTrigger className="h-8 text-xs bg-secondary/40">
        {selectedAgent ? (
          <AgentSelectDisplay agent={selectedAgent} size="xs" />
        ) : (
          <SelectValue placeholder={loading ? "…" : "Select agent"} />
        )}
      </SelectTrigger>
      <SelectContent>
        {agents.map((a) => (
          <SelectItem key={a.id} value={a.id} textValue={a.name}>
            <AgentSelectDisplay agent={a} size="xs" />
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
