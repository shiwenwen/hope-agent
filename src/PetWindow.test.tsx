// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  emit: vi.fn(() => Promise.resolve()),
  listeners: new Map<string, (payload: unknown) => void>(),
  listen: vi.fn((event: string, handler: (payload: unknown) => void) => {
    mocks.listeners.set(event, handler)
    return () => mocks.listeners.delete(event)
  }),
  startChat: vi.fn(() => Promise.resolve("ok")),
  startDragging: vi.fn(() => Promise.resolve()),
  windowMoved: null as null | ((event: { payload: { x: number; y: number } }) => void),
  onMoved: vi.fn(
    (handler: (event: { payload: { x: number; y: number } }) => void) => {
      mocks.windowMoved = handler
      return Promise.resolve(() => {
        if (mocks.windowMoved === handler) mocks.windowMoved = null
      })
    },
  ),
  completeAction: null as null | ((action: string) => void),
  approvals: [] as Array<Record<string, unknown>>,
  layoutOverride: null as null | {
    mode: "none" | "bubble" | "tray" | "menu"
    horizontal: "left" | "right"
    vertical: "above" | "below"
    visible: boolean
  },
  petSnapshot: {
    revision: 0,
    generatedAt: "",
    stale: false,
    dominant: null as null | "needs_input" | "blocked" | "ready" | "running",
    activities: [] as Array<Record<string, unknown>>,
    total: 0,
    truncated: false,
  },
}))

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: mocks.startDragging, onMoved: mocks.onMoved }),
}))

vi.mock("@tauri-apps/api/event", () => ({
  emit: mocks.emit,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    call: mocks.call,
    listen: mocks.listen,
    startChat: mocks.startChat,
  }),
}))

vi.mock("@/lib/logger", () => ({
  logger: { warn: vi.fn() },
}))

vi.mock("@/components/pet/hooks/usePetActivity", () => ({
  usePetActivity: () => ({
    initialized: true,
    snapshot: mocks.petSnapshot,
  }),
}))

vi.mock("@/components/chat/hooks/useApprovals", () => ({
  useApprovals: () => ({
    approvalRequests: mocks.approvals,
    handleApprovalResponse: vi.fn(),
  }),
}))

vi.mock("@/components/pet/hooks/usePetAssetUrl", () => ({
  usePetAssetUrl: () => ({ src: "pet.png", loading: false, failed: false }),
}))

vi.mock("@/components/pet/hooks/usePetAnimator", () => ({
  actionForStatus: () => "idle",
}))

vi.mock("@/components/pet/hooks/usePetWindowLayout", () => ({
  usePetWindowLayout: (mode: "none" | "bubble" | "tray" | "menu") =>
    mocks.layoutOverride ?? {
      mode,
      horizontal: "left",
      vertical: "above",
      visible: true,
    },
}))

vi.mock("@/components/pet/PetSprite", () => ({
  AnimatedPetSprite: ({
    action,
    onActionComplete,
  }: {
    action: string
    onActionComplete?: (action: string) => void
  }) => {
    mocks.completeAction = onActionComplete ?? null
    return (
      <span aria-hidden="true" data-testid="pet-sprite" data-action={action}>
        pet
      </span>
    )
  },
}))

import PetWindow from "./PetWindow"

function petButton(): HTMLButtonElement {
  const button = screen.getByRole("button", {
    name: "Interact with Nori",
  }) as HTMLButtonElement
  Object.defineProperty(button, "setPointerCapture", {
    configurable: true,
    value: vi.fn(),
  })
  return button
}

beforeEach(() => {
  mocks.call.mockReset()
  mocks.emit.mockClear()
  mocks.listen.mockClear()
  mocks.startChat.mockClear()
  mocks.startDragging.mockClear()
  mocks.onMoved.mockClear()
  mocks.windowMoved = null
  mocks.listeners.clear()
  mocks.completeAction = null
  mocks.approvals = []
  mocks.layoutOverride = null
  mocks.petSnapshot = {
    revision: 0,
    generatedAt: "",
    stale: false,
    dominant: null,
    activities: [],
    total: 0,
    truncated: false,
  }
  mocks.call.mockImplementation((command: string) => {
    if (command === "get_pet_config_cmd") {
      return Promise.resolve({ selectedPetRef: "builtin:hope-default" })
    }
    if (command === "pet_list_cmd") {
      return Promise.resolve({ pets: [] })
    }
    if (command === "get_pending_ask_user_group") return Promise.resolve(null)
    return Promise.resolve(undefined)
  })
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(performance.now())
    return 1
  })
  vi.stubGlobal("cancelAnimationFrame", vi.fn())
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

