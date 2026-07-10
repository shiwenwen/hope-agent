// @vitest-environment jsdom

import type { ReactNode } from "react"
import { createContext, useContext } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import DefaultAgentSection from "./DefaultAgentSection"

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

interface SelectMockContextValue {
  disabled?: boolean
  onValueChange?: (value: string) => void
}

const SelectMockContext = createContext<SelectMockContextValue>({})

vi.mock("@/components/ui/select", () => ({
  Select: ({
    children,
    disabled,
    onValueChange,
  }: {
    children: ReactNode
    disabled?: boolean
    onValueChange?: (value: string) => void
  }) => (
    <SelectMockContext.Provider value={{ disabled, onValueChange }}>
      <div data-testid="select-root">{children}</div>
    </SelectMockContext.Provider>
  ),
  SelectContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SelectItem: ({ children, value }: { children: ReactNode; value: string }) => {
    const { disabled, onValueChange } = useContext(SelectMockContext)
    return (
      <button
        type="button"
        role="option"
        disabled={disabled}
        onClick={() => onValueChange?.(value)}
      >
        {children}
      </button>
    )
  },
  SelectTrigger: ({ children }: { children: ReactNode }) => {
    const { disabled } = useContext(SelectMockContext)
    return (
      <button type="button" disabled={disabled}>
        {children}
      </button>
    )
  },
}))

vi.mock("@/components/common/AgentSelectDisplay", () => ({
  AgentSelectDisplay: ({
    agent,
    fallbackName,
  }: {
    agent?: { name?: string | null; id?: string } | null
    fallbackName?: string
  }) => <span>{agent?.name || fallbackName || agent?.id}</span>,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const agents = [
  {
    id: "agent-1",
    name: "Research agent",
    description: null,
    hasAgentMd: false,
    hasPersona: false,
    hasToolsGuide: false,
  },
  {
    id: "agent-2",
    name: "Writer agent",
    description: null,
    hasAgentMd: false,
    hasPersona: false,
    hasToolsGuide: false,
  },
]

describe("DefaultAgentSection", () => {
  it("shows a retryable, redacted load failure for the default agent selector", async () => {
    let loadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_default_agent_id") {
        loadCalls += 1
        if (loadCalls === 1) throw new Error("default load token=default-secret")
        return "agent-1"
      }
      return null
    })

    render(<DefaultAgentSection agents={agents} />)

    expect(await screen.findByText("Failed to load agent")).toBeTruthy()
    expect(screen.getByText("Details: default load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/default-secret/)).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: /common\.retry/ }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load agent")).toBeNull()
    })
    expect(loadCalls).toBe(2)
    expect(screen.getAllByText("Research agent").length).toBeGreaterThan(0)
  })

  it("rolls back and shows redacted detail when saving the default agent fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_default_agent_id") return "agent-1"
      if (command === "set_default_agent_id") {
        throw new Error("default save api_key=default-save-secret")
      }
      return null
    })

    render(<DefaultAgentSection agents={agents} />)

    await screen.findByText("Research agent")
    fireEvent.click(screen.getByRole("option", { name: "Writer agent" }))

    expect(await screen.findByText("Save failed")).toBeTruthy()
    expect(screen.getByText("Details: default save api_key=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/default-save-secret/)).toBeNull()

    await waitFor(() => {
      expect(screen.getAllByText("Research agent").length).toBeGreaterThan(0)
    })
  })
})
