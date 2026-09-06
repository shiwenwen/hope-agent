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
  test("effective unlimited waits do not advertise inactive model defaults", () => {
    render(
      <AskUserQuestionBlock
        group={{ ...group, questions: [{ ...group.questions[0], defaultValues: ["a"] }] }}
      />,
    )
    expect(screen.getByText("planMode.question.waitForever")).toBeInTheDocument()
    expect(screen.queryByText("planMode.question.default")).not.toBeInTheDocument()
    expect(screen.queryByText(/planMode.question.fallback/)).not.toBeInTheDocument()
  })

  test("timed waits disclose both option and free-text fallback assumptions", () => {
    render(
      <AskUserQuestionBlock
        group={{
          ...group,
          timeoutAt: Math.floor(Date.now() / 1000) + 120,
          questions: [{ ...group.questions[0], defaultValues: ["a", "Only reversible changes"] }],
        }}
      />,
    )
    expect(screen.getByText("planMode.question.timeoutWithFallback")).toBeInTheDocument()
    expect(
      screen.getByText(/planMode.question.fallback: Option A · Only reversible changes/),
    ).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /Option A/ })).toHaveAttribute(
      "aria-pressed",
      "false",
    )
  })

  test("owner questions never advertise default answers", () => {
    render(
      <AskUserQuestionBlock
        group={{
          ...group,
          timeoutAt: Math.floor(Date.now() / 1000) + 120,
          ownerResponse: { action: "record_domain_evidence" },
          questions: [{ ...group.questions[0], defaultValues: ["a"] }],
        }}
      />,
    )
    expect(screen.getByText("planMode.question.timeoutWithoutFallback")).toBeInTheDocument()
    expect(screen.queryByText("planMode.question.default")).not.toBeInTheDocument()
  })

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
