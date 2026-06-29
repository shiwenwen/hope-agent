import { describe, expect, test } from "vitest"
import { detectActiveQuickPrompt } from "./quickPromptTokens"

describe("detectActiveQuickPrompt", () => {
  test("detects a query at start or after whitespace", () => {
    expect(detectActiveQuickPrompt("#sum", 4)).toMatchObject({
      anchor: 0,
      caret: 4,
      token: "sum",
    })
    expect(detectActiveQuickPrompt("please #sum", 11)).toMatchObject({
      anchor: 7,
      caret: 11,
      token: "sum",
    })
  })

  test("does not trigger inside words or URL fragments", () => {
    expect(detectActiveQuickPrompt("C# guide", 3)).toBeNull()
    expect(detectActiveQuickPrompt("https://x.test/#section", 23)).toBeNull()
  })

  test("stops at whitespace and repeated hash characters", () => {
    expect(detectActiveQuickPrompt("#one two", 8)).toBeNull()
    expect(detectActiveQuickPrompt("##one", 5)).toBeNull()
  })
})
