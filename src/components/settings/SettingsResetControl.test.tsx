// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"

import SettingsResetControl from "./SettingsResetControl"
import { RESET_SCOPE_BY_SECTION } from "./settingsReset"
import { AUTO_SEND_PENDING_EVENT } from "@/components/chat/autoSendPendingPreference"
import { CHAT_DISPLAY_MODE_EVENT, CHAT_DISPLAY_MODE_STORAGE_KEY } from "@/components/chat/chatDisplayModePreference"
import { COMPLETED_TURN_COLLAPSE_EVENT } from "@/components/chat/completedTurnCollapsePreference"

const tMock = vi.hoisted(() => (key: string, options?: Record<string, unknown>) => {
  let text = key
  for (const [name, value] of Object.entries(options ?? {})) {
    text = text.replaceAll(`{{${name}}}`, String(value))
  }
  return text
})

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: tMock }),
}))

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
}))

vi.mock("sonner", () => ({ toast: toastMock }))

const initLanguageMock = vi.hoisted(() => vi.fn(() => Promise.resolve()))
vi.mock("@/i18n/i18n", () => ({ initLanguageFromConfig: initLanguageMock }))

const invalidateThinkingMock = vi.hoisted(() => vi.fn())
vi.mock("@/components/chat/thinkingCache", () => ({
  invalidateThinkingExpandCache: invalidateThinkingMock,
}))

const transportMock = vi.hoisted(() => ({ call: vi.fn() }))
const localTauriTransportMock = vi.hoisted(() => ({ call: vi.fn() }))
const switchToEmbeddedMock = vi.hoisted(() => vi.fn())
const isTauriModeMock = vi.hoisted(() => vi.fn(() => false))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
  switchToEmbedded: switchToEmbeddedMock,
}))

vi.mock("@/lib/transport", () => ({
  isTauriMode: isTauriModeMock,
}))

vi.mock("@/lib/transport-tauri", () => ({
  TauriTransport: vi.fn(function MockTauriTransport() {
    return localTauriTransportMock
  }),
}))

if (!HTMLElement.prototype.hasPointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    value: () => false,
  })
}

if (!HTMLElement.prototype.setPointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    value: () => undefined,
  })
}

if (!HTMLElement.prototype.releasePointerCapture) {
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
    configurable: true,
    value: () => undefined,
  })
}

function createLocalStorage(): Storage {
  const values = new Map<string, string>()
  return {
    get length() {
      return values.size
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key)
    },
    setItem: (key, value) => {
      values.set(key, value)
    },
  }
}

beforeEach(() => {
  vi.stubGlobal("localStorage", createLocalStorage())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  isTauriModeMock.mockReturnValue(false)
  window.localStorage.clear()
  vi.unstubAllGlobals()
})

function openDialog() {
  fireEvent.click(screen.getByRole("button", { name: "settings.resetDefaultsPageAction" }))
  return screen.getByRole("alertdialog")
}

function confirmReset() {
  const dialog = screen.getByRole("alertdialog")
  fireEvent.click(within(dialog).getByRole("button", { name: "common.restoreDefaults" }))
}

