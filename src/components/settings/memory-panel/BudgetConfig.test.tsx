// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import BudgetConfig from "./BudgetConfig"

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

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
  },
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

const runtimeConfig = {
  enabled: true,
  core: {
    enabled: true,
    totalTokens: 1600,
    hardMaxTokens: 2400,
    globalTokens: 350,
    agentTokens: 450,
    projectTokens: 650,
    protocolTokens: 150,
    topicReadMaxTokens: 800,
  },
  recall: {
    enabled: true,
    mode: "fast",
    maxTokens: 800,
    maxSelected: 5,
    candidateLimit: 24,
    timeoutMs: 100,
    includeClaims: true,
    includeProfile: true,
    includeProcedures: true,
    includeGraph: true,
  },
  deepRecall: {
    enabled: false,
    timeoutMs: 4500,
    cacheTtlSecs: 60,
    maxChars: 220,
    budgetTokens: 512,
  },
  learning: { mode: "smart", promoteCoreAutomatically: false },
  rollout: { enabled: true, dynamicRecall: true, coreRepository: true, shadowPlan: false },
  compatibility: { legacyStaticMemory: false },
}

const legacyBudgetConfig = {
  totalChars: 12345,
  coreMemoryFileChars: 2345,
  sqliteEntryMaxChars: 456,
  sqliteSections: {
    userProfile: 111,
    aboutUser: 222,
    preferences: 333,
    projectContext: 444,
    references: 555,
  },
}

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("BudgetConfig", () => {
  it("shows redacted load failure detail without rendering fallback defaults, then retries", async () => {
    let loadCalls = 0
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_memory_runtime_config") return runtimeConfig
      if (command === "get_memory_budget_config") {
        loadCalls += 1
        if (loadCalls === 1) throw new Error("budget read failed token=budget-secret")
        return legacyBudgetConfig
      }
      return null
    })

    render(<BudgetConfig />)
    fireEvent.click(screen.getByRole("button", { name: /settings\.memoryBudget\.title/ }))

    expect(await screen.findByText("Failed to load memory budget")).toBeTruthy()
    expect(screen.getByText("Details: budget read failed token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/budget-secret/)).toBeNull()
    expect(screen.queryByText("settings.memoryBudget.totalChars")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Retry" }))

    await waitFor(() => {
      expect(screen.queryByText("Failed to load memory budget")).toBeNull()
    })
    expect(loadCalls).toBe(2)
    expect(screen.getByText("settings.memoryBudget.totalChars")).toBeTruthy()
  })

  it("saves the two budget sections sequentially after a full reset", async () => {
    const initialRuntime = structuredClone(runtimeConfig)
    initialRuntime.core.totalTokens = 1700
    const saveOrder: string[] = []
    transportMock.call.mockImplementation(async (command: string, args?: { config?: unknown }) => {
      if (command === "get_memory_runtime_config") return initialRuntime
      if (command === "get_memory_budget_config") return legacyBudgetConfig
      if (command === "save_memory_runtime_config") {
        saveOrder.push(command)
        return args?.config
      }
      if (command === "save_memory_budget_config") {
        saveOrder.push(command)
        return { saved: true }
      }
      return null
    })

    render(<BudgetConfig />)
    fireEvent.click(screen.getByRole("button", { name: /settings\.memoryBudget\.title/ }))
    await screen.findByText("settings.memoryBudget.engine.totalTokens")

    fireEvent.click(
      screen.getByRole("button", { name: "settings.memoryBudget.resetToDefaults" }),
    )
    fireEvent.click(screen.getByRole("button", { name: "common.save" }))

    await waitFor(() => expect(saveOrder).toEqual([
      "save_memory_runtime_config",
      "save_memory_budget_config",
    ]))
  })
})
