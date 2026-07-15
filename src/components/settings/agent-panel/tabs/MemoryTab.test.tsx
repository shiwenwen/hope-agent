// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import MemoryTab from "./MemoryTab"
import type { AgentConfig } from "../types"

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

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
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
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("@/components/ui/tabs", async () => {
  const React = await import("react")
  type TabsContextValue = {
    value?: string
    onValueChange?: (value: string) => void
  }
  const TabsContext = React.createContext<TabsContextValue>({})

  return {
    Tabs: ({
      value,
      onValueChange,
      children,
    }: {
      value?: string
      onValueChange?: (value: string) => void
      children: React.ReactNode
    }) => (
      <TabsContext.Provider value={{ value, onValueChange }}>
        <div>{children}</div>
      </TabsContext.Provider>
    ),
    TabsList: ({ children }: { children: React.ReactNode }) => (
      <div role="tablist">{children}</div>
    ),
    TabsTrigger: ({
      value,
      children,
    }: {
      value: string
      children: React.ReactNode
    }) => {
      const context = React.useContext(TabsContext)
      return (
        <button
          type="button"
          role="tab"
          aria-selected={context.value === value}
          onClick={() => context.onValueChange?.(value)}
        >
          {children}
        </button>
      )
    },
    TabsContent: ({
      value,
      children,
    }: {
      value: string
      children: React.ReactNode
    }) => {
      const context = React.useContext(TabsContext)
      return context.value === value ? <div role="tabpanel">{children}</div> : null
    },
  }
})

vi.mock("@/components/settings/memory-panel/useMemoryData", () => ({
  useMemoryData: () => ({ view: "list", items: [] }),
}))

vi.mock("@/components/settings/memory-panel/ExtractConfig", () => ({
  default: () => <div data-testid="extract-config" />,
}))

vi.mock("@/components/settings/memory-panel/MemoryFormView", () => ({
  default: () => <div data-testid="memory-form" />,
}))

vi.mock("@/components/settings/memory-panel/MemoryListView", () => ({
  default: () => <div data-testid="memory-list" />,
}))

vi.mock("@/components/settings/memory-panel/MemoryBudgetInputs", () => ({
  default: () => <div data-testid="memory-budget-inputs" />,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const baseConfig = {
  name: "Agent",
  model: {
    fallbacks: [],
  },
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

function renderMemoryTab() {
  return render(
    <MemoryTab
      agentId="agent-1"
      config={baseConfig}
      updateConfig={vi.fn()}
      openclawMode={false}
    />,
  )
}

function openManageTab() {
  fireEvent.click(screen.getByRole("tab", { name: "settings.memoryTabs.manage" }))
}

describe("Agent MemoryTab core memory editor", () => {
  it("shows redacted load failures without rendering an empty editor, then retries", async () => {
    let loadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "core_memory_get_cmd") {
        loadCalls += 1
        if (loadCalls === 1) throw new Error("agent core read failed token=agent-core-secret")
        return { content: "Prefer concise answers.", fileHash: "hash-1" }
      }
      return null
    })

    renderMemoryTab()
    openManageTab()

    expect(await screen.findByText("Failed to load agent core memory")).toBeTruthy()
    expect(screen.getByText("Details: agent core read failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/agent-core-secret/)).toBeNull()
    expect(screen.queryByPlaceholderText("settings.coreMemoryPlaceholder")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Retry" }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load agent core memory")).toBeNull()
    })
    expect(loadCalls).toBe(2)
    expect(
      (screen.getByPlaceholderText("settings.coreMemoryPlaceholder") as HTMLTextAreaElement).value,
    ).toBe("Prefer concise answers.")
  })

  it("shows redacted save failures for agent core memory edits", async () => {
    let saveArgs: Record<string, unknown> | undefined
    transportMock.call.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "core_memory_get_cmd") {
        return { content: "Prefer concise answers.", fileHash: "hash-1" }
      }
      if (command === "core_memory_save_cmd") {
        saveArgs = args
        throw new Error("agent core write failed api_key=agent-save-secret")
      }
      return null
    })

    renderMemoryTab()
    openManageTab()

    const editor = (await screen.findByPlaceholderText(
      "settings.coreMemoryPlaceholder",
    )) as HTMLTextAreaElement
    fireEvent.change(editor, { target: { value: "Prefer concise answers.\nUse examples." } })
    fireEvent.click(screen.getByRole("button", { name: /common\.save/ }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to save agent core memory", {
        description: "Details: agent core write failed api_key=[redacted]",
      }),
    )
    expect(screen.queryByText(/agent-save-secret/)).toBeNull()
    expect(saveArgs).toMatchObject({
      scopeType: "agent",
      scopeId: "agent-1",
      expectedFileHash: "hash-1",
    })
  })
})
