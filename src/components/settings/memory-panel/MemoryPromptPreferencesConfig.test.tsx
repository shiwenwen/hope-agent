// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import MemoryPromptPreferencesConfig from "./MemoryPromptPreferencesConfig"
import { DEFAULT_MEMORY_PROMPT_PREFERENCES } from "./memoryPromptPreferences"

const transport = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transport }))
vi.mock("@/lib/logger", () => ({ logger: { error: vi.fn() } }))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

describe("memory writing preferences", () => {
  it("inherits each Agent stage without changing other overrides", async () => {
    transport.call.mockResolvedValue({
      promptPreferences: {
        ...DEFAULT_MEMORY_PROMPT_PREFERENCES,
        extraction: { style: "concise", supplemental: "retain dates" },
      },
    })
    const change = vi.fn()
    const profile = { style: "detailed" as const, supplemental: "work context" }
    render(<MemoryPromptPreferencesConfig overrides={{ profile }} onAgentChange={change} />)
    const group = await screen.findByRole("radiogroup", { name: "settings.memoryPromptPreferences.extraction" })
    expect(within(group).getByRole("radio", { name: "settings.memoryPromptPreferences.concise" })).toHaveAttribute("aria-checked", "true")
    expect(screen.getByDisplayValue("retain dates")).toBeDisabled()
    fireEvent.click(screen.getByRole("switch", { name: "settings.memoryPromptPreferences.extraction: settings.memoryPromptPreferences.inherit" }))
    expect(change).toHaveBeenCalledWith({
      profile, extraction: { style: "concise", supplemental: "retain dates" },
    })
    expect(transport.call).toHaveBeenCalledTimes(1)
  })

  it("preserves current recall consent when saving preferences", async () => {
    const initial = { promptPreferences: DEFAULT_MEMORY_PROMPT_PREFERENCES, recall: { enabled: false, userConfigured: false } }
    const latest = { ...initial, recall: { enabled: true, userConfigured: true }, core: { totalTokens: 2400 } }
    transport.call.mockResolvedValueOnce(initial).mockResolvedValueOnce(latest).mockImplementation(async (_command, args) => args.config)
    render(<MemoryPromptPreferencesConfig />)
    const group = await screen.findByRole("radiogroup", { name: "settings.memoryPromptPreferences.extraction" })
    fireEvent.click(within(group).getByRole("radio", { name: "settings.memoryPromptPreferences.detailed" }))
    fireEvent.click(screen.getByRole("button", { name: "settings.memoryPromptPreferences.save" }))
    await screen.findByRole("button", { name: "settings.memoryPromptPreferences.saved" })
    expect(transport.call).toHaveBeenLastCalledWith("save_memory_runtime_config", {
      config: { ...latest, promptPreferences: {
        ...DEFAULT_MEMORY_PROMPT_PREFERENCES,
        extraction: { style: "detailed", supplemental: "" },
      } },
    })
  })

  it("retains edits and reports persistence failure", async () => {
    transport.call.mockImplementation(async (command) => {
      if (command === "save_memory_runtime_config") throw new Error("write failed")
      return { promptPreferences: DEFAULT_MEMORY_PROMPT_PREFERENCES }
    })
    render(<MemoryPromptPreferencesConfig />)
    const group = await screen.findByRole("radiogroup", { name: "settings.memoryPromptPreferences.extraction" })
    fireEvent.click(within(group).getByRole("radio", { name: "settings.memoryPromptPreferences.concise" }))
    fireEvent.click(screen.getByRole("button", { name: "settings.memoryPromptPreferences.save" }))
    await waitFor(() => expect(screen.getByRole("button", { name: "settings.memoryPromptPreferences.error" })).toBeEnabled())
    expect(within(group).getByRole("radio", { name: "settings.memoryPromptPreferences.concise" })).toHaveAttribute("aria-checked", "true")
  })
})
