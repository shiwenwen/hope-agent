import { describe, expect, it } from "vitest"

import type { AgentConfig } from "./types"
import {
  agentSkillSettingsUseDefaults,
  agentToolSettingsUseDefaults,
  createDefaultAgentModelConfig,
  createDefaultSkillSettingsPatch,
  createDefaultSubagentConfig,
  createDefaultToolSettingsPatch,
} from "./agentTabDefaults"

function createConfig(): AgentConfig {
  return {
    enabled: true,
    name: "Agent",
    model: createDefaultAgentModelConfig(),
    personality: { traits: [], principles: [] },
    capabilities: {
      maxToolRounds: 20,
      sandbox: true,
      defaultSandboxMode: "workspace",
      skillEnvCheck: false,
      tools: { allow: ["browser"], deny: ["write"] },
      skills: { allow: ["authoring"], deny: ["legacy"] },
      asyncToolPolicy: "always-background",
      mcpEnabled: false,
      enableCustomToolApproval: true,
      customApprovalTools: ["browser"],
      defaultSessionPermissionMode: "smart",
    },
    openclawMode: false,
    subagents: createDefaultSubagentConfig(),
  }
}

describe("agent capability tab defaults", () => {
  it("restores tool settings without changing skills or approval settings", () => {
    const config = createConfig()
    expect(agentToolSettingsUseDefaults(config, "custom tool guidance")).toBe(false)

    const reset = { ...config, ...createDefaultToolSettingsPatch(config) }

    expect(agentToolSettingsUseDefaults(reset, "")).toBe(true)
    expect(reset.capabilities.skills).toEqual(config.capabilities.skills)
    expect(reset.capabilities.skillEnvCheck).toBe(false)
    expect(reset.capabilities.enableCustomToolApproval).toBe(true)
    expect(reset.capabilities.customApprovalTools).toEqual(["browser"])
    expect(reset.capabilities.defaultSessionPermissionMode).toBe("smart")
  })

  it("restores skill settings without changing tool settings", () => {
    const config = createConfig()
    expect(agentSkillSettingsUseDefaults(config)).toBe(false)

    const reset = { ...config, ...createDefaultSkillSettingsPatch(config) }

    expect(agentSkillSettingsUseDefaults(reset)).toBe(true)
    expect(reset.capabilities.tools).toEqual(config.capabilities.tools)
    expect(reset.capabilities.maxToolRounds).toBe(20)
    expect(reset.capabilities.asyncToolPolicy).toBe("always-background")
    expect(reset.capabilities.mcpEnabled).toBe(false)
  })
})
