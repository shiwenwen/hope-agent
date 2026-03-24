import SubagentPanelComponent from "@/components/settings/SubagentPanel"
import type { AgentConfig } from "../types"

interface SubagentTabProps {
  config: AgentConfig
  agentId: string
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function SubagentTab({ config, agentId, updateConfig }: SubagentTabProps) {
  return (
    <SubagentPanelComponent
      config={config.subagents}
      currentAgentId={agentId}
      onChange={(subagents) => updateConfig({ subagents })}
    />
  )
}
