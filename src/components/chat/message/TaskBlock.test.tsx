// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import type { Task, ToolCall } from "@/types/chat"
import TaskBlock from "./TaskBlock"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { completed?: number; total?: number; defaultValue?: string }) => {
      if (key === "executionStatus.task.running") return `Running ${options?.completed}/${options?.total}`
      if (key === "executionStatus.task.pending") return `Waiting ${options?.completed}/${options?.total}`
      if (key === "executionStatus.task.completed") return `Completed ${options?.completed}/${options?.total}`
      if (key === "executionStatus.task.empty") return "No tasks"
      if (key === "chat.taskProgressRunning") return `Running ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressCancelling") return `Stopping ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressFailed") return `Failed ${options?.completed}/${options?.total}`
      if (key === "chat.taskProgressWaiting") return `Waiting ${options?.completed}/${options?.total}`
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
    content: "Run tests",
    activeForm: "Running tests",
    status: "in_progress",
    createdAt: "2026-04-29T00:00:00.000Z",
    updatedAt: "2026-04-29T00:00:00.000Z",
    ...patch,
  }
}

function toolWithTasks(tasks: Task[]): ToolCall {
  return {
    callId: "task-call-1",
    name: "task_update",
    arguments: "{}",
    result: JSON.stringify({ tasks }),
  }
}

describe("TaskBlock", () => {
  test("starts collapsed when every task is completed", () => {
    render(
      <TaskBlock
        tool={toolWithTasks([
          task({ id: 1, content: "Write code", status: "completed" }),
          task({ id: 2, content: "Run tests", status: "completed" }),
        ])}
      />,
    )

    const toggle = screen.getByRole("button", { name: /Completed 2\/2/ })
    expect(toggle.getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByText("Write code")).toBeNull()

    fireEvent.click(toggle)
    expect(toggle.getAttribute("aria-expanded")).toBe("true")
    expect(screen.getByText("Write code")).toBeTruthy()
  })

  test("auto-collapses when the task list becomes complete", async () => {
    const { rerender } = render(
      <TaskBlock
        tool={toolWithTasks([
          task({ id: 1, content: "Write code", status: "completed" }),
          task({ id: 2, content: "Run tests", status: "pending" }),
        ])}
      />,
    )

    expect(screen.getByRole("button", { name: /Waiting 1\/2/ }).getAttribute("aria-expanded")).toBe(
      "true",
    )
    expect(screen.getByText("Run tests")).toBeTruthy()

    rerender(
      <TaskBlock
        tool={toolWithTasks([
          task({ id: 1, content: "Write code", status: "completed" }),
          task({ id: 2, content: "Run tests", status: "completed" }),
        ])}
      />,
    )

    const toggle = await screen.findByRole("button", { name: /Completed 2\/2/ })
    await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("false"))
  })

  test("does not spin an in-progress task when execution is no longer running", () => {
    const { container } = render(
      <TaskBlock tool={toolWithTasks([task({})])} executionState="idle" />,
    )

    expect(screen.getByText("Waiting 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeNull()
    expect(container.querySelector(".lucide-circle-pause")).toBeTruthy()
  })

  test("keeps the spinner while execution is running", () => {
    const { container } = render(
      <TaskBlock tool={toolWithTasks([task({})])} executionState="running" />,
    )

    expect(screen.getByText("Running 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeTruthy()
  })

  test("uses the failed icon and stops spinning when execution failed", () => {
    const { container } = render(
      <TaskBlock tool={toolWithTasks([task({})])} executionState="failed" />,
    )

    expect(screen.getByText("Failed 0/1")).toBeTruthy()
    expect(container.querySelector(".animate-spin")).toBeNull()
    expect(container.querySelector(".lucide-circle-alert")).toBeTruthy()
  })
})
