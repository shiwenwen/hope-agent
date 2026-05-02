import { describe, expect, test } from "vitest"
import { getLatestUserTurnKey, getMessageRowKey } from "./chatScrollKeys"
import type { Message } from "@/types/chat"

function message(patch: Partial<Message>): Message {
  return {
    role: "assistant",
    content: "",
    ...patch,
  } as Message
}

describe("getLatestUserTurnKey", () => {
  test("uses stable message identity without embedding user content", () => {
    const longContent = "x".repeat(10_000)

    expect(
      getLatestUserTurnKey([
        message({ role: "assistant", content: "previous" }),
        message({
          role: "user",
          content: longContent,
          timestamp: "2026-04-26T00:01:00.000Z",
        }),
        message({ role: "assistant", content: "" }),
      ]),
    ).toBe("user-turn:ts:user:2026-04-26T00:01:00.000Z")
  })

  test("prefers database id when available", () => {
    expect(
      getLatestUserTurnKey([
        message({ role: "user", content: "first", dbId: 1 }),
        message({ role: "assistant", content: "answer" }),
        message({ role: "user", content: "latest", dbId: 3 }),
      ]),
    ).toBe("user-turn:db:3")
  })

  test("stays stable when older messages are prepended", () => {
    const visibleWindow = [
      message({ role: "assistant", content: "answer", dbId: 20 }),
      message({ role: "user", content: "latest", dbId: 21 }),
      message({ role: "assistant", content: "streaming", dbId: 22 }),
    ]
    const prepended = [
      message({ role: "user", content: "older", dbId: 1 }),
      message({ role: "assistant", content: "older answer", dbId: 2 }),
      ...visibleWindow,
    ]

    expect(getLatestUserTurnKey(prepended)).toBe(getLatestUserTurnKey(visibleWindow))
  })
})

describe("getMessageRowKey", () => {
  test("prefers database id", () => {
    expect(getMessageRowKey(message({ role: "user", content: "x", dbId: 7 }), 0)).toBe("message:db:7")
  })

  test("falls back to timestamp with role discriminator", () => {
    expect(
      getMessageRowKey(
        message({ role: "user", content: "x", timestamp: "2026-04-26T00:00:00.000Z" }),
        2,
      ),
    ).toBe("message:ts:user:2026-04-26T00:00:00.000Z")
  })

  test("user and assistant messages with the same timestamp produce distinct keys", () => {
    const ts = "2026-04-26T00:00:00.000Z"
    const userKey = getMessageRowKey(message({ role: "user", content: "q", timestamp: ts }), 0)
    const assistantKey = getMessageRowKey(
      message({ role: "assistant", content: "", timestamp: ts }),
      1,
    )
    expect(userKey).not.toBe(assistantKey)
  })

  test("falls back to index when neither dbId nor timestamp is present", () => {
    expect(getMessageRowKey(message({ role: "user", content: "x" }), 5)).toBe("message:idx:5")
  })
})
