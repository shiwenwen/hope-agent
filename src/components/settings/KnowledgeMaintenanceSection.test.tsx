// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import KnowledgeMaintenanceSection from "./KnowledgeMaintenanceSection"
import type { MaintenanceConfig } from "@/types/knowledge"

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
  useTranslation: () => ({ t: tMock }),
}))

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
  message: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: toastMock,
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    warn: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

if (!window.requestAnimationFrame) {
  Object.defineProperty(window, "requestAnimationFrame", {
    configurable: true,
    value: (cb: FrameRequestCallback) => window.setTimeout(() => cb(Date.now()), 0),
  })
}

if (!window.cancelAnimationFrame) {
  Object.defineProperty(window, "cancelAnimationFrame", {
    configurable: true,
    value: (id: number) => window.clearTimeout(id),
  })
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function defaultMaintenanceConfig(overrides: Partial<MaintenanceConfig> = {}): MaintenanceConfig {
  const config: MaintenanceConfig = {
    enabled: false,
    idleTrigger: { enabled: false, idleMinutes: 30 },
    cronTrigger: { enabled: false, cronExpr: "0 0 * * * *" },
    manualEnabled: true,
    tasks: {
      autoLink: true,
      orphanRescue: true,
      frontmatterFill: true,
      dedupMerge: false,
      knowledgeGap: true,
      autoTag: true,
      mocUpkeep: true,
      memoryToNote: false,
      sourceCompile: true,
      sourceConflict: true,
      openQuestionsMoc: true,
      forAgentSummary: true,
    },
    autoApprove: false,
    maxProposalsPerCycle: 20,
    dedupSimilarity: 0.92,
    llmTimeoutSecs: 60,
    llmMaxTokens: 1200,
  }
  return { ...config, ...overrides }
}

describe("KnowledgeMaintenanceSection", () => {
  it("shows load failures instead of a silent empty settings section", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "kb_maintenance_config_get_cmd") {
        throw new Error("maintenance load token=maintenance-load-secret")
      }
      return null
    })

    render(<KnowledgeMaintenanceSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Autonomous maintenance/i }))
    expect(await screen.findByText("Failed to load autonomous maintenance settings")).toBeTruthy()
    expect(screen.getByText("Details: maintenance load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/maintenance-load-secret/)).toBeNull()
  })

  it("shows save failures for maintenance config edits with redacted detail", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "kb_maintenance_config_get_cmd") return defaultMaintenanceConfig()
      if (command === "kb_maintenance_config_set_cmd") {
        throw new Error("maintenance save token=maintenance-save-secret")
      }
      return null
    })

    render(<KnowledgeMaintenanceSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Autonomous maintenance/i }))
    expect(await screen.findByText("Enable background maintenance")).toBeTruthy()
    fireEvent.click(screen.getAllByRole("switch")[0])
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith(
        "Failed to save autonomous maintenance settings",
        {
          description: "Details: maintenance save token=[redacted]",
        },
      ),
    )
    expect(screen.queryByText(/maintenance-save-secret/)).toBeNull()
  })

  it("shows manual maintenance run failures with redacted detail", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "kb_maintenance_config_get_cmd") return defaultMaintenanceConfig()
      if (command === "kb_maintenance_run_cmd") {
        throw new Error("maintenance run token=maintenance-run-secret")
      }
      return null
    })

    render(<KnowledgeMaintenanceSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Autonomous maintenance/i }))
    fireEvent.click(await screen.findByRole("button", { name: /Scan now/i }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to run autonomous maintenance", {
        description: "Details: maintenance run token=[redacted]",
      }),
    )
    expect(screen.queryByText(/maintenance-run-secret/)).toBeNull()
  })
})
