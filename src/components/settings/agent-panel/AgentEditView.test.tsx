// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import AgentEditView from "./AgentEditView"
import type { AgentConfig } from "./types"

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

describe("AgentEditView", () => {
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
})
