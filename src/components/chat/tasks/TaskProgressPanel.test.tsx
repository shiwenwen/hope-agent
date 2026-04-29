// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import type { Task } from "@/types/chat"
import { createTaskProgressSnapshot } from "./taskProgress"
import TaskProgressPanel from "./TaskProgressPanel"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { completed?: number; total?: number; defaultValue?: string }) => {
      if (key === "chat.tasks") return "Tasks"
      if (key === "chat.taskProgress") return `${options?.completed}/${options?.total} completed`
      return options?.defaultValue ?? key
    },
  }),
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
    expect(screen.getByText("1/2 completed")).toBeTruthy()
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
})
