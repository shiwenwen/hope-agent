// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import ContextCompactPanel from "./ContextCompactPanel"

const mocks = vi.hoisted(() => ({ call: vi.fn(), t: (key: string) => key }))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => mocks }))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: mocks.t, i18n: { language: "en" } }),
}))
vi.mock("@/components/ui/reasoning-effort-select", () => ({
  ReasoningEffortSelect: ({
    value,
    inheritLabel,
    onChange,
  }: {
    value: string | null
    inheritLabel?: string
    onChange: (value: string | null) => void
  }) => (
    <div>
      <span>{value ?? inheritLabel}</span>
      <button onClick={() => onChange("none")}>disable summary reasoning</button>
      <button onClick={() => onChange(null)}>restore summary policy</button>
    </div>
  ),
}))

const config = {
  enabled: true,
  cacheTtlSecs: 300,
  toolPolicies: {},
  maxToolResultContextShare: 0.3,
  softTrimRatio: 0.5,
  hardClearRatio: 0.7,
  preserveRecentRounds: 4,
  minPrunableToolChars: 20000,
  softTrimMaxChars: 6000,
  softTrimHeadChars: 2000,
  softTrimTailChars: 2000,
  hardClearEnabled: true,
  hardClearPlaceholder: "preserved placeholder",
  summarizationThreshold: 0.85,
  identifierPolicy: "strict",
  identifierInstructions: null,
  customInstructions: "preserved instruction",
  summarizationModel: null,
  modelOverride: { providerId: "dedicated", modelId: "summary-model" },
  summaryReasoningEffort: null,
  summarizationTimeoutSecs: 300,
  summaryMaxTokens: 4321,
  maxHistoryShare: 0.5,
  maxCompactionSummaryChars: 16000,
  maxCompactionInjectedContextShare: 0.5,
  reactiveMicrocompactEnabled: true,
  reactiveTriggerRatio: 0.75,
}
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe = vi.fn()
      unobserve = vi.fn()
      disconnect = vi.fn()
    },
  )
  mocks.call.mockReset().mockImplementation(async (command: string) => {
    if (command === "get_compact_config") return structuredClone(config)
    if (command === "list_builtin_tools" || command === "get_providers") return []
    return null
  })
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

test("explicit summary none is a draft until saved and preserves the dedicated model and budget", async () => {
  render(<ContextCompactPanel />)
  await screen.findByText("settings.contextCompactSummaryReasoningDefault")
  fireEvent.click(screen.getByRole("button", { name: "disable summary reasoning" }))
  expect(mocks.call.mock.calls.some(([command]) => command === "save_compact_config")).toBe(false)
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() =>
    expect(mocks.call).toHaveBeenCalledWith("save_compact_config", {
      config: { ...config, summaryReasoningEffort: "none" },
    }),
  )
  expect(
    mocks.call.mock.calls.some(
      ([command]) => command === "set_global_reasoning_effort" || command === "set_active_model",
    ),
  ).toBe(false)
})

test("restoring the summary policy writes null without changing other settings", async () => {
  mocks.call.mockImplementation(async (command: string) => {
    if (command === "get_compact_config")
      return { ...structuredClone(config), summaryReasoningEffort: "low" }
    if (command === "list_builtin_tools" || command === "get_providers") return []
    return null
  })
  render(<ContextCompactPanel />)
  await screen.findByText("low")
  fireEvent.click(screen.getByRole("button", { name: "restore summary policy" }))
  fireEvent.click(screen.getByRole("button", { name: "common.save" }))
  await waitFor(() => expect(mocks.call).toHaveBeenCalledWith("save_compact_config", { config }))
})
