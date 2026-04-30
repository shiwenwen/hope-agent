import { describe, expect, test } from "vitest"
import { getLatestUserTurnKey } from "./chatScrollKeys"
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
    ).toBe("user-turn:1:ts:2026-04-26T00:01:00.000Z")
  })

  test("prefers database id when available", () => {
    expect(
      getLatestUserTurnKey([
        message({ role: "user", content: "first", dbId: 1 }),
        message({ role: "assistant", content: "answer" }),
        message({ role: "user", content: "latest", dbId: 3 }),
      ]),
    ).toBe("user-turn:2:db:3")
  })
})
