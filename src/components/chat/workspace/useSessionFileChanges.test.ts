import { describe, it, expect } from "vitest"
import { aggregateSessionFileChanges, messagesHaveFileActivity } from "./useSessionFileChanges"
import type { ContentBlock, FileChangeMetadata, Message, ToolMetadata } from "@/types/chat"

function change(
  path: string,
  action: FileChangeMetadata["action"],
  added = 1,
  removed = 0,
): FileChangeMetadata {
  return {
    kind: "file_change",
    path,
    action,
    linesAdded: added,
    linesRemoved: removed,
    before: action === "create" ? null : "old",
    after: action === "delete" ? null : "new",
    language: "ts",
    truncated: false,
  }
}

function toolMsg(...metas: ToolMetadata[]): Message {
  const blocks: ContentBlock[] = metas.map((metadata, i) => ({
    type: "tool_call",
    tool: { callId: `c${i}`, name: "tool", arguments: "{}", metadata },
  }))
  return { role: "assistant", content: "", contentBlocks: blocks }
}

describe("messagesHaveFileActivity", () => {
  it("does not open an empty workspace for a cross-session message receipt", () => {
    const receipt: ToolMetadata = {
      kind: "session_message",
      sessionId: "target",
      messageId: 42,
      turnId: "turn-1",
    }
    const message = toolMsg(receipt)
    expect(aggregateSessionFileChanges([message])).toEqual([])
    expect(messagesHaveFileActivity([message])).toBe(false)
    expect(
      messagesHaveFileActivity([
        {
          role: "assistant",
          content: "",
          toolCalls: [
            { callId: "send", name: "sessions_send", arguments: "{}", metadata: receipt },
          ],
        },
      ]),
    ).toBe(false)
  })

  it("still recognizes file reads, changes and local media beside a receipt", () => {
    expect(messagesHaveFileActivity([toolMsg(change("a.ts", "edit"))])).toBe(true)
    expect(messagesHaveFileActivity([toolMsg({ kind: "file_read", path: "a.ts", lines: 2 })])).toBe(
      true,
    )
    expect(
      messagesHaveFileActivity([
        toolMsg({ kind: "file_changes", changes: [change("a.ts", "create")] }),
      ]),
    ).toBe(true)
    expect(messagesHaveFileActivity([toolMsg({ kind: "file_changes", changes: [] })])).toBe(false)
    const message = toolMsg({
      kind: "session_message",
      sessionId: "target",
      messageId: 42,
      turnId: "turn-1",
    })
    const block = message.contentBlocks![0]
    if (block.type !== "tool_call") throw new Error("Expected a tool call")
    block.tool.mediaItems = [
      {
        kind: "image",
        name: "result.png",
        mimeType: "image/png",
        sizeBytes: 12,
        url: "https://example.com/result.png",
        localPath: "/tmp/result.png",
      },
    ]
    expect(messagesHaveFileActivity([message])).toBe(true)
  })
})

describe("aggregateSessionFileChanges", () => {
  it("returns a modified entry carrying the diff payload", () => {
    const result = aggregateSessionFileChanges([toolMsg(change("a.ts", "edit", 3, 1))])
    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({ path: "a.ts", kind: "modified", linesAdded: 3, linesRemoved: 1 })
    expect(result[0].diff).not.toBeNull()
    expect(result[0].readLines).toBeNull()
    expect(result[0].language).toBe("ts")
  })

  it("expands a file_changes payload into one entry per file", () => {
    const result = aggregateSessionFileChanges([
      toolMsg({ kind: "file_changes", changes: [change("a.ts", "edit"), change("b.ts", "create")] }),
    ])
    expect(result.map((e) => e.path).sort()).toEqual(["a.ts", "b.ts"])
  })

  it("records a read entry with line count and no diff", () => {
    const result = aggregateSessionFileChanges([toolMsg({ kind: "file_read", path: "r.ts", lines: 42 })])
    expect(result[0]).toMatchObject({ path: "r.ts", kind: "read", readLines: 42 })
    expect(result[0].diff).toBeNull()
  })

  it("does not downgrade a written file to read (read after edit)", () => {
    const result = aggregateSessionFileChanges([
      toolMsg(change("a.ts", "edit")),
      toolMsg({ kind: "file_read", path: "a.ts", lines: 10 }),
    ])
    expect(result).toHaveLength(1)
    expect(result[0].kind).toBe("modified")
    expect(result[0].diff).not.toBeNull()
    expect(result[0].language).toBe("ts")
  })

  it("upgrades a read file to modified (edit after read)", () => {
    const result = aggregateSessionFileChanges([
      toolMsg({ kind: "file_read", path: "a.ts", lines: 10 }),
      toolMsg(change("a.ts", "edit")),
    ])
    expect(result).toHaveLength(1)
    expect(result[0].kind).toBe("modified")
  })

  it("keeps the latest diff when a file is edited multiple times", () => {
    const result = aggregateSessionFileChanges([
      toolMsg(change("a.ts", "edit", 1, 0)),
      toolMsg(change("a.ts", "edit", 9, 9)),
    ])
    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({ linesAdded: 9, linesRemoved: 9 })
  })

  it("orders most-recently-touched first", () => {
    const result = aggregateSessionFileChanges([
      toolMsg(change("a.ts", "edit")),
      toolMsg(change("b.ts", "edit")),
      toolMsg(change("a.ts", "edit")),
    ])
    expect(result.map((e) => e.path)).toEqual(["a.ts", "b.ts"])
  })

  it("falls back to legacy toolCalls when contentBlocks is absent", () => {
    const msg: Message = {
      role: "assistant",
      content: "",
      toolCalls: [{ callId: "c", name: "edit", arguments: "{}", metadata: change("legacy.ts", "edit") }],
    }
    const result = aggregateSessionFileChanges([msg])
    expect(result[0]).toMatchObject({ path: "legacy.ts", kind: "modified" })
  })

  it("recovers files from legacy messages without structured metadata", () => {
    const msg: Message = {
      role: "assistant",
      content: "",
      contentBlocks: [
        {
          type: "tool_call",
          tool: {
            callId: "c",
            name: "write",
            arguments: JSON.stringify({ path: "old.ts" }),
            result: "Successfully wrote old.ts",
          },
        },
      ],
    }
    const result = aggregateSessionFileChanges([msg])
    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({ path: "old.ts", kind: "modified", diff: null })
  })

  it("ignores tool calls with neither metadata nor a recoverable path", () => {
    const msg: Message = {
      role: "assistant",
      content: "",
      contentBlocks: [{ type: "tool_call", tool: { callId: "c", name: "web_search", arguments: "{}" } }],
    }
    expect(aggregateSessionFileChanges([msg])).toEqual([])
  })
})
