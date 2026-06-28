import { describe, expect, test } from "vitest"
import type { Message } from "@/types/chat"
import { isQuickPromptEligibleUserMessage, recentUserInputHistory } from "./messageQuickPrompts"

const user = (content: string, extra: Partial<Message> = {}): Message => ({
  role: "user",
  content,
  ...extra,
})

describe("message quick prompt helpers", () => {
  test("accepts only real user text messages", () => {
    expect(isQuickPromptEligibleUserMessage(user("hello"))).toBe(true)
    expect(isQuickPromptEligibleUserMessage(user("   "))).toBe(false)
    expect(isQuickPromptEligibleUserMessage(user("plan", { isPlanTrigger: true }))).toBe(false)
    expect(isQuickPromptEligibleUserMessage(user("cron", { isCronTrigger: true }))).toBe(false)
    expect(isQuickPromptEligibleUserMessage(user("agent", { fromAgentId: "a1" }))).toBe(false)
    expect(
      isQuickPromptEligibleUserMessage(
        user("slash", { slashEvent: { kind: "command", displayAs: "user" } }),
      ),
    ).toBe(false)
  })

  test("builds newest-first de-duplicated input history", () => {
    expect(
      recentUserInputHistory([
        user("first"),
        { role: "assistant", content: "ignored" },
        user("second"),
        user(" first "),
        user("plan", { isPlanTrigger: true }),
      ] as Message[]),
    ).toEqual(["first", "second"])
  })
})