describe("SettingsResetControl", () => {
  it("only exposes the 18 approved scopes and excludes global model settings", () => {
    expect(Object.keys(RESET_SCOPE_BY_SECTION)).toHaveLength(18)
    expect(RESET_SCOPE_BY_SECTION.modelConfig).toBeUndefined()

    const { container } = render(
      <SettingsResetControl section="modelConfig" sectionLabel="Model" onReset={vi.fn()} />,
    )
    expect(container.childElementCount).toBe(0)
  })

  it("requires confirmation and cancellation does not call the transport", () => {
    render(<SettingsResetControl section="general" sectionLabel="General" onReset={vi.fn()} />)

    const dialog = openDialog()
    expect(within(dialog).getByText("settings.resetDefaultsPreserve")).toBeTruthy()
    fireEvent.click(within(dialog).getByRole("button", { name: "common.cancel" }))

    expect(transportMock.call).not.toHaveBeenCalled()
    expect(screen.queryByRole("alertdialog")).toBeNull()
  })

  it("does not open the reset dialog while its target is busy", () => {
    render(
      <SettingsResetControl
        scope="approval"
        resetSection="protected_paths"
        sectionLabel="Protected paths"
        level="region"
        disabled
        onReset={vi.fn()}
      />,
    )

    const button = screen.getByRole("button", { name: "settings.resetDefaultsRegionAction" })
    expect(button).toHaveProperty("disabled", true)
    fireEvent.click(button)
    expect(screen.queryByRole("alertdialog")).toBeNull()
    expect(transportMock.call).not.toHaveBeenCalled()
  })

  it("sends a validated sub-section and does not show the page-level knowledge warning", async () => {
    transportMock.call.mockResolvedValue({
      scope: "knowledge",
      section: "search",
      changed: false,
      reindexStarted: false,
      warningCodes: [],
    })
    const onReset = vi.fn()
    render(
      <SettingsResetControl
        scope="knowledge"
        resetSection="search"
        sectionLabel="Search"
        level="region"
        onReset={onReset}
      />,
    )

    fireEvent.click(screen.getByRole("button", { name: "settings.resetDefaultsRegionAction" }))
    expect(screen.queryByText("settings.resetDefaultsKnowledgeWarning")).toBeNull()
    confirmReset()

    await waitFor(() => expect(onReset).toHaveBeenCalledTimes(1))
    expect(transportMock.call).toHaveBeenCalledWith("reset_settings_section", {
      scope: "knowledge",
      section: "search",
    })
    expect(toastMock.success).toHaveBeenCalledWith(
      "settings.resetDefaultsAlreadyDefault",
      { description: undefined },
    )
  })

  it("disables repeated confirmation, refreshes after success, and shows reindex status", async () => {
    let resolveReset: ((value: unknown) => void) | undefined
    transportMock.call.mockReturnValue(
      new Promise((resolve) => {
        resolveReset = resolve
      }),
    )
    const onReset = vi.fn()
    render(<SettingsResetControl section="knowledge" sectionLabel="Knowledge" onReset={onReset} />)

    const dialog = openDialog()
    expect(within(dialog).getByText("settings.resetDefaultsKnowledgeWarning")).toBeTruthy()
    confirmReset()

    await waitFor(() =>
      expect(
        within(screen.getByRole("alertdialog")).getByRole("button", {
          name: "common.restoreDefaults",
        }),
      ).toHaveProperty("disabled", true),
    )
    fireEvent.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "common.restoreDefaults",
      }),
    )
    expect(transportMock.call).toHaveBeenCalledTimes(1)
    expect(transportMock.call).toHaveBeenCalledWith("reset_settings_section", {
      scope: "knowledge",
    })

    resolveReset?.({
      scope: "knowledge",
      changed: true,
      reindexStarted: true,
      warningCodes: [],
    })
    await waitFor(() => expect(onReset).toHaveBeenCalledTimes(1))
    expect(toastMock.success).toHaveBeenCalledWith("settings.resetDefaultsSuccess", {
      description: "settings.resetDefaultsReindexStarted",
    })
    expect(screen.queryByRole("alertdialog")).toBeNull()
  })

  it("keeps the page mounted and the dialog open when reset fails", async () => {
    transportMock.call.mockRejectedValue(new Error("config locked"))
    const onReset = vi.fn()
    render(<SettingsResetControl section="chat" sectionLabel="Chat" onReset={onReset} />)

    openDialog()
    confirmReset()

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("settings.resetDefaultsFailed", {
        description: "config locked",
      }),
    )
    expect(onReset).not.toHaveBeenCalled()
    expect(screen.getByRole("alertdialog")).toBeTruthy()
  })

  it("resets local server config before switching a remote desktop client to embedded", async () => {
    isTauriModeMock.mockReturnValue(true)
    localTauriTransportMock.call.mockResolvedValue({
      scope: "server",
      changed: true,
      reindexStarted: false,
      warningCodes: [],
    })
    render(<SettingsResetControl section="server" sectionLabel="Server" onReset={vi.fn()} />)

    openDialog()
    confirmReset()

    await waitFor(() =>
      expect(switchToEmbeddedMock).toHaveBeenCalledWith({ dirtyConfirmed: true }),
    )
    expect(localTauriTransportMock.call).toHaveBeenCalledWith("reset_settings_section", {
      scope: "server",
    })
    expect(transportMock.call).not.toHaveBeenCalled()
  })

  it("resets server config through HTTP when running as a standalone web client", async () => {
    transportMock.call.mockResolvedValue({
      scope: "server",
      changed: true,
      reindexStarted: false,
      warningCodes: [],
    })
    render(<SettingsResetControl section="server" sectionLabel="Server" onReset={vi.fn()} />)

    openDialog()
    confirmReset()

    await waitFor(() => expect(switchToEmbeddedMock).toHaveBeenCalledTimes(1))
    expect(transportMock.call).toHaveBeenCalledWith("reset_settings_section", {
      scope: "server",
    })
    expect(localTauriTransportMock.call).not.toHaveBeenCalled()
  })

  it("synchronizes mounted chat preferences after a chat reset", async () => {
    transportMock.call
      .mockResolvedValueOnce({
        scope: "chat",
        changed: true,
        reindexStarted: false,
        warningCodes: [],
      })
      .mockResolvedValueOnce({
        autoSendPending: false,
        autoCollapseCompletedTurns: true,
        chatDisplayMode: null,
      })
    window.localStorage.setItem(CHAT_DISPLAY_MODE_STORAGE_KEY, "bubble")
    const displayListener = vi.fn()
    const autoSendListener = vi.fn()
    const collapseListener = vi.fn()
    window.addEventListener(CHAT_DISPLAY_MODE_EVENT, displayListener)
    window.addEventListener(AUTO_SEND_PENDING_EVENT, autoSendListener)
    window.addEventListener(COMPLETED_TURN_COLLAPSE_EVENT, collapseListener)

    render(<SettingsResetControl section="chat" sectionLabel="Chat" onReset={vi.fn()} />)
    openDialog()
    confirmReset()

    await waitFor(() => expect(invalidateThinkingMock).toHaveBeenCalledTimes(1))
    expect(window.localStorage.getItem(CHAT_DISPLAY_MODE_STORAGE_KEY)).toBeNull()
    expect(displayListener).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { mode: "timeline" } }),
    )
    expect(autoSendListener).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { enabled: false } }),
    )
    expect(collapseListener).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { enabled: true } }),
    )

    window.removeEventListener(CHAT_DISPLAY_MODE_EVENT, displayListener)
    window.removeEventListener(AUTO_SEND_PENDING_EVENT, autoSendListener)
    window.removeEventListener(COMPLETED_TURN_COLLAPSE_EVENT, collapseListener)
  })

  it("refreshes general-only frontend preferences after reset", async () => {
    transportMock.call
      .mockResolvedValueOnce({
        scope: "general",
        changed: true,
        reindexStarted: false,
        warningCodes: [],
      })
      .mockResolvedValueOnce({ chatDisplayMode: null })
      .mockResolvedValueOnce("compact")
    window.localStorage.setItem(CHAT_DISPLAY_MODE_STORAGE_KEY, "bubble")
    const effectsListener = vi.fn()
    const sidebarListener = vi.fn()
    window.addEventListener("ui-effects-changed", effectsListener)
    window.addEventListener("sidebar-display-mode-changed", sidebarListener)

    render(<SettingsResetControl section="general" sectionLabel="General" onReset={vi.fn()} />)
    openDialog()
    confirmReset()

    await waitFor(() => expect(initLanguageMock).toHaveBeenCalledTimes(1))
    expect(effectsListener).toHaveBeenCalledTimes(1)
    expect(sidebarListener).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { mode: "compact" } }),
    )
    expect(window.localStorage.getItem(CHAT_DISPLAY_MODE_STORAGE_KEY)).toBeNull()

    window.removeEventListener("ui-effects-changed", effectsListener)
    window.removeEventListener("sidebar-display-mode-changed", sidebarListener)
  })
})
