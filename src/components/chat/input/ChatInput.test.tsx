// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import type { ActiveModel, AvailableModel, SessionMode } from "@/types/chat"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import { TooltipProvider } from "@/components/ui/tooltip"
import ChatInput from "./ChatInput"
import IncognitoToggle from "./IncognitoToggle"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

type MockTransportCall = (command: string, args?: unknown) => Promise<unknown>
type MockDirEntry = { name: string; path: string; isDir: boolean }
type MockDirectoryResult = { path: string; entries: MockDirEntry[]; truncated: boolean }

const transportMock = vi.hoisted(() => {
  const defaultCall: MockTransportCall = (command) => {
    if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
    return Promise.resolve([])
  }
  return {
    defaultCall,
    call: vi.fn<MockTransportCall>(defaultCall),
    searchFiles: vi.fn(() => Promise.resolve({ entries: [], truncated: false })),
    listServerDirectory: vi.fn<() => Promise<MockDirectoryResult>>(() =>
      Promise.resolve({ path: "/tmp", entries: [], truncated: false }),
    ),
  }
})

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = vi.fn()
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  transportMock.call.mockImplementation(transportMock.defaultCall)
  transportMock.searchFiles.mockResolvedValue({ entries: [], truncated: false })
  transportMock.listServerDirectory.mockResolvedValue({
    path: "/tmp",
    entries: [],
    truncated: false,
  })
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

const inProgressTaskSnapshot: TaskProgressSnapshot = {
  tasks: [
    {
      id: 1,
      sessionId: "s1",
      content: "Run tests",
      activeForm: "Running tests",
      status: "in_progress",
      createdAt: "2026-04-29T00:00:00.000Z",
      updatedAt: "2026-04-29T00:00:00.000Z",
    },
  ],
  total: 1,
  completed: 0,
  remaining: 1,
  inProgress: true,
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

  test("allows sending when only attachments are present", () => {
    const onSend = vi.fn()
    const file = new File(["image"], "photo.png", { type: "image/png" })
    renderChatInput({ attachedFiles: [file], onSend })

    const sendButton = screen.getByRole("button", { name: "chat.send" }) as HTMLButtonElement
    expect(sendButton.disabled).toBe(false)

    fireEvent.click(sendButton)
    expect(onSend).toHaveBeenCalledTimes(1)
  })

  test.each([
    ["default", "smart"],
    ["smart", "yolo"],
    ["yolo", "default"],
  ] satisfies Array<[SessionMode, SessionMode]>)(
    "cycles permission mode from %s to %s with Shift+Tab",
    (permissionMode, nextMode) => {
      const onPermissionModeChange = vi.fn()
      renderChatInput({ permissionMode, onPermissionModeChange })

      fireEvent.keyDown(screen.getByRole("textbox"), { key: "Tab", shiftKey: true })

      expect(onPermissionModeChange).toHaveBeenCalledTimes(1)
      expect(onPermissionModeChange).toHaveBeenCalledWith(nextMode)
    },
  )

  test("does not cycle permission mode on plain Tab", () => {
    const onPermissionModeChange = vi.fn()
    renderChatInput({ onPermissionModeChange })

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Tab" })

    expect(onPermissionModeChange).not.toHaveBeenCalled()
  })

  test("lets slash command menu consume Shift+Tab before permission cycling", async () => {
    const onPermissionModeChange = vi.fn()
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
      if (command === "list_slash_commands") {
        return Promise.resolve([
          {
            name: "new",
            category: "session",
            descriptionKey: "slashCommands.new.description",
            hasArgs: false,
          },
        ])
      }
      return Promise.resolve([])
    })

    renderChatInput({ input: "/", onPermissionModeChange })

    await waitFor(() => expect(screen.getByText("/new")).toBeTruthy())
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Tab", shiftKey: true })

    expect(onPermissionModeChange).not.toHaveBeenCalled()
  })

  test("lets file mention menu consume Shift+Tab before permission cycling", async () => {
    const onInputChange = vi.fn()
    const onPermissionModeChange = vi.fn()
    transportMock.listServerDirectory.mockResolvedValue({
      path: "/tmp",
      entries: [{ name: "notes.md", path: "/tmp/notes.md", isDir: false }],
      truncated: false,
    })

    renderChatInput({
      input: "@",
      onInputChange,
      onPermissionModeChange,
      workingDir: "/tmp",
    })

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement
    textbox.setSelectionRange(1, 1)
    fireEvent.select(textbox)

    await waitFor(() => expect(screen.getByText("notes.md")).toBeTruthy())
    fireEvent.keyDown(textbox, { key: "Tab", shiftKey: true })

    expect(onPermissionModeChange).not.toHaveBeenCalled()
    expect(onInputChange).toHaveBeenCalledWith("@notes.md ")
  })

  test("explicit interrupted execution state wins over loading for task progress", () => {
    renderChatInput({
      loading: true,
      taskProgressSnapshot: inProgressTaskSnapshot,
      executionState: "interrupted",
    })

    expect(screen.getByText("chat.taskProgressWaiting")).toBeTruthy()
    expect(screen.queryByText("chat.taskProgressRunning")).toBeNull()
  })

  test("explicit cancelling execution state wins over loading for task progress", () => {
    renderChatInput({
      loading: true,
      taskProgressSnapshot: inProgressTaskSnapshot,
      executionState: "cancelling",
    })

    expect(screen.getByText("chat.taskProgressCancelling")).toBeTruthy()
    expect(screen.queryByText("chat.taskProgressRunning")).toBeNull()
  })

  test("explicit failed execution state wins over loading for task progress", () => {
    renderChatInput({
      loading: true,
      taskProgressSnapshot: inProgressTaskSnapshot,
      executionState: "failed",
    })

    expect(screen.getByText("chat.taskProgressFailed")).toBeTruthy()
    expect(screen.queryByText("chat.taskProgressRunning")).toBeNull()
  })
})
