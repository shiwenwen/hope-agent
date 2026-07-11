// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import AgentListView from "./AgentListView"

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
  }),
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("./OpenClawImportDialog", () => ({
  default: () => <div data-testid="openclaw-import-dialog" />,
}))

vi.mock("./DefaultAgentSection", () => ({
  default: () => <div data-testid="default-agent-section" />,
}))

vi.mock("@/components/common/AgentSelectDisplay", () => ({
  AgentAvatarBadge: () => <div data-testid="agent-avatar" />,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("AgentListView", () => {
  it("shows a retryable, redacted list load failure instead of an empty-agent state", async () => {
    let listCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "list_agents") {
        listCalls += 1
        if (listCalls === 1) throw new Error("agent list failed token=agent-list-secret")
        return [{ id: "agent-1", name: "Research agent", description: null }]
      }
      return null
    })

    render(<AgentListView onEditAgent={vi.fn()} />)

    expect(await screen.findByText("Failed to load agent")).toBeTruthy()
    expect(screen.getByText("Details: agent list failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText("settings.agentNoAgents")).toBeNull()
    expect(screen.queryByText(/agent-list-secret/)).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: /common\.retry/ }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load agent")).toBeNull()
    })
    expect(listCalls).toBe(2)
    expect(await screen.findByText("Research agent")).toBeTruthy()
  })

  it("shows redacted detail when creating an agent fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "list_agents") return []
      if (command === "save_agent_config_cmd") {
        throw new Error("create failed api_key=create-agent-secret")
      }
      return null
    })

    render(<AgentListView onEditAgent={vi.fn()} />)

    fireEvent.click(await screen.findByRole("button", { name: "settings.agentNew" }))
    fireEvent.change(screen.getByPlaceholderText("settings.agentNewIdPlaceholder"), {
      target: { value: "research" },
    })
    fireEvent.click(screen.getByRole("button", { name: "common.add" }))

    expect(await screen.findByText("Save failed")).toBeTruthy()
    expect(screen.getByText("Details: create failed api_key=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/create-agent-secret/)).toBeNull()
  })
})
