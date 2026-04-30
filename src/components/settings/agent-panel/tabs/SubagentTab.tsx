import SubagentPanelComponent from "@/components/settings/SubagentPanel"
import type { AgentConfig } from "../types"

interface SubagentTabProps {
  config: AgentConfig
  agentId: string
  updateConfig: (patch: Partial<AgentConfig>) => void
}

export default function SubagentTab({ config, agentId, updateConfig }: SubagentTabProps) {
  const subagentEnabled = config.capabilities.capabilityToggles?.subagent ?? true
  const updateSubagentEnabled = (subagent: boolean) => {
    updateConfig({
      capabilities: {
        ...config.capabilities,
        capabilityToggles: {
          ...(config.capabilities.capabilityToggles ?? {}),
          subagent,
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
