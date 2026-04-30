import { describe, expect, test } from "vitest"
import type { Message, Task } from "@/types/chat"
import {
  createTaskProgressSnapshot,
  extractLatestTaskProgressSnapshot,
  getTaskDisplayLabel,
  parseTaskToolResult,
  selectCurrentTaskBatch,
  shouldShowTaskProgressPanel,
} from "./taskProgress"

function task(patch: Partial<Task>): Task {
  return {
    id: 1,
    sessionId: "s1",
    content: "Run checks",
    activeForm: null,
    status: "pending",
    batchId: null,
    createdAt: "2026-04-29T00:00:00.000Z",
    updatedAt: "2026-04-29T00:00:00.000Z",
    ...patch,
  }
}

function assistantWithTaskResult(result: string, callId = "call-1"): Message {
  return {
    role: "assistant",
    content: "",
    contentBlocks: [
      {
        type: "tool_call",
        tool: {
          callId,
          name: "task_update",
          arguments: "{}",
          result,
        },
      },
    ],
  }
}

describe("task progress parsing", () => {
  test("extracts the latest complete task snapshot from task tool results", () => {
    const first = [
      task({ id: 1, status: "pending", updatedAt: "2026-04-29T00:00:00.000Z" }),
      task({ id: 2, content: "Update UI", status: "pending", updatedAt: "2026-04-29T00:00:00.000Z" }),
    ]
    const latest = [
      task({ id: 1, status: "completed", updatedAt: "2026-04-29T00:02:00.000Z" }),
      task({
        id: 2,
        content: "Update UI",
        activeForm: "Updating UI",
        status: "in_progress",
        updatedAt: "2026-04-29T00:03:00.000Z",
      }),
    ]

    const snapshot = extractLatestTaskProgressSnapshot([
      assistantWithTaskResult(JSON.stringify(first), "call-1"),
      assistantWithTaskResult(JSON.stringify(latest), "call-2"),
    ])

    expect(snapshot?.tasks).toEqual(latest)
    expect(snapshot?.completed).toBe(1)
    expect(snapshot?.remaining).toBe(1)
    expect(snapshot?.inProgress).toBe(true)
  })

  test("falls back to legacy toolCalls and ignores malformed results", () => {
    const latest = [task({ id: 3, content: "Ship", status: "completed" })]
    const message: Message = {
      role: "assistant",
      content: "",
      toolCalls: [
        {
          callId: "bad",
          name: "task_update",
          arguments: "{}",
          result: "not json",
        },
        {
          callId: "good",
          name: "task_list",
          arguments: "{}",
          result: JSON.stringify(latest),
        },
      ],
    }

    expect(parseTaskToolResult("not json")).toEqual([])
    expect(extractLatestTaskProgressSnapshot([message])?.tasks).toEqual(latest)
  })

  test("summarizes counts and uses activeForm for in-progress labels", () => {
    const tasks = [
      task({ id: 1, content: "Write code", status: "completed" }),
      task({ id: 2, content: "Run tests", activeForm: "Running tests", status: "in_progress" }),
      task({ id: 3, content: "Review", status: "pending" }),
    ]

    const snapshot = createTaskProgressSnapshot(tasks)

    expect(snapshot).toMatchObject({
      total: 3,
      completed: 1,
      remaining: 2,
      inProgress: true,
    })
    expect(getTaskDisplayLabel(tasks[1], "Untitled")).toBe("Running tests")
  })

  test("hides the input task panel once every task is completed", () => {
    const snapshot = createTaskProgressSnapshot([
      task({ id: 1, content: "Write code", status: "completed" }),
      task({ id: 2, content: "Review", status: "completed" }),
    ])

    expect(shouldShowTaskProgressPanel(snapshot)).toBe(false)
  })

  test("selects the latest task creation batch for current progress", () => {
    const tasks = [
      task({ id: 6, content: "Old structure", status: "completed", createdAt: "2026-04-29T02:58:13.483Z" }),
      task({ id: 7, content: "Old UI", status: "completed", createdAt: "2026-04-29T02:58:13.484Z" }),
      task({ id: 8, content: "Old validation", status: "in_progress", createdAt: "2026-04-29T02:58:13.485Z" }),
      task({ id: 11, content: "New structure", status: "completed", createdAt: "2026-04-29T03:12:38.429Z" }),
      task({ id: 12, content: "New UI", status: "completed", createdAt: "2026-04-29T03:12:38.433Z" }),
      task({ id: 13, content: "New validation", status: "in_progress", createdAt: "2026-04-29T03:12:38.439Z" }),
    ]

    expect(selectCurrentTaskBatch(tasks).map((item) => item.id)).toEqual([11, 12, 13])
  })
})
