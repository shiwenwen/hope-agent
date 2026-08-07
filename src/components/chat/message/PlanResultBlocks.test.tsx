// @vitest-environment jsdom

import { cleanup, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { AskUserQuestionResult } from "./PlanResultBlocks"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: { defaultValue?: string }) => values?.defaultValue ?? key,
  }),
}))

afterEach(cleanup)

describe("AskUserQuestionResult", () => {
  test("matches duplicate labels by selected value", () => {
    render(
      <AskUserQuestionResult
        toolArguments={JSON.stringify({
          questions: [
            {
              question_id: "handling",
              text: "How should this be handled?",
              options: [
                { value: "steps", label: "Same label", description: "Only explain steps" },
                { value: "implement", label: "Same label", description: "Implement the change" },
              ],
            },
          ],
        })}
        result={JSON.stringify({
          answers: [
            {
              questionId: "handling",
              question: "How should this be handled?",
              selected: ["Same label"],
              selectedValues: ["implement"],
            },
          ],
        })}
      />,
    )

    const firstRow = screen.getByText("Only explain steps").parentElement?.parentElement
    const secondRow = screen.getByText("Implement the change").parentElement?.parentElement
    expect(firstRow).toHaveClass("bg-background/30")
    expect(secondRow).toHaveClass("bg-green-500/10")
    expect(firstRow).toHaveClass("border-border/50")
    expect(secondRow).toHaveClass("border-border/50")
  })

  test("ignores malformed raw options instead of throwing", () => {
    expect(() =>
      render(
        <AskUserQuestionResult
          toolArguments={JSON.stringify({
            questions: [{ question_id: "freeform", text: "What next?", options: {} }],
          })}
          result={JSON.stringify({
            answers: [
              {
                questionId: "freeform",
                question: "What next?",
                selected: [],
                selectedValues: [],
                customInput: "Use the safe fallback",
              },
            ],
          })}
        />,
      ),
    ).not.toThrow()
    expect(screen.getByText("Use the safe fallback")).toBeInTheDocument()
  })

  test("labels timeout defaults per question", () => {
    render(
      <AskUserQuestionResult
        toolArguments={JSON.stringify({
          questions: [
            {
              question_id: "with-default",
              text: "Default answer",
              options: [{ value: "safe", label: "Safe" }],
              default_values: ["safe"],
            },
            {
              question_id: "without-default",
              text: "No default answer",
              options: [{ value: "manual", label: "Manual" }],
            },
          ],
        })}
        result={JSON.stringify({
          timedOut: true,
          answers: [
            {
              questionId: "with-default",
              question: "Default answer",
              selected: ["Safe"],
              selectedValues: ["safe"],
            },
            {
              questionId: "without-default",
              question: "No default answer",
              selected: [],
              selectedValues: [],
            },
          ],
        })}
      />,
    )

    const withDefault = screen.getByText("Default answer").closest("section")
    const withoutDefault = screen.getByText("No default answer").closest("section")
    expect(withDefault).not.toBeNull()
    expect(withoutDefault).not.toBeNull()
    expect(within(withDefault!).getByText("tools.ask_user.timed_out")).toBeInTheDocument()
    expect(within(withoutDefault!).getByText("timed out")).toBeInTheDocument()
    expect(within(withoutDefault!).queryByText("tools.ask_user.timed_out")).not.toBeInTheDocument()
  })
})
