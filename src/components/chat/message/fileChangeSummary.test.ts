import { describe, expect, test } from "vitest"

import type { ToolCall } from "@/types/chat"

import { getFileChangeSummary } from "./fileChangeSummary"

function tool(overrides: Partial<ToolCall>): ToolCall {
  return {
    callId: "call-1",
    name: "edit",
    arguments: "{}",
    ...overrides,
  }
}

describe("getFileChangeSummary", () => {
  test("uses argument estimates while an edit is still running", () => {
    const summary = getFileChangeSummary(
      tool({
        arguments: JSON.stringify({
          old_text: "first\nsecond\n",
          new_text: "first\nsecond\nthird\n",
        }),
      }),
    )

    expect(summary).toMatchObject({
      linesAdded: 1,
      linesRemoved: 0,
      estimated: true,
    })
  })

  test("does not estimate deltas for failed completed edits", () => {
    const summary = getFileChangeSummary(
      tool({
        arguments: JSON.stringify({
          old_text: "first\nsecond\n",
          new_text: "first\nsecond\nthird\n",
        }),
        result: "Tool error: old_text not found",
        isError: true,
      }),
    )

    expect(summary).toBeNull()
  })

  test("does not estimate deltas for failed completed patches", () => {
    const summary = getFileChangeSummary(
      tool({
        name: "apply_patch",
        arguments: JSON.stringify({
          input: [
            "*** Begin Patch",
            "*** Update File: src/example.ts",
            "@@",
            "-old",
            "+new",
            "*** End Patch",
          ].join("\n"),
        }),
        result: "Tool error: patch failed",
        isError: true,
      }),
    )

    expect(summary).toBeNull()
  })

  test("prefers real metadata after a completed edit", () => {
    const summary = getFileChangeSummary(
      tool({
        arguments: JSON.stringify({
          old_text: "one",
          new_text: "one\ntwo",
        }),
        result: "ok",
        metadata: {
          kind: "file_change",
          path: "src/example.ts",
          action: "edit",
          linesAdded: 1,
          linesRemoved: 0,
          before: "one\n",
          after: "one\ntwo\n",
          language: "typescript",
          truncated: false,
        },
      }),
    )

    expect(summary).toMatchObject({
      linesAdded: 1,
      linesRemoved: 0,
      estimated: false,
    })
  })
})
