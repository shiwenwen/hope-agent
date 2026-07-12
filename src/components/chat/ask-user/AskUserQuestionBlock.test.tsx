// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import AskUserQuestionBlock, { type AskUserQuestionGroup } from "./AskUserQuestionBlock"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

afterEach(cleanup)

const group: AskUserQuestionGroup = {
  requestId: "ask-1",
  sessionId: "session-1",
  questions: [
    {
      questionId: "choice",
      text: "Choose one",
      options: [
        { value: "a", label: "Option A" },
        { value: "b", label: "Option B" },
      ],
      allowCustom: true,
      multiSelect: false,
    },
  ],
}

describe("AskUserQuestionBlock", () => {
  test("selects an option on the first click after hover", () => {
    render(<AskUserQuestionBlock group={group} />)

    const option = screen.getByRole("button", { name: "Option A" })
    fireEvent.mouseEnter(option)
    fireEvent.pointerDown(option)
    fireEvent.mouseDown(option)
    fireEvent.pointerUp(option)
    fireEvent.mouseUp(option)
    fireEvent.click(option)

    expect(option.getAttribute("aria-pressed")).toBe("true")
  })
})