describe("PetWindow pointer interactions", () => {
  test("suppresses the synthetic click after dragging without poisoning the next click", async () => {
    render(<PetWindow />)
    const pet = petButton()

    fireEvent.pointerDown(pet, { button: 0, clientX: 10, clientY: 10, pointerId: 1 })
    await act(async () => {
      fireEvent.pointerMove(pet, { clientX: 30, clientY: 10, pointerId: 1 })
      await Promise.resolve()
    })
    await waitFor(() => expect(mocks.startDragging).toHaveBeenCalledOnce())
    act(() => mocks.listeners.get("pet:native_drag_ended")?.(undefined))

    fireEvent.click(pet)
    expect(mocks.call).not.toHaveBeenCalledWith("pet_focus_target_cmd", expect.anything())

    fireEvent.pointerDown(pet, { button: 0, clientX: 30, clientY: 10, pointerId: 2 })
    fireEvent.click(pet)
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "jump")
    expect(mocks.call).toHaveBeenCalledWith("pet_focus_target_cmd", { target: null })
  })

  test("paints and updates the running direction throughout the native drag", () => {
    vi.useFakeTimers()
    const frames: FrameRequestCallback[] = []
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.push(callback)
      return frames.length
    })
    render(<PetWindow />)
    const pet = petButton()

    fireEvent.pointerDown(pet, { button: 0, clientX: 30, clientY: 10, pointerId: 1 })
    fireEvent.pointerMove(pet, { clientX: 10, clientY: 10, pointerId: 1 })

    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "run_left")
    expect(mocks.startDragging).not.toHaveBeenCalled()

    act(() => frames.shift()?.(performance.now()))
    expect(mocks.startDragging).not.toHaveBeenCalled()

    act(() => vi.advanceTimersByTime(33))
    expect(mocks.startDragging).not.toHaveBeenCalled()

    act(() => vi.advanceTimersByTime(1))
    expect(mocks.startDragging).toHaveBeenCalledOnce()
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "run_left")

    act(() => mocks.windowMoved?.({ payload: { x: 100, y: 20 } }))
    act(() => mocks.windowMoved?.({ payload: { x: 90, y: 20 } }))
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "run_left")

    act(() => mocks.windowMoved?.({ payload: { x: 110, y: 20 } }))
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "run_right")

    act(() => mocks.listeners.get("pet:native_drag_ended")?.(undefined))
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "idle")
  })

  test("plays the hover wave action and restores the business animation after completion", () => {
    render(<PetWindow />)
    const pet = petButton()

    fireEvent.pointerEnter(pet)
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "wave")

    act(() => mocks.completeAction?.("wave"))
    expect(screen.getByTestId("pet-sprite")).toHaveAttribute("data-action", "idle")
  })

  test("switches the stack control between a collapse chevron and conversation count", async () => {
    mocks.petSnapshot = {
      revision: 2,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "running",
      activities: [
        {
          activityId: "session-1",
          status: "running",
          title: "First task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "regular", sessionId: "session-1" },
        },
        {
          activityId: "session-2",
          status: "ready",
          title: "Second task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:01Z",
          target: { kind: "regular", sessionId: "session-2" },
        },
      ],
      total: 2,
      truncated: false,
    }
    const { rerender } = render(<PetWindow />)

    const collapse = await screen.findByRole("button", { name: "Collapse conversations" })
    expect(collapse).toHaveClass("right-1", "size-7", "rounded-full", "p-0")
    fireEvent.click(collapse)
    expect(screen.getByRole("button", { name: "Show 2 conversations" })).toHaveTextContent("2")

    mocks.petSnapshot = {
      ...mocks.petSnapshot,
      revision: 3,
      activities: [
        ...mocks.petSnapshot.activities,
        {
          activityId: "session-3",
          status: "running",
          title: "Third task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:02Z",
          target: { kind: "regular", sessionId: "session-3" },
        },
      ],
      total: 3,
    }
    rerender(<PetWindow />)

    expect(
      await screen.findByRole("button", { name: "Collapse conversations" }),
    ).toBeInTheDocument()
    expect(screen.getAllByText("Third task").length).toBeGreaterThan(0)
  })

  test("keeps the committed bubbles mounted while the close transition fades them out", async () => {
    mocks.petSnapshot = {
      revision: 9,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "running",
      activities: [
        {
          activityId: "session-fade",
          status: "running",
          title: "Fade this bubble",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "regular", sessionId: "session-fade" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    const collapse = await screen.findByRole("button", { name: "Collapse conversations" })
    mocks.layoutOverride = {
      mode: "bubble",
      horizontal: "left",
      vertical: "above",
      visible: false,
    }
    fireEvent.click(collapse)

    const bubbleTitle = screen.getByText("Fade this bubble")
    expect(bubbleTitle).toBeInTheDocument()
    let fadingOverlay: HTMLElement | null = bubbleTitle.parentElement
    while (fadingOverlay && !fadingOverlay.classList.contains("opacity-0")) {
      fadingOverlay = fadingOverlay.parentElement
    }
    expect(fadingOverlay).toHaveClass("opacity-0", "duration-[180ms]")
  })

  test("dismisses a completed bubble optimistically and advances its read boundary", async () => {
    mocks.petSnapshot = {
      revision: 6,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "ready",
      activities: [
        {
          activityId: "session-ready",
          status: "ready",
          title: "Completed task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          boundary: 42,
          preview: "Finished",
          target: { kind: "regular", sessionId: "session-ready" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click((await screen.findAllByRole("button", { name: "Dismiss" }))[0])
    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("mark_session_read_cmd", {
        sessionId: "session-ready",
        throughMessageId: 42,
      })
    })
    expect(screen.queryByText("Completed task")).not.toBeInTheDocument()
  })

  test("defers the read receipt to the destination after opening a completed bubble", async () => {
    mocks.petSnapshot = {
      revision: 7,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "ready",
      activities: [
        {
          activityId: "session-open",
          status: "ready",
          title: "Open completed task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          boundary: 42,
          preview: "Finished",
          target: { kind: "regular", sessionId: "session-open" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    const openButton = (await screen.findAllByText("Open completed task"))
      .map((title) => title.closest("button"))
      .find(
        (button): button is HTMLButtonElement =>
          button instanceof HTMLButtonElement && !button.disabled,
      )
    expect(openButton).toBeDefined()
    fireEvent.click(openButton!)

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("pet_focus_target_cmd", {
        target: { kind: "regular", sessionId: "session-open" },
      })
    })
    expect(mocks.call).not.toHaveBeenCalledWith("mark_session_read_cmd", expect.anything())
  })

  test("marks only exposed terminal boundaries read when the user collapses the stack", async () => {
    mocks.petSnapshot = {
      revision: 8,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "ready",
      activities: [
        {
          activityId: "session-ready",
          status: "ready",
          title: "Completed task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          boundary: 42,
          preview: "Finished",
          target: { kind: "regular", sessionId: "session-ready" },
        },
        {
          activityId: "session-running",
          status: "running",
          title: "Running task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:01Z",
          boundary: 43,
          target: { kind: "regular", sessionId: "session-running" },
        },
      ],
      total: 2,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click(await screen.findByRole("button", { name: "Collapse conversations" }))
    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("mark_session_read_cmd", {
        sessionId: "session-ready",
        throughMessageId: 42,
      })
    })
    expect(mocks.call).not.toHaveBeenCalledWith("mark_session_read_cmd", {
      sessionId: "session-running",
      throughMessageId: 43,
    })
  })

  test("hides a running bubble without cancelling or marking its turn read", async () => {
    mocks.petSnapshot = {
      revision: 7,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "running",
      activities: [
        {
          activityId: "session-running",
          status: "running",
          title: "Running task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          boundary: 43,
          target: { kind: "regular", sessionId: "session-running" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click((await screen.findAllByRole("button", { name: "Dismiss" }))[0])
    await waitFor(() => expect(screen.queryByText("Running task")).not.toBeInTheDocument())
    expect(mocks.call).not.toHaveBeenCalledWith("mark_session_read_cmd", expect.anything())
  })

  test("collapses approval cards into an amber badge and reopens for a new approval", async () => {
    mocks.petSnapshot = {
      revision: 3,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "needs_input",
      activities: [
        {
          activityId: "session-approval",
          status: "needs_input",
          title: "Needs permission",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "regular", sessionId: "session-approval" },
        },
      ],
      total: 1,
      truncated: false,
    }
    mocks.approvals = [
      {
        request_id: "approval-1",
        session_id: "session-approval",
        command: "pnpm typecheck",
        cwd: "/workspace",
      },
    ]
    const { rerender } = render(<PetWindow />)

    fireEvent.click(await screen.findByRole("button", { name: "Collapse conversations" }))
    expect(screen.queryByRole("heading", { name: "Approval needed" })).not.toBeInTheDocument()
    expect(screen.queryByText("Needs permission")).not.toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /Show 1 conversations.*need your input/i }),
    ).toHaveClass("bg-amber-400/90")

    mocks.approvals = [
      ...mocks.approvals,
      {
        request_id: "approval-2",
        session_id: "session-approval",
        command: "pnpm test",
        cwd: "/workspace",
      },
    ]
    rerender(<PetWindow />)

    expect(
      await screen.findByRole("button", { name: "Collapse conversations" }),
    ).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Approval needed" })).toBeInTheDocument()
  })

  test("queues replies onto a running turn instead of starting another turn", async () => {
    mocks.petSnapshot = {
      revision: 4,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "running",
      activities: [
        {
          activityId: "session-running",
          status: "running",
          title: "Running task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "regular", sessionId: "session-running" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click(await screen.findByRole("button", { name: "Reply" }))
    const composer = screen.getAllByPlaceholderText("Ask anything…")[0]
    fireEvent.change(composer, { target: { value: "Use the paw icon" } })
    fireEvent.keyDown(composer, { key: "Enter" })

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith(
        "queue_turn_user_message",
        expect.objectContaining({
          sessionId: "session-running",
          message: "Use the paw icon",
          attachments: [],
        }),
      )
    })
    expect(mocks.startChat).not.toHaveBeenCalled()
  })

  test("stops only the running conversation selected from its hover actions", async () => {
    mocks.petSnapshot = {
      revision: 8,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "running",
      activities: [
        {
          activityId: "session-stop",
          status: "running",
          title: "Running task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "regular", sessionId: "session-stop" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click(await screen.findByRole("button", { name: "Stop reply" }))

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("stop_chat", {
        sessionId: "session-stop",
        turnId: null,
      })
    })
  })

  test("continues a completed conversation through the Pet chat surface", async () => {
    mocks.petSnapshot = {
      revision: 5,
      generatedAt: "2026-07-24T00:00:00Z",
      stale: false,
      dominant: "ready",
      activities: [
        {
          activityId: "session-ready",
          status: "ready",
          title: "Ready task",
          titleKind: "session",
          updatedAt: "2026-07-24T00:00:00Z",
          target: { kind: "design", sessionId: "session-ready", projectId: "design-1" },
        },
      ],
      total: 1,
      truncated: false,
    }
    render(<PetWindow />)

    fireEvent.click(await screen.findByRole("button", { name: "Reply" }))
    const composer = screen.getAllByPlaceholderText("Ask anything…")[0]
    fireEvent.change(composer, { target: { value: "Make the capsule smaller" } })
    fireEvent.keyDown(composer, { key: "Enter" })

    await waitFor(() => {
      expect(mocks.startChat).toHaveBeenCalledWith(
        {
          message: "Make the capsule smaller",
          attachments: [],
          sessionId: "session-ready",
          uiSurface: "pet_chat",
          toolScope: "design",
        },
        expect.any(Function),
      )
    })
    expect(mocks.call).not.toHaveBeenCalledWith("queue_turn_user_message", expect.anything())
  })

  test("offers settings and close from the pet context menu", async () => {
    render(<PetWindow />)
    fireEvent.contextMenu(petButton())

    const settings = screen.getByRole("menuitem", { name: "Settings" })
    const close = screen.getByRole("menuitem", { name: "Close" })
    expect(settings).toHaveClass("h-6", "min-w-0", "px-2", "text-[11px]")
    expect(close).toHaveClass("h-6", "min-w-0", "px-2", "text-[11px]")
    expect(close.parentElement).toHaveClass("left-1/2", "top-1/2")

    fireEvent.click(settings)
    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("pet_focus_target_cmd", { target: null })
      expect(mocks.emit).toHaveBeenCalledWith("open-settings", { section: "pets" })
    })

    fireEvent.contextMenu(petButton())
    fireEvent.click(screen.getByRole("menuitem", { name: "Close" }))
    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("pet_set_enabled_cmd", {
        enabled: false,
        source: "pet-window",
      })
    })
  })
})
