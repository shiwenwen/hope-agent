import { TOOL_SUBAGENT } from "@/types/tools"
import type { AgentConfig } from "./types"

export const DEFAULT_SUBAGENT_TIMEOUT_SECS = 0
export const DEFAULT_SUBAGENT_ANNOUNCE_TIMEOUT_SECS = 120

export function createDefaultAgentModelConfig(): AgentConfig["model"] {
  return {
    primary: null,
    fallbacks: [],
    planModel: null,
    temperature: null,
    reasoningEffort: null,
  }
}

export function agentModelUsesDefaults(model: AgentConfig["model"]): boolean {
  return (
    model.primary == null &&
    model.fallbacks.length === 0 &&
    model.planModel == null &&
    model.temperature == null &&
    model.reasoningEffort == null
  )
}

export function createDefaultSubagentConfig(): AgentConfig["subagents"] {
  return {
    allowedAgents: [],
    deniedAgents: [],
    maxConcurrent: 8,
    defaultTimeoutSecs: DEFAULT_SUBAGENT_TIMEOUT_SECS,
    model: null,
    deniedTools: [],
    maxSpawnDepth: null,
    maxBatchSize: null,
    archiveAfterMinutes: null,
    announceTimeoutSecs: null,
  }
}

export function subagentTabUsesDefaults(config: AgentConfig): boolean {
  const subagents = config.subagents
  const hasToolOverride =
    config.capabilities.tools.allow.includes(TOOL_SUBAGENT) ||
    config.capabilities.tools.deny.includes(TOOL_SUBAGENT)

  return (
    !hasToolOverride &&
    subagents.allowedAgents.length === 0 &&
    subagents.deniedAgents.length === 0 &&
    subagents.maxConcurrent === 8 &&
    subagents.defaultTimeoutSecs === DEFAULT_SUBAGENT_TIMEOUT_SECS &&
    subagents.model == null &&
    (subagents.deniedTools?.length ?? 0) === 0 &&
    (subagents.maxSpawnDepth ?? 3) === 3 &&
    (subagents.maxBatchSize ?? 10) === 10 &&
    subagents.archiveAfterMinutes == null &&
    (subagents.announceTimeoutSecs ?? DEFAULT_SUBAGENT_ANNOUNCE_TIMEOUT_SECS) ===
      DEFAULT_SUBAGENT_ANNOUNCE_TIMEOUT_SECS
  )
}

export function createDefaultSubagentTabPatch(
  config: AgentConfig,
): Pick<AgentConfig, "capabilities" | "subagents"> {
  return {
    capabilities: {
      ...config.capabilities,
      tools: {
        allow: config.capabilities.tools.allow.filter((name) => name !== TOOL_SUBAGENT),
        deny: config.capabilities.tools.deny.filter((name) => name !== TOOL_SUBAGENT),
      },
    },
    subagents: createDefaultSubagentConfig(),
  }
}

export function agentApprovalUsesDefaults(config: AgentConfig): boolean {
  return (
    config.capabilities.defaultSessionPermissionMode == null &&
    !(config.capabilities.enableCustomToolApproval ?? false) &&
    (config.capabilities.customApprovalTools?.length ?? 0) === 0
  )
}

export function createDefaultApprovalTabPatch(
  config: AgentConfig,
): Pick<AgentConfig, "capabilities"> {
  return {
    capabilities: {
      ...config.capabilities,
      defaultSessionPermissionMode: null,
      enableCustomToolApproval: false,
      customApprovalTools: [],
    },
  }
}

export function agentToolSettingsUseDefaults(config: AgentConfig, toolsGuide: string): boolean {
  const capabilities = config.capabilities
  return (
    capabilities.maxToolRounds === 0 &&
    !capabilities.sandbox &&
    capabilities.defaultSandboxMode == null &&
    capabilities.tools.allow.length === 0 &&
    capabilities.tools.deny.length === 0 &&
    (capabilities.asyncToolPolicy ?? "model-decide") === "model-decide" &&
    (capabilities.mcpEnabled ?? true) &&
    toolsGuide.length === 0
  )
}

export function createDefaultToolSettingsPatch(
  config: AgentConfig,
): Pick<AgentConfig, "capabilities"> {
  return {
    capabilities: {
      ...config.capabilities,
      maxToolRounds: 0,
      sandbox: false,
      defaultSandboxMode: null,
      tools: { allow: [], deny: [] },
      asyncToolPolicy: "model-decide",
      mcpEnabled: true,
    },
  }
}

export function agentSkillSettingsUseDefaults(config: AgentConfig): boolean {
  return (
    config.capabilities.skills.allow.length === 0 &&
    config.capabilities.skills.deny.length === 0 &&
    (config.capabilities.skillEnvCheck ?? true)
  )
}

export function createDefaultSkillSettingsPatch(
  config: AgentConfig,
): Pick<AgentConfig, "capabilities"> {
  return {
    capabilities: {
      ...config.capabilities,
      skillEnvCheck: true,
      skills: { allow: [], deny: [] },
    },
  }
}
