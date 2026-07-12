// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { TooltipProvider } from "@/components/ui/tooltip"
import type { AvailableModel } from "@/types/chat"
import ModelPicker from "./ModelPicker"
import type { UnsupportedModelBehavior } from "@/components/chat/model-capabilities"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string | { defaultValue?: string }) =>
      typeof fallback === "string" ? fallback : (fallback?.defaultValue ?? key),
  }),
}))

vi.mock("@/components/common/ProviderIcon", () => ({
  default: () => <span data-testid="provider-icon" />,
}))

const models: AvailableModel[] = [
  {
    providerId: "openai",
    providerName: "OpenAI",
    apiType: "openai-chat",
    modelId: "gpt-test",
    modelName: "GPT Test",
    inputTypes: ["text", "image", "video"],
    contextWindow: 128_000,
    maxTokens: 4_096,
    reasoning: true,
  },
  {
    providerId: "openai",
    providerName: "OpenAI",
    apiType: "openai-chat",
    modelId: "gpt-other",
    modelName: "GPT Other",
    inputTypes: ["text"],
    contextWindow: 128_000,
    maxTokens: 4_096,
    reasoning: true,
  },
]

function mockGeometry({
  viewportWidth,
  viewportHeight,
  left,
  right,
  top,
  bottom,
}: {
  viewportWidth: number
  viewportHeight: number
  left: number
  right: number
  top: number
  bottom: number
}) {
  Object.defineProperties(window, {
    innerWidth: { configurable: true, value: viewportWidth },
    innerHeight: { configurable: true, value: viewportHeight },
  })
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    bottom,
    height: bottom - top,
    left,
    right,
    top,
    width: right - left,
    x: left,
    y: top,
    toJSON: () => ({}),
  })
}

async function openModelSubmenu(options: {
  requiredInputTypes?: string[]
  unsupportedBehavior?: UnsupportedModelBehavior
} = {}) {
  const onModelChange = vi.fn()
  render(
    <TooltipProvider>
      <ModelPicker
        availableModels={models}
        activeModel={{ providerId: "openai", modelId: "gpt-test" }}
        reasoningEffort="high"
        onModelChange={onModelChange}
        onEffortChange={vi.fn()}
        currentModelInfo={models[0]}
        sessionTemperature={null}
        {...options}
      />
    </TooltipProvider>,
  )

  fireEvent.click(screen.getByRole("button", { name: /GPT Test/ }))
  const modelRow = await waitFor(() => {
    const row = screen
      .getAllByRole("button", { name: /GPT Test/ })
      .find((button) => button.getAttribute("aria-haspopup") === "menu")
    expect(row).toBeDefined()
    return row
  })

  fireEvent.mouseEnter(modelRow!)
  const submenu = await screen.findByRole("menu")
  return { modelRow, onModelChange, submenu }
}

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe("ModelPicker", () => {
  it("disables models missing required input types by default", async () => {
    const { onModelChange } = await openModelSubmenu({ requiredInputTypes: ["image"] })
    const unsupportedModel = screen.getByRole("button", { name: "GPT Other" })

    expect((unsupportedModel as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(unsupportedModel)
    expect(onModelChange).not.toHaveBeenCalled()
  })

  it("can hide models missing required input types", async () => {
    await openModelSubmenu({
      requiredInputTypes: ["image"],
      unsupportedBehavior: "hide",
    })

    expect(screen.queryByRole("button", { name: "GPT Other" })).toBeNull()
  })

  it("shows provider branding and model media capabilities", async () => {
    mockGeometry({
      viewportWidth: 1_200,
      viewportHeight: 800,
      left: 300,
      right: 560,
      top: 200,
      bottom: 600,
    })

    await openModelSubmenu()

    expect(screen.getAllByTestId("provider-icon").length).toBeGreaterThan(0)
    expect(screen.getByLabelText("model.supportsImageMultimodal")).toBeTruthy()
    expect(screen.getByLabelText("model.supportsVideoMultimodal")).toBeTruthy()
  })

  it("keeps a top-positioned model submenu open while the pointer crosses root items", async () => {
    mockGeometry({
      viewportWidth: 400,
      viewportHeight: 800,
      left: 70,
      right: 330,
      top: 300,
      bottom: 700,
    })

    const { modelRow, submenu } = await openModelSubmenu()
    expect(submenu.style.bottom).toBe("506px")
    expect(submenu.parentElement).toBe(document.body)
    expect(modelRow?.getAttribute("aria-expanded")).toBe("true")

    fireEvent.mouseEnter(screen.getByRole("button", { name: "effort.low" }))

    expect(modelRow?.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByRole("button", { name: "GPT Other" })).toBeTruthy()
  })

  it("prefers the right side when it has enough room", async () => {
    mockGeometry({
      viewportWidth: 1_200,
      viewportHeight: 800,
      left: 300,
      right: 560,
      top: 200,
      bottom: 600,
    })

    const { submenu } = await openModelSubmenu()

    expect(submenu.style.left).toBe("566px")
    expect(submenu.style.right).toBe("")
  })

  it("uses the left side when the right side is too narrow", async () => {
    mockGeometry({
      viewportWidth: 900,
      viewportHeight: 800,
      left: 500,
      right: 760,
      top: 200,
      bottom: 600,
    })

    const { onModelChange, submenu } = await openModelSubmenu()

    expect(submenu.style.left).toBe("")
    expect(submenu.style.right).toBe("406px")

    const otherModel = screen.getByRole("button", { name: "GPT Other" })
    fireEvent.mouseDown(otherModel)
    expect(screen.getByRole("button", { name: "GPT Other" })).toBeTruthy()
    fireEvent.click(otherModel)
    expect(onModelChange).toHaveBeenCalledWith("openai::gpt-other", {
      applyToAgentDefault: false,
    })
  })

  it("uses the roomier vertical side when neither horizontal side fits", async () => {
    mockGeometry({
      viewportWidth: 400,
      viewportHeight: 800,
      left: 70,
      right: 330,
      top: 50,
      bottom: 350,
    })

    const { submenu } = await openModelSubmenu()

    expect(submenu.style.top).toBe("356px")
  })
})
