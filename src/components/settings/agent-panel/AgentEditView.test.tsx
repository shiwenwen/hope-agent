// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import AgentEditView from "./AgentEditView"
import type { AgentConfig, AgentTab } from "./types"

const tMock = vi.hoisted(() => {
  return (
    key: string,
    options?:
      | string
      | ({
          defaultValue?: string
        } & Record<string, unknown>),
  ) => {
    let text =
      typeof options === "string"
        ? options
        : typeof options?.defaultValue === "string"
          ? options.defaultValue
          : key

    if (typeof options === "object") {
      for (const [name, value] of Object.entries(options)) {
        text = text.replaceAll(`{{${name}}}`, String(value))
      }
    }

    return text
  }
})

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: tMock,
    i18n: { language: "en" },
  }),
}))

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: toastMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  resolveAssetUrl: vi.fn((value: string) => value),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/hooks/useAvatarUpload", () => ({
  useAvatarUpload: () => ({
    cropSrc: null,
    handleAvatarPick: vi.fn(),
    handleCropCancel: vi.fn(),
    handleCropConfirm: vi.fn(),
  }),
}))

vi.mock("@/components/settings/AvatarCropDialog", () => ({
  AvatarCropDialog: () => null,
}))

vi.mock("./tabs/IdentityTab", () => ({
  default: () => <div data-testid="identity-tab" />,
}))

vi.mock("./tabs/PersonalityTab", () => ({
  default: () => <div data-testid="personality-tab" />,
}))

vi.mock("./tabs/CapabilitiesTab", () => ({
  default: () => <div data-testid="capabilities-tab" />,
}))

vi.mock("./tabs/ModelTab", () => ({
  default: () => <div data-testid="model-tab" />,
}))

vi.mock("./tabs/MemoryTab", () => ({
  default: () => <div data-testid="memory-tab" />,
}))

vi.mock("./tabs/SubagentTab", () => ({
  default: () => <div data-testid="subagent-tab" />,
}))

vi.mock("./tabs/ApprovalTab", () => ({
  default: () => <div data-testid="approval-tab" />,
}))

