// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import type { ActiveModel, AvailableModel, SessionMode } from "@/types/chat"
import type { TaskProgressSnapshot } from "@/components/chat/tasks/taskProgress"
import { TooltipProvider } from "@/components/ui/tooltip"
import type { GoalSnapshot } from "@/components/chat/workspace/useGoal"
import ChatInput from "./ChatInput"
import IncognitoToggle from "./IncognitoToggle"
import { getPastedTextFileMeta } from "./pastedTextAttachment"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

// MentionComposerInput is a heavy CodeMirror 6 editor (Phase 1 composer refactor)
// that doesn't drive its updateListener reliably under jsdom. Stub it with a
// plain contenteditable so these ChatInput-wiring tests can fire onChange without
// simulating CM internals; the editor's own behavior is covered elsewhere.
vi.mock("./MentionComposerInput", async () => {
  const React = await import("react")
  return {
    default: React.forwardRef(function MockComposer(
      props: {
        value?: string
        onChange?: (v: string) => void
        onKeyDown?: (e: React.KeyboardEvent<HTMLElement>) => void
        onPaste?: (e: {
          clipboardData: DataTransfer | null
          preventDefault: () => void
          defaultPrevented?: boolean
        }) => void
        onSelectionChange?: () => void
        readOnly?: boolean
      },
      ref: React.Ref<{
        focus: () => void
        getValue: () => string
        getSelectionRange: () => { start: number; end: number }
        setSelectionRange: (start: number, end: number) => void
      }>,
    ) {
      const value = props.value ?? ""
      // Emulate the real ComposerInputHandle so the `@`-mention hook (which reads
      // the caret via getSelectionRange) can drive the popper under jsdom. Caret
      // sits at end-of-value, which is where these wiring tests expect it.
      React.useImperativeHandle(
        ref,
        () => ({
          focus: () => {},
          getValue: () => value,
          getSelectionRange: () => ({ start: value.length, end: value.length }),
          setSelectionRange: () => {},
        }),
        [value],
      )
      return React.createElement("div", {
        role: "textbox",
        "aria-multiline": "true",
        contentEditable: !props.readOnly,
        suppressContentEditableWarning: true,
        onInput: (e: React.FormEvent<HTMLDivElement>) =>
          props.onChange?.(e.currentTarget.textContent ?? ""),
        onKeyDown: props.onKeyDown,
        onSelect: props.onSelectionChange,
        onPaste: (e: React.ClipboardEvent<HTMLElement>) => props.onPaste?.(e),
      })
    }),
  }
})

type MockTransportCall = (command: string, args?: unknown) => Promise<unknown>
type MockDirEntry = { name: string; path: string; isDir: boolean }
type MockDirectoryResult = { path: string; entries: MockDirEntry[]; truncated: boolean }
type MockFileMatch = {
  name: string
  path: string
  relPath: string
  isDir: boolean
  score: number
}
type MockSearchResult = { root: string; matches: MockFileMatch[]; truncated: boolean }

