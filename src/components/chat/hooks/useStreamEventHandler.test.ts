import { describe, expect, test } from "vitest"
import type { Message } from "@/types/chat"
import { createStreamDeltaBuffers, handleStreamEvent } from "./useStreamEventHandler"

function createDeps(messagesRef: { current: Message[] }) {
  return {
    deltaBuffersRef: { current: createStreamDeltaBuffers() },
    updateSessionMessages: (_sessionId: string, updater: (prev: Message[]) => Message[]) => {
      messagesRef.current = updater(messagesRef.current)
    },
  }
}

function parseEvent(message: Message): Record<string, unknown> {
  expect(message.role).toBe("event")
  return JSON.parse(message.content) as Record<string, unknown>
}

describe("handleStreamEvent context compaction notices", () => {
  test("shows Tier 3 summarization progress before the assistant placeholder", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }

    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 3,
          description: "summarizing",
          messages_to_summarize: 8,
        },
      },
      "s1",
      createDeps(messagesRef),
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    expect((event.data as Record<string, unknown>).description).toBe("summarizing")
    expect((event.data as Record<string, unknown>).messages_to_summarize).toBe(8)
  })

  test("replaces live summarization progress with the final compaction notice", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }
    const deps = createDeps(messagesRef)

    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 3,
          description: "summarizing",
          messages_to_summarize: 8,
        },
      },
      "s1",
      deps,
    )
    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 3,
          description: "summarization_needed",
          tokens_before: 1000,
          tokens_after: 420,
          messages_affected: 0,
        },
      },
      "s1",
      deps,
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    const data = event.data as Record<string, unknown>
    expect(data.description).toBe("summarization_needed")
    expect(data.messages_to_summarize).toBe(8)
    expect(data.tokens_after).toBe(420)
  })

  test("replaces progress phase notice with the final compaction notice", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }
    const deps = createDeps(messagesRef)

    handleStreamEvent(
      {
        type: "context_compaction_progress",
        data: {
          phase: "summarizing",
          kind: "summary",
          messages_to_summarize: 9,
        },
      },
      "s1",
      deps,
    )
    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 3,
          description: "summarization_needed",
          tokens_before: 1000,
          tokens_after: 420,
          messages_affected: 0,
        },
      },
      "s1",
      deps,
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    const data = event.data as Record<string, unknown>
    expect(event.type).toBe("context_compacted")
    expect(data.description).toBe("summarization_needed")
    expect(data.messages_to_summarize).toBe(9)
    expect(data.phase).toBeUndefined()
    expect(data.kind).toBeUndefined()
  })

  test("shows emergency compaction progress and replaces it with the final notice", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }
    const deps = createDeps(messagesRef)

    handleStreamEvent(
      {
        type: "context_compaction_progress",
        data: {
          phase: "preparing",
          kind: "emergency",
          attempt: 1,
          max_attempts: 1,
        },
      },
      "s1",
      deps,
    )
    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 4,
          description: "emergency_compact",
          tokens_before: 1200,
          tokens_after: 360,
          messages_affected: 6,
        },
      },
      "s1",
      deps,
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    const data = event.data as Record<string, unknown>
    expect(data.description).toBe("emergency_compact")
    expect(data.messages_affected).toBe(6)
    expect(data.tokens_after).toBe(360)
  })

  test("continues to suppress Tier 0/1 compaction noise", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }

    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 1,
          description: "tool_results_truncated",
        },
      },
      "s1",
      createDeps(messagesRef),
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "assistant"])
  })

  test("coalesces repeated final compaction notices before the active assistant", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }
    const deps = createDeps(messagesRef)

    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 3,
          description: "summarized",
          messages_affected: 17,
        },
      },
      "s1",
      deps,
    )
    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 2,
          description: "summarization_not_applied_sync_compaction_only",
          messages_affected: 9,
        },
      },
      "s1",
      deps,
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    const data = event.data as Record<string, unknown>
    expect(data.description).toBe("summarized")
    expect(data.messages_affected).toBe(17)
  })

  test("replaces repeated sync compaction notices with the latest one", () => {
    const messagesRef = {
      current: [
        { role: "user", content: "continue" },
        { role: "assistant", content: "" },
      ] satisfies Message[],
    }
    const deps = createDeps(messagesRef)

    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 2,
          description: "summarization_not_applied_sync_compaction_only",
          messages_affected: 1,
        },
      },
      "s1",
      deps,
    )
    handleStreamEvent(
      {
        type: "context_compacted",
        data: {
          tier_applied: 2,
          description: "context_pruned",
          messages_affected: 9,
        },
      },
      "s1",
      deps,
    )

    expect(messagesRef.current.map((m) => m.role)).toEqual(["user", "event", "assistant"])
    const event = parseEvent(messagesRef.current[1])
    const data = event.data as Record<string, unknown>
    expect(data.description).toBe("context_pruned")
    expect(data.messages_affected).toBe(9)
  })
})
