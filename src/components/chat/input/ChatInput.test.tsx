// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"

import type { ActiveModel, AvailableModel } from "@/types/chat"
import { TooltipProvider } from "@/components/ui/tooltip"
import ChatInput from "./ChatInput"
import IncognitoToggle from "./IncognitoToggle"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(() => Promise.resolve([])),
  searchFiles: vi.fn(() => Promise.resolve({ entries: [], truncated: false })),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const model: AvailableModel = {
  providerId: "openai",
  providerName: "OpenAI",
  apiType: "openai-chat",
  modelId: "gpt-test",
  modelName: "GPT Test",
  inputTypes: ["text"],
  contextWindow: 128_000,
  maxTokens: 4_096,
  reasoning: true,
}

const activeModel: ActiveModel = {
  providerId: model.providerId,
  modelId: model.modelId,
}

function renderChatInput(overrides: Partial<Parameters<typeof ChatInput>[0]> = {}) {
  const props: Parameters<typeof ChatInput>[0] = {
    input: "",
    onInputChange: vi.fn(),
    onSend: vi.fn(),
    loading: false,
    availableModels: [model],
    activeModel,
    reasoningEffort: "medium",
    onModelChange: vi.fn(),
    onEffortChange: vi.fn(),
    attachedFiles: [],
    onAttachFiles: vi.fn(),
    onRemoveFile: vi.fn(),
    permissionMode: "default",
    onPermissionModeChange: vi.fn(),
    ...overrides,
  }

  return {
    props,
    view: render(
      <TooltipProvider>
        <ChatInput {...props} />
      </TooltipProvider>,
    ),
  }
}

describe("IncognitoToggle", () => {
  test("emits the next enabled state", () => {
    const onChange = vi.fn()
    render(
      <TooltipProvider>
        <IncognitoToggle sessionId={null} enabled={false} onChange={onChange} />
      </TooltipProvider>,
    )

    fireEvent.click(screen.getByRole("button", { name: "chat.incognito" }))

    expect(onChange).toHaveBeenCalledWith(true)
  })

  test("stays disabled when project or channel context forbids incognito", () => {
    const onChange = vi.fn()
    render(
      <TooltipProvider>
        <IncognitoToggle
          sessionId="s1"
          enabled={false}
          disabledReason="project"
          onChange={onChange}
        />
      </TooltipProvider>,
    )

    const button = screen.getByRole("button", { name: "chat.incognito" }) as HTMLButtonElement
    expect(button.disabled).toBe(true)
    fireEvent.click(button)
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe("ChatInput", () => {
  test("forwards textarea changes and disables empty sends", () => {
    const onInputChange = vi.fn()
    const onSend = vi.fn()
    const { props, view } = renderChatInput({ onInputChange, onSend })

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "hello" } })
    expect(onInputChange).toHaveBeenCalledWith("hello")
    expect((screen.getByRole("button", { name: "chat.send" }) as HTMLButtonElement).disabled).toBe(
      true,
    )

    view.rerender(
      <TooltipProvider>
        <ChatInput {...props} input="hello" />
      </TooltipProvider>,
    )
    fireEvent.click(screen.getByRole("button", { name: "chat.send" }))
    expect(onSend).toHaveBeenCalledTimes(1)
  })

  test("renders incognito controls when enabled by the caller", () => {
    const onIncognitoChange = vi.fn()
    renderChatInput({
      currentSessionId: null,
      incognitoEnabled: true,
      onIncognitoChange,
    })

    fireEvent.click(screen.getByRole("button", { name: "chat.incognito" }))

    expect(onIncognitoChange).toHaveBeenCalledWith(false)
  })
})
