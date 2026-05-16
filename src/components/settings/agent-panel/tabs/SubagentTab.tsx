import SubagentPanelComponent from "@/components/settings/SubagentPanel"
import { TOOL_SUBAGENT } from "@/types/tools"
import type { AgentConfig } from "../types"

interface SubagentTabProps {
  config: AgentConfig
  agentId: string
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function SubagentTab({ config, agentId, updateConfig }: SubagentTabProps) {
  const subagentEnabled = !config.capabilities.tools.deny.includes(TOOL_SUBAGENT)
  const updateSubagentEnabled = (enabled: boolean) => {
    const allow = config.capabilities.tools.allow.filter((n) => n !== TOOL_SUBAGENT)
    const deny = config.capabilities.tools.deny.filter((n) => n !== TOOL_SUBAGENT)
    updateConfig({
      capabilities: {
        ...config.capabilities,
        tools: {
          allow,
          deny: enabled ? deny : [...deny, TOOL_SUBAGENT],
        },
      },
    })
  }

  return (
    <SubagentPanelComponent
      config={config.subagents}
      enabled={subagentEnabled}
      currentAgentId={agentId}
      onChange={(subagents) => updateConfig({ subagents })}
      onEnabledChange={updateSubagentEnabled}
    />
  )
}
