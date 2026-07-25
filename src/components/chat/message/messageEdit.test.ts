import { describe, expect, it } from "vitest"
import type { Message } from "@/types/chat"
import { assistantTurnHasFileMutations, editableLastUserMessageIndex } from "./messageEdit"

describe("editableLastUserMessageIndex", () => {
  const settled: Message[] = [
    { role: "user", content: "first", dbId: 1 },
    { role: "assistant", content: "first answer", dbId: 2 },
    { role: "user", content: "latest", dbId: 3 },
    { role: "assistant", content: "latest answer", dbId: 4 },
  ]

  it("selects only the latest human prompt after a settled assistant", () => {
    expect(editableLastUserMessageIndex(settled, false, "completed", false)).toBe(2)
  })

  it("hides edit while running or while newer rows may be outside the window", () => {
    expect(editableLastUserMessageIndex(settled, true, "running", false)).toBeNull()
    expect(editableLastUserMessageIndex(settled, false, "completed", true)).toBeNull()
  })

  it("rejects an internal user-shaped tail", () => {
    const messages: Message[] = [
      ...settled,
      { role: "user", content: "subagent result", dbId: 5, isSubagentResult: true },
      { role: "assistant", content: "follow-up", dbId: 6 },
    ]
    expect(editableLastUserMessageIndex(messages, false, "completed", false)).toBeNull()
  })

  it("allows a persisted terminal failure without an assistant text row", () => {
    const messages: Message[] = [
      { role: "user", content: "try this", dbId: 1 },
      { role: "event", content: "request failed", dbId: 2, isTurnError: true },
    ]
    expect(editableLastUserMessageIndex(messages, false, null, false)).toBe(0)
  })

  it("hides queued and slash-command messages that cannot be safely replayed", () => {
    const queued: Message[] = [
      { role: "user", content: "queued", dbId: 1, isQueuedMessage: true },
      { role: "assistant", content: "done", dbId: 2 },
    ]
    const slash: Message[] = [
      { role: "user", content: "/skill args", dbId: 3 },
      { role: "assistant", content: "done", dbId: 4 },
    ]
    expect(editableLastUserMessageIndex(queued, false, "completed", false)).toBeNull()
    expect(editableLastUserMessageIndex(slash, false, "completed", false)).toBeNull()
  })
})

describe("assistantTurnHasFileMutations", () => {
  it("detects structured file changes, including deletion", () => {
    const messages: Message[] = [
      { role: "user", content: "change it", dbId: 1 },
      {
        role: "assistant",
        content: "done",
        dbId: 2,
        contentBlocks: [
          {
            type: "tool_call",
            tool: {
              callId: "call-1",
              name: "apply_patch",
              arguments: "{}",
              result: "Patch applied",
              metadata: {
                kind: "file_change",
                action: "delete",
                path: "old.ts",
                linesAdded: 0,
                linesRemoved: 1,
                before: "old",
                after: null,
                language: "typescript",
                truncated: false,
              },
            },
          },
        ],
      },
    ]
    expect(assistantTurnHasFileMutations(messages, 0)).toBe(true)
  })
})