const transportMock = vi.hoisted(() => {
  const defaultCall: MockTransportCall = (command) => {
    if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
    return Promise.resolve([])
  }
  return {
    defaultCall,
    call: vi.fn<MockTransportCall>(defaultCall),
    searchFiles: vi.fn<() => Promise<MockSearchResult>>(() =>
      Promise.resolve({ root: "", matches: [], truncated: false }),
    ),
    supportsLocalFileOps: () => false,
    listen: vi.fn(() => () => {}),
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
  transportMock.searchFiles.mockResolvedValue({ root: "", matches: [], truncated: false })
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

const activeGoalSnapshot: GoalSnapshot = {
  goal: {
    id: "goal-1",
    sessionId: "s1",
    objective: "Complete Goal v2 review",
    completionCriteria: "[required] code is reviewed\n[required] GUI path works",
    revision: 4,
    state: "active",
    modeSnapshot: null,
    budgetTokenLimit: null,
    budgetTimeLimitSecs: null,
    budgetTurnLimit: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:10:00Z",
    completedAt: null,
    finalSummary: null,
    finalEvidence: {},
    blockedReason: null,
    lastEvaluatorResult: {},
  },
  links: [],
  events: [],
  auditStale: false,
  criteriaItems: [
    { id: "criterion-1", text: "code is reviewed", kind: "required" },
    { id: "criterion-2", text: "GUI path works", kind: "required" },
  ],
  criteria: [
    {
      id: "criterion-1",
      text: "code is reviewed",
      kind: "required",
      status: "satisfied",
      evidenceIds: ["workflow:wf-1"],
      reason: "Reviewed.",
    },
    {
      id: "criterion-2",
      text: "GUI path works",
      kind: "required",
      status: "missing",
      evidenceIds: [],
      reason: "Needs GUI evidence.",
    },
  ],
  evidence: [],
  timeline: [],
  budget: undefined,
  workflowRuns: [],
  tasks: [],
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
    sandboxMode: "off",
    onSandboxModeChange: vi.fn(),
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
  test("forwards composer changes and disables empty sends", () => {
    const onInputChange = vi.fn()
    const onSend = vi.fn()
    const { props, view } = renderChatInput({ onInputChange, onSend })

    const textbox = screen.getByRole("textbox")
    textbox.textContent = "hello"
    fireEvent.input(textbox)
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

  test("turns long pasted text into a staged text attachment", async () => {
    const onAttachFiles = vi.fn()
    const onInputChange = vi.fn()
    renderChatInput({ onAttachFiles, onInputChange })

    const longText = "# THIS IS AN AUTO ATTACHMENT\n" + "line\n".repeat(40)
    fireEvent.paste(screen.getByRole("textbox"), {
      clipboardData: {
        items: [],
        getData: (type: string) => (type === "text/plain" ? longText : ""),
      },
    })

    expect(onInputChange).not.toHaveBeenCalled()
    expect(onAttachFiles).toHaveBeenCalledTimes(1)
    const files = onAttachFiles.mock.calls[0]?.[0] as File[]
    expect(files).toHaveLength(1)
    expect(files[0].type).toBe("text/plain")
    expect(await files[0].text()).toBe(longText)
    expect(getPastedTextFileMeta(files[0])?.source).toBe("pasted_text")
  })

  test("keeps the input dock from clipping upward toolbar menus", () => {
    renderChatInput()

    const inputDock = screen.getByRole("textbox").closest(".rounded-input-dock")

    expect(inputDock).toBeTruthy()
    expect(inputDock?.className).toContain("overflow-visible")
    expect(inputDock?.className).not.toContain("overflow-hidden")
  })

  test("insets the context usage bar inside the rounded input dock corners", () => {
    const { view } = renderChatInput({
      contextUsage: {
        usedTokens: 12_000,
        contextWindow: 128_000,
        usedK: 12,
        ctxK: 128,
        pct: 9,
      },
    })

    expect(view.container.querySelector(".absolute.inset-x-4.bottom-0")).toBeTruthy()
    expect(view.container.querySelector(".h-full.rounded-full")).toBeTruthy()
    expect(view.container.querySelector(".absolute.inset-x-0.bottom-0")).toBeNull()
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

  test("syncs workflow mode status from slash command events", async () => {
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
      if (command === "get_workflow_mode") return Promise.resolve({ mode: "off" })
      return Promise.resolve([])
    })

    renderChatInput({ currentSessionId: "s1" })

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("get_workflow_mode", { sessionId: "s1" })
    })
    expect(screen.queryByText("chat.workflowMode.active")).toBeNull()

    act(() => {
      window.dispatchEvent(
        new CustomEvent("hope-agent:workflow-mode-changed", {
          detail: { sessionId: "other", mode: "ultracode" },
        }),
      )
    })
    expect(screen.queryByText("chat.workflowMode.active")).toBeNull()

    act(() => {
      window.dispatchEvent(
        new CustomEvent("hope-agent:workflow-mode-changed", {
          detail: { sessionId: "s1", mode: "on" },
        }),
      )
    })

    expect(await screen.findByText("自动")).toBeTruthy()
    expect(screen.getByText("chat.workflowMode.activeOnDetail")).toBeTruthy()
  })

  test("submits the current draft through goal mode instead of normal send", async () => {
    const onGoalModeSubmit = vi.fn(() => Promise.resolve(true))
    const onInputChange = vi.fn()
    const onSend = vi.fn()
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
      () =>
        ({
          x: 0,
          y: 0,
          width: 1200,
          height: 80,
          top: 0,
          left: 0,
          right: 1200,
          bottom: 80,
          toJSON: () => ({}),
        }) as DOMRect,
    )

    try {
      renderChatInput({
        input: "Complete Goal v2 review",
        onGoalModeSubmit,
        onInputChange,
        onSend,
      })

      fireEvent.click(await screen.findByRole("button", { name: "chat.goalMode.enter" }))
      expect(screen.getByText("chat.goalMode.restricted")).toBeTruthy()

      fireEvent.click(screen.getByRole("button", { name: "chat.send" }))

      await waitFor(() => {
        expect(onGoalModeSubmit).toHaveBeenCalledWith("Complete Goal v2 review")
      })
      expect(onSend).not.toHaveBeenCalled()
      expect(onInputChange).toHaveBeenCalledWith("")
    } finally {
      rectSpy.mockRestore()
    }
  })

  test("treats /goal drafts as goal mode when sent directly", async () => {
    const onGoalModeSubmit = vi.fn(() => Promise.resolve(true))
    const onInputChange = vi.fn()
    const onSend = vi.fn()

    renderChatInput({
      input: "/goal Complete the product-grade UI pass",
      onGoalModeSubmit,
      onInputChange,
      onSend,
    })

    fireEvent.click(screen.getByRole("button", { name: "chat.send" }))

    await waitFor(() => {
      expect(onGoalModeSubmit).toHaveBeenCalledWith("Complete the product-grade UI pass")
    })
    expect(onSend).not.toHaveBeenCalled()
    expect(onInputChange).toHaveBeenCalledWith("")
  })

  test("lets /goal drafts bypass slash execution on Enter", async () => {
    const onGoalModeSubmit = vi.fn(() => Promise.resolve(true))
    const onCommandAction = vi.fn()
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
      if (command === "list_slash_commands") {
        return Promise.resolve([
          {
            name: "goal",
            category: "utility",
            descriptionKey: "slashCommands.goal.description",
            hasArgs: true,
            argsOptional: true,
            argOptions: ["status", "pause", "resume", "clear"],
          },
        ])
      }
      if (command === "execute_slash_command") {
        return Promise.resolve({
          content: "should not execute",
          action: { type: "displayOnly" },
        })
      }
      return Promise.resolve([])
    })

    renderChatInput({
      input: "/goal Build a durable goal flow",
      onGoalModeSubmit,
      onCommandAction,
    })

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("list_slash_commands")
    })
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" })

    await waitFor(() => {
      expect(onGoalModeSubmit).toHaveBeenCalledWith("Build a durable goal flow")
    })
    expect(onCommandAction).not.toHaveBeenCalled()
    expect(transportMock.call).not.toHaveBeenCalledWith(
      "execute_slash_command",
      expect.anything(),
    )
  })

  test("keeps goal composer and plan composer mutually exclusive", async () => {
    const onExitPlanMode = vi.fn(() => Promise.resolve())

    renderChatInput({
      planState: "planning",
      onExitPlanMode,
    })

    expect(screen.getByText("planMode.restricted")).toBeTruthy()

    fireEvent.click(await screen.findByRole("button", { name: "chat.goalMode.enter" }))

    expect(onExitPlanMode).toHaveBeenCalledTimes(1)
    expect(screen.getByText("chat.goalMode.restricted")).toBeTruthy()
    await waitFor(() => {
      expect(screen.queryByText("planMode.restricted")).toBeNull()
    })
  })

  test("passes the selected active-goal action when goal mode appends follow-up", async () => {
    const onGoalModeSubmit = vi.fn(() => Promise.resolve(true))
    const onInputChange = vi.fn()
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
      () =>
        ({
          x: 0,
          y: 0,
          width: 1200,
          height: 80,
          top: 0,
          left: 0,
          right: 1200,
          bottom: 80,
          toJSON: () => ({}),
        }) as DOMRect,
    )

    try {
      renderChatInput({
        input: "Manual browser smoke",
        goalSnapshot: activeGoalSnapshot,
        onGoalModeSubmit,
        onInputChange,
      })

      fireEvent.click(await screen.findByRole("button", { name: "chat.goalMode.enter" }))
      expect(screen.getByText("chat.goalMode.activeRestricted")).toBeTruthy()

      fireEvent.click(screen.getByRole("button", { name: "chat.goalMode.actionFollowUp" }))
      fireEvent.click(screen.getByRole("button", { name: "chat.send" }))

      await waitFor(() => {
        expect(onGoalModeSubmit).toHaveBeenCalledWith(
          "Manual browser smoke",
          "append_follow_up",
        )
      })
      expect(onInputChange).toHaveBeenCalledWith("")
    } finally {
      rectSpy.mockRestore()
    }
  })

  test("shows the active goal strip with required criteria progress above the composer", () => {
    renderChatInput({ goalSnapshot: activeGoalSnapshot })

    expect(screen.getByText("chat.goalMode.activeGoal")).toBeTruthy()
    expect(screen.getByText("Complete Goal v2 review")).toBeTruthy()
    expect(screen.getByText("1/2")).toBeTruthy()
  })

  test("previews required optional and follow-up criteria while editing the active goal", () => {
    renderChatInput({
      goalSnapshot: {
        ...activeGoalSnapshot,
        goal: {
          ...activeGoalSnapshot.goal,
          completionCriteria:
            "[required] code is reviewed\n[optional] polish copy\n[follow-up] browser smoke",
        },
      },
    })

    fireEvent.click(screen.getByRole("button", { name: "chat.goalMode.edit" }))

    expect(screen.getByText("chat.goalMode.criteriaPreview")).toBeTruthy()
    expect(screen.getByText("chat.goalMode.criteriaRequiredCount")).toBeTruthy()
    expect(screen.getByText("chat.goalMode.criteriaOptionalCount")).toBeTruthy()
    expect(screen.getByText("chat.goalMode.criteriaFollowUpCount")).toBeTruthy()
    expect(screen.getByText("code is reviewed")).toBeTruthy()
    expect(screen.getByText("polish copy")).toBeTruthy()
    expect(screen.getByText("browser smoke")).toBeTruthy()
  })

  test("stages workflow mode in a draft session without materializing it", async () => {
    const onEnsureSession = vi.fn(() => Promise.resolve("s-created"))
    const onDraftWorkflowModeChange = vi.fn()
    transportMock.call.mockImplementation((command: string, args?: unknown) => {
      if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
      if (command === "set_workflow_mode") {
        return Promise.resolve({ mode: (args as { mode?: string } | undefined)?.mode ?? "off" })
      }
      return Promise.resolve([])
    })
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
      () =>
        ({
          x: 0,
          y: 0,
          width: 1200,
          height: 80,
          top: 0,
          left: 0,
          right: 1200,
          bottom: 80,
          toJSON: () => ({}),
        }) as DOMRect,
    )

    try {
      renderChatInput({ currentSessionId: null, onEnsureSession, onDraftWorkflowModeChange })

      const workflowButton = await screen.findByRole("button", {
        name: "工作流模式",
      })
      fireEvent.pointerDown(workflowButton, { button: 0, ctrlKey: false, pointerType: "mouse" })
      fireEvent.click(workflowButton)
      fireEvent.click((await screen.findByText("自动")).closest("button")!)

      await waitFor(() => {
        expect(onDraftWorkflowModeChange).toHaveBeenCalledWith("on")
      })
      expect(onEnsureSession).not.toHaveBeenCalled()
      expect(transportMock.call).not.toHaveBeenCalledWith(
        "set_workflow_mode",
        expect.anything(),
      )
      expect(await screen.findByText("chat.workflowMode.activeOnDetail")).toBeTruthy()
    } finally {
      rectSpy.mockRestore()
    }
  })

  test("materializes a draft session before executing workflow slash mode command", async () => {
    const onEnsureSession = vi.fn(() => Promise.resolve("s-created"))
    const onCommandAction = vi.fn()
    transportMock.call.mockImplementation((command: string) => {
      if (command === "get_awareness_config") return Promise.resolve({ enabled: false })
      if (command === "list_slash_commands") {
        return Promise.resolve([
          {
            name: "workflow",
            category: "utility",
            descriptionKey: "slashCommands.workflow.description",
            hasArgs: true,
            argsOptional: true,
            argOptions: ["on", "off", "ultracode"],
          },
        ])
      }
      if (command === "execute_slash_command") {
        return Promise.resolve({
          content: "Workflow Mode is now **On** (`on`).",
          action: { type: "setWorkflowMode", mode: "on" },
        })
      }
      return Promise.resolve([])
    })

    renderChatInput({
      input: "/workflow on",
      currentSessionId: null,
      onEnsureSession,
      onCommandAction,
    })

    await waitFor(() => expect(screen.getByText("on")).toBeTruthy())
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" })

    await waitFor(() => {
      expect(onEnsureSession).toHaveBeenCalledTimes(1)
      expect(transportMock.call).toHaveBeenCalledWith("execute_slash_command", {
        sessionId: "s-created",
        agentId: "ha-main",
        commandText: "/workflow on",
      })
      expect(onCommandAction).toHaveBeenCalledWith(
        expect.objectContaining({
          action: { type: "setWorkflowMode", mode: "on" },
          _sessionId: "s-created",
          _slashCommandText: "/workflow on",
        }),
      )
    })
  })

  test("lets file mention menu consume Shift+Tab before permission cycling", async () => {
    const onInputChange = vi.fn()
    const onPermissionModeChange = vi.fn()
    // In the composer a bare `@` shows the note section; a query (`@notes`) drives
    // the file-search section. searchFiles backs that path (list mode is for `/`).
    transportMock.searchFiles.mockResolvedValue({
      root: "/tmp",
      matches: [
        { name: "notes.md", path: "/tmp/notes.md", relPath: "notes.md", isDir: false, score: 1 },
      ],
      truncated: false,
    })

    renderChatInput({
      input: "@notes",
      onInputChange,
      onPermissionModeChange,
      workingDir: "/tmp",
    })

    // Nudge the mention popper open (mirrors a caret move after typing `@notes`).
    fireEvent.select(screen.getByRole("textbox"))

    await waitFor(() => expect(screen.getByText("notes.md")).toBeTruthy())
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Tab", shiftKey: true })

    expect(onPermissionModeChange).not.toHaveBeenCalled()
    expect(onInputChange).toHaveBeenCalledWith("@notes.md ")
  })

  test("cycles recent user input history only from an empty draft", () => {
    const onInputChange = vi.fn()
    const { props, view } = renderChatInput({
      input: "",
      inputHistory: ["second", "first"],
      onInputChange,
    })

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowUp" })
    expect(onInputChange).toHaveBeenLastCalledWith("second")

    view.rerender(
      <TooltipProvider>
        <ChatInput {...props} input="second" />
      </TooltipProvider>,
    )
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowDown" })
    expect(onInputChange).toHaveBeenLastCalledWith("")
  })

  test("resets history browsing after sending a selected history item", () => {
    const onInputChange = vi.fn()
    const onSend = vi.fn()
    const { props, view } = renderChatInput({
      input: "",
      inputHistory: ["second", "first"],
      onInputChange,
      onSend,
    })

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowUp" })
    expect(onInputChange).toHaveBeenLastCalledWith("second")

    view.rerender(
      <TooltipProvider>
        <ChatInput {...props} input="second" />
      </TooltipProvider>,
    )
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" })
    expect(onSend).toHaveBeenCalledTimes(1)

    view.rerender(
      <TooltipProvider>
        <ChatInput {...props} input="" />
      </TooltipProvider>,
    )
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowUp" })
    expect(onInputChange).toHaveBeenLastCalledWith("second")
  })

  test("does not replace a manual draft with input history", () => {
    const onInputChange = vi.fn()
    renderChatInput({
      input: "manual draft",
      inputHistory: ["previous"],
      onInputChange,
    })

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowUp" })

    expect(onInputChange).not.toHaveBeenCalled()
  })

  test("inserts a selected quick prompt from the hash menu", async () => {
    const onInputChange = vi.fn()
    renderChatInput({
      input: "please #sum",
      onInputChange,
      quickPrompts: [
        {
          id: "qp1",
          title: "Summarize",
          content: "summarize this thread",
          createdAt: "2026-06-28T00:00:00Z",
        },
      ],
    })

    fireEvent.select(screen.getByRole("textbox"))

    await waitFor(() => expect(screen.getByText("Summarize")).toBeTruthy())
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" })

    expect(onInputChange).toHaveBeenCalledWith("please summarize this thread")
  })

  test("lets Enter send when the hash menu has no quick prompt matches", async () => {
    const onSend = vi.fn()
    renderChatInput({
      input: "#triage",
      onSend,
      quickPrompts: [
        {
          id: "qp1",
          title: "Summarize",
          content: "summarize this thread",
          createdAt: "2026-06-28T00:00:00Z",
        },
      ],
    })

    fireEvent.select(screen.getByRole("textbox"))

    await waitFor(() => expect(screen.getByText("chat.quickPrompts.noMatches")).toBeTruthy())
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" })

    expect(onSend).toHaveBeenCalledTimes(1)
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
