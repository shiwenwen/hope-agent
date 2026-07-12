import { describe, expect, test } from "vitest"
import { explainMemorySearchMatch } from "./memorySearchExplain"
import type { MemoryEntry } from "./types"

function memory(patch: Partial<MemoryEntry>): MemoryEntry {
  return {
    id: 1,
    memoryType: "user",
    scope: { kind: "global" },
    content: "用户偏好：请默认使用中文回复。",
    tags: ["profile", "中文"],
    source: "user",
    sourceSessionId: "session-abc",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    relevanceScore: null,
    ...patch,
  }
}

describe("explainMemorySearchMatch", () => {
  test("detects CJK content and tag literal matches", () => {
    expect(explainMemorySearchMatch(memory({}), "中文").map((item) => item.kind)).toEqual([
      "content",
      "tag",
    ])
  })

  test("detects source/session matches", () => {
    expect(
      explainMemorySearchMatch(
        memory({ source: "import", sourceSessionId: "chat-42" }),
        "chat",
      ).map((item) => item.kind),
    ).toEqual(["session"])
    expect(
      explainMemorySearchMatch(
        memory({ source: "auto-claim", sourceSessionId: null }),
        "claim",
      ).map((item) => item.kind),
    ).toEqual(["source"])
  })

  test("falls back to ranked explanation for semantic or expanded matches", () => {
    expect(
      explainMemorySearchMatch(
        memory({ content: "No literal overlap", tags: [], relevanceScore: 0.42 }),
        "unseen",
      ),
    ).toEqual([{ kind: "ranked" }])
  })
})
