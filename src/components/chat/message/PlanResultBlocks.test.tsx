// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
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
})
