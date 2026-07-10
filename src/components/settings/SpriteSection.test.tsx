// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import SpriteSection from "./SpriteSection"
import type { SpriteConfig } from "@/types/knowledge"

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
  listen: vi.fn(() => () => {}),
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

function defaultSpriteConfig(overrides: Partial<SpriteConfig> = {}): SpriteConfig {
  const config: SpriteConfig = {
    enabled: false,
    idleEditSecs: 8,
    minChangeChars: 80,
    cooldownSecs: 120,
    maxPerSessionPerHour: 6,
    periodicSecs: 300,
    pasteMinChars: 200,
    proactive: true,
    triggers: {
      editIdle: true,
      noteOpen: true,
      conversation: true,
      periodic: false,
      paste: true,
    },
    senses: {
      doc: true,
      edit: true,
      conversation: true,
      memory: true,
      awareness: true,
    },
    maxTokens: 240,
    timeoutSecs: 20,
  }
  return { ...config, ...overrides }
}

describe("SpriteSection", () => {
  it("shows load failures instead of a silent empty settings section", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "sprite_config_get_cmd") {
        throw new Error("sprite load token=sprite-load-secret")
      }
      return null
    })

    render(<SpriteSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Sprite \/ inspiration mode/i }))
    expect(await screen.findByText("Failed to load sprite settings")).toBeTruthy()
    expect(screen.getByText("Details: sprite load token=[redacted]")).toBeTruthy()
    expect(screen.queryByText(/sprite-load-secret/)).toBeNull()
  })

  it("rolls back the master toggle when saving fails", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "sprite_config_get_cmd") return defaultSpriteConfig()
      if (command === "sprite_config_set_cmd") {
        throw new Error("sprite toggle token=sprite-toggle-secret")
      }
      return null
    })

    render(<SpriteSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Sprite \/ inspiration mode/i }))
    expect(await screen.findByText("Enable sprite mode")).toBeTruthy()
    const masterSwitch = screen.getAllByRole("switch")[0]
    expect(masterSwitch.getAttribute("aria-checked")).toBe("false")
    fireEvent.click(masterSwitch)

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to update sprite mode", {
        description: "Details: sprite toggle token=[redacted]",
      }),
    )
    expect(masterSwitch.getAttribute("aria-checked")).toBe("false")
    expect(screen.queryByText(/sprite-toggle-secret/)).toBeNull()
  })

  it("shows save failures for tuning edits with redacted detail", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "sprite_config_get_cmd") return defaultSpriteConfig()
      if (command === "sprite_config_set_cmd") {
        throw new Error("sprite save token=sprite-save-secret")
      }
      return null
    })

    render(<SpriteSection />)

    fireEvent.click(await screen.findByRole("button", { name: /Sprite \/ inspiration mode/i }))
    expect(await screen.findByText("More proactive")).toBeTruthy()
    fireEvent.click(screen.getAllByRole("switch")[1])
    fireEvent.click(screen.getByRole("button", { name: /^Save$/i }))

    await waitFor(() =>
      expect(toastMock.error).toHaveBeenCalledWith("Failed to save sprite settings", {
        description: "Details: sprite save token=[redacted]",
      }),
    )
    expect(screen.queryByText(/sprite-save-secret/)).toBeNull()
  })
})