vi.mock("./tabs/CustomTab", () => ({
  default: () => <div data-testid="custom-tab" />,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const agentConfig = {
  enabled: true,
  name: "Research agent",
  description: null,
  emoji: null,
  avatar: null,
  model: { primary: null, fallbacks: [] },
  personality: {
    traits: [],
    principles: [],
  },
  capabilities: {
    maxToolRounds: 10,
    sandbox: true,
    skillEnvCheck: true,
    tools: { allow: [], deny: [] },
    skills: { allow: [], deny: [] },
  },
  openclawMode: false,
  subagents: {
    allowedAgents: [],
    deniedAgents: [],
    maxConcurrent: 2,
    defaultTimeoutSecs: 600,
  },
} satisfies AgentConfig

function mockSuccessfulLoadThenSaveFailure() {
  transportMock.call.mockImplementation(async (command: string) => {
    if (command === "get_agent_config") return structuredClone(agentConfig)
    if (command === "get_agent_markdown") return ""
    if (command === "get_skills") return []
    if (command === "list_builtin_tools") return []
    if (command === "get_available_models") return []
    if (command === "save_agent_config_cmd") {
      throw new Error("save failed token=agent-save-secret")
    }
    return null
  })
}

const tabResetCases: Array<{
  name: string
  tab: AgentTab
  configure: (config: AgentConfig) => void
  assertDefaults: (config: AgentConfig) => void
}> = [
  {
    name: "model",
    tab: "model",
    configure: (config) => {
      config.model = {
        primary: "provider::primary",
        fallbacks: ["provider::fallback"],
        planModel: "provider::plan",
        temperature: 0.4,
        reasoningEffort: "high",
      }
    },
    assertDefaults: (config) => {
      expect(config.model).toEqual({
        primary: null,
        fallbacks: [],
        planModel: null,
        temperature: null,
        reasoningEffort: null,
      })
    },
  },
  {
    name: "sub-agent",
    tab: "subagent",
    configure: (config) => {
      config.capabilities.tools = {
        allow: ["browser", "subagent"],
        deny: ["write", "subagent"],
      }
      config.subagents = {
        allowedAgents: ["researcher"],
        deniedAgents: ["writer"],
        maxConcurrent: 2,
        defaultTimeoutSecs: 1000,
        model: "provider::worker",
        deniedTools: ["browser"],
        maxSpawnDepth: 5,
        maxBatchSize: 30,
        archiveAfterMinutes: 15,
        announceTimeoutSecs: 300,
      }
    },
    assertDefaults: (config) => {
      expect(config.capabilities.tools).toEqual({ allow: ["browser"], deny: ["write"] })
      expect(config.subagents).toEqual({
        allowedAgents: [],
        deniedAgents: [],
        maxConcurrent: 8,
        defaultTimeoutSecs: 0,
        model: null,
        deniedTools: [],
        maxSpawnDepth: null,
        maxBatchSize: null,
        archiveAfterMinutes: null,
        announceTimeoutSecs: null,
        providerRetryAttempts: 3,
        providerRetryBackoffSecs: 5,
      })
    },
  },
  {
    name: "approval",
    tab: "approval",
    configure: (config) => {
      config.capabilities.defaultSessionPermissionMode = "smart"
      config.capabilities.enableCustomToolApproval = true
      config.capabilities.customApprovalTools = ["browser", "web_fetch"]
    },
    assertDefaults: (config) => {
      expect(config.capabilities.defaultSessionPermissionMode).toBeNull()
      expect(config.capabilities.enableCustomToolApproval).toBe(false)
      expect(config.capabilities.customApprovalTools).toEqual([])
      expect(config.capabilities.maxToolRounds).toBe(10)
    },
  },
]

describe("AgentEditView", () => {
  it.each(tabResetCases)("restores the whole $name tab to defaults", async (testCase) => {
    const configured = structuredClone(agentConfig) as AgentConfig
    testCase.configure(configured)
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_agent_config") return structuredClone(configured)
      if (command === "get_agent_markdown") return ""
      if (command === "get_skills") return []
      if (command === "list_builtin_tools") return []
      if (command === "get_available_models") return []
      return null
    })

    render(<AgentEditView agentId="agent-1" initialTab={testCase.tab} onBack={vi.fn()} />)

    expect(await screen.findByText("Research agent")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "settings.resetDefaultsTabAction" }))
    fireEvent.click(await screen.findByRole("button", { name: "common.restoreDefaults" }))
    fireEvent.click(screen.getByRole("button", { name: "common.save" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith(
        "save_agent_config_cmd",
        expect.objectContaining({ id: "agent-1" }),
      )
    })
    const saveCall = transportMock.call.mock.calls.find(
      ([command]) => command === "save_agent_config_cmd",
    )
    expect(saveCall).toBeTruthy()
    testCase.assertDefaults(saveCall?.[1].config as AgentConfig)
  })

  it("shows redacted detail when saving an agent config fails", async () => {
    mockSuccessfulLoadThenSaveFailure()

    render(<AgentEditView agentId="agent-1" initialTab="memory" onBack={vi.fn()} />)

    expect(await screen.findByText("Research agent")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "common.save" }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Save failed", {
        description: "Details: save failed token=[redacted]",
      }),
    )
    expect(screen.queryByText(/agent-save-secret/)).toBeNull()
  })

  it("previews dependencies and requires a replacement before deletion", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_agent_config") return structuredClone(agentConfig)
      if (command === "get_agent_markdown") return ""
      if (command === "get_skills") return []
      if (command === "list_builtin_tools") return []
      if (command === "get_available_models") return []
      if (command === "list_agents") return [{ id: "ha-main", name: "Hope", enabled: true }]
      if (command === "preview_agent_delete") {
        return {
          agentId: "agent-1",
          agentName: "Research agent",
          enabled: true,
          isMain: false,
          references: {
            globalConfig: 1,
            projects: 2,
            cronJobs: 1,
            otherAgentConfigs: 0,
            historicalSessions: 3,
            historicalSubagentRuns: 0,
            historicalTeams: 0,
            agentMemories: 4,
          },
          activeWork: {
            agentRuns: 0,
            foregroundSessions: 0,
            subagentRuns: 0,
            teams: 0,
            cronRuns: 0,
            backgroundJobs: 0,
          },
          hasHomeDir: true,
          hasPlanDir: true,
          blockers: [],
        }
      }
      if (command === "delete_agent") return { trashPath: "/trash/agent-1" }
      return null
    })

    const onBack = vi.fn()
    render(<AgentEditView agentId="agent-1" onBack={onBack} />)

    expect(await screen.findByText("Research agent")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "common.delete" }))
    expect(await screen.findByText("agentLifecycle.deleteTitle")).toBeTruthy()

    const confirmDeleteButton = await screen.findByRole("button", { name: "common.delete" })
    fireEvent.click(confirmDeleteButton)

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("delete_agent", {
        id: "agent-1",
        replacementAgentId: "ha-main",
      })
    })
    expect(onBack).toHaveBeenCalled()
  })

  it("can disable a non-main Agent without deleting its data", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_agent_config") return structuredClone(agentConfig)
      if (command === "get_agent_markdown") return ""
      if (command === "get_skills") return []
      if (command === "list_builtin_tools") return []
      if (command === "get_available_models") return []
      return null
    })

    render(<AgentEditView agentId="agent-1" onBack={vi.fn()} />)

    expect(await screen.findByText("Research agent")).toBeTruthy()
    fireEvent.click(screen.getByRole("switch", { name: "provider.disable" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("set_agent_enabled", {
        id: "agent-1",
        enabled: false,
      })
    })
  })
})
