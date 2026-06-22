// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { Task } from "@/types/chat"
import { createTaskProgressSnapshot } from "./taskProgress"
import TaskProgressPanel from "./TaskProgressPanel"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { completed?: number; total?: number; defaultValue?: string }) => {
      if (key === "chat.tasks") return "Tasks"
      if (key === "chat.taskProgress") return `${options?.completed}/${options?.total} completed`
      if (key === "chat.taskProgressRunning") return `Running ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressCancelling") return `Stopping ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressFailed") return `Failed ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressWaiting") return `Waiting ${options?.completed}/${options?.total}`
      return options?.defaultValue ?? key
    },
  }),
}))

vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
  Tooltip: ({ children }: { children: ReactNode }) => children,
  TooltipTrigger: ({ children }: { children: ReactNode }) => children,
  TooltipContent: ({ children }: { children: ReactNode }) => children,
  TooltipProvider: ({ children }: { children: ReactNode }) => children,
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: vi.fn().mockResolvedValue(undefined) }),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function task(patch: Partial<Task>): Task {
  return {
    id: 1,
    sessionId: "s1",
    content: "Write code",
    activeForm: null,
    status: "pending",
    createdAt: "2026-04-29T00:00:00.000Z",
    updatedAt: "2026-04-29T00:00:00.000Z",
    ...patch,
  }
}

describe("TaskProgressPanel", () => {
  test("starts collapsed when every task is completed", () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code", status: "completed" }),
      task({ id: 2, content: "Run tests", status: "completed" }),
    ])

    render(<TaskProgressPanel snapshot={snapshot} />)

    const toggle = screen.getByRole("button", { name: /Tasks/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByText("Run tests")).toBeNull()

    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Run tests")).toBeTruthy()
  })

  test("auto-collapses when every task becomes completed", async () => {
    const pendingSnapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code", status: "completed" }),
      task({ id: 2, content: "Run tests", status: "pending" }),
    ])
    const completedSnapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code", status: "completed" }),
      task({ id: 2, content: "Run tests", status: "completed" }),
    ])

    const { rerender } = render(<TaskProgressPanel snapshot={pendingSnapshot} />)

    const toggle = screen.getByRole("button", { name: /Tasks/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Run tests")).toBeTruthy()

    rerender(<TaskProgressPanel snapshot={completedSnapshot} />)

    await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("false"))
  })

  test("renders summary and expands the task list", () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code", status: "completed" }),
      task({
        id: 2,
        content: "Run tests",
        activeForm: "Running tests",
        status: "in_progress",
      }),
    ])

    render(<TaskProgressPanel snapshot={snapshot} defaultExpanded={false} />)

    const toggle = screen.getByRole("button", { name: /Tasks/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("false")
    expect(screen.getByText("Tasks")).toBeTruthy()
    expect(screen.getByText("Waiting 1/2")).toBeTruthy()
    expect(screen.queryByText("Running tests")).toBeNull()

    fireEvent.click(toggle)

    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Running tests")).toBeTruthy()
    expect(screen.getByText("Write code").classList.contains("line-through")).toBe(true)
  })

  test("uses ordinal numbering instead of database ids", () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 42, content: "First task" }),
      task({ id: 99, content: "Second task" }),
    ])

    render(<TaskProgressPanel snapshot={snapshot} />)

    expect(screen.getByText("1.")).toBeTruthy()
    expect(screen.getByText("2.")).toBeTruthy()
    expect(screen.queryByText("#42")).toBeNull()
  })

  test("auto-collapses when workspace opens", async () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code" }),
      task({ id: 2, content: "Run tests" }),
    ])

    const { rerender } = render(
      <TaskProgressPanel snapshot={snapshot} onOpenWorkspace={vi.fn()} workspaceOpen={false} />,
    )

    const toggle = screen.getByRole("button", { name: /Tasks/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Run tests")).toBeTruthy()

    rerender(
      <TaskProgressPanel snapshot={snapshot} onOpenWorkspace={vi.fn()} workspaceOpen />,
    )

    expect(toggle.getAttribute("aria-expanded")).toBe("false")
    await waitFor(() => expect(screen.queryByText("Run tests")).toBeNull())

    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Run tests")).toBeTruthy()
  })

  test("starts collapsed when mounted with the workspace already open", () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code" }),
      task({ id: 2, content: "Run tests" }),
    ])

    render(<TaskProgressPanel snapshot={snapshot} onOpenWorkspace={vi.fn()} workspaceOpen />)

    const toggle = screen.getByRole("button", { name: /Tasks/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByText("Run tests")).toBeNull()

    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Run tests")).toBeTruthy()
  })

  test("does not spin an in-progress task when execution is no longer running", () => {
    const snapshot = createTaskProgressSnapshot([
      task({
        id: 2,
        content: "Run tests",
        activeForm: "Running tests",
        status: "in_progress",
      }),
    ])

    const { container } = render(<TaskProgressPanel snapshot={snapshot} executionState="idle" />)

    expect(screen.getByText("Waiting 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeNull()
  })

  test("renders stopping state without a spinner", () => {
    const snapshot = createTaskProgressSnapshot([
      task({
        id: 2,
        content: "Run tests",
        activeForm: "Running tests",
        status: "in_progress",
      }),
    ])

    const { container } = render(
      <TaskProgressPanel snapshot={snapshot} executionState="cancelling" />,
    )

    expect(screen.getByText("Stopping 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeNull()
  })

  test("renders failed execution with alert icon and no spinner", () => {
    const snapshot = createTaskProgressSnapshot([
      task({
        id: 2,
        content: "Run tests",
        activeForm: "Running tests",
        status: "in_progress",
      }),
    ])

    const { container } = render(<TaskProgressPanel snapshot={snapshot} executionState="failed" />)

    expect(screen.getByText("Failed 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeNull()
    expect(container.querySelector(".text-destructive")).toBeTruthy()
    expect(container.querySelector(".lucide-circle-alert")).toBeTruthy()
    expect(container.querySelector(".lucide-circle-pause")).toBeNull()
  })
})
