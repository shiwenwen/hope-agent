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
    expect(within(withDefault!).getByText("planMode.question.fallback")).toBeInTheDocument()
    expect(within(withoutDefault!).getByText("timed out")).toBeInTheDocument()
    expect(
      within(withoutDefault!).queryByText("planMode.question.fallback"),
    ).not.toBeInTheDocument()
    expect(screen.queryByText("planMode.question.answered")).not.toBeInTheDocument()
  })

  test("new timeout results show fallback identity without claiming the user answered", () => {
    render(
      <AskUserQuestionResult
        toolArguments={JSON.stringify({
          questions: [
            { question_id: "required", text: "Which account?", options: [] },
            {
              question_id: "optional",
              text: "Which style?",
              options: [{ value: "simple", label: "Simple" }],
            },
          ],
        })}
        result={JSON.stringify({
          status: "timed_out",
          answers: [],
          fallback: [
            {
              questionId: "optional",
              question: "Which style?",
              selected: ["Simple"],
              selectedValues: ["simple"],
            },
          ],
        })}
      />,
    )
    const required = screen.getByText("Which account?").closest("section")!
    const optional = screen.getByText("Which style?").closest("section")!
    expect(within(required).getByText("tools.ask_user.no_answers")).toBeInTheDocument()
    expect(within(required).queryByText("Simple")).not.toBeInTheDocument()
    expect(within(optional).getByText("planMode.question.fallback")).toBeInTheDocument()
    expect(screen.queryByText("planMode.question.answered")).not.toBeInTheDocument()
    expect(screen.queryByText("planMode.question.response")).not.toBeInTheDocument()
  })

  test("keeps legacy timeout defaults in question order when question text repeats", () => {
    render(
      <AskUserQuestionResult
        toolArguments={JSON.stringify({
          questions: [
            { question_id: "frontend", text: "Choose a framework", header: "Frontend" },
            { question_id: "backend", text: "Choose a framework", header: "Backend" },
            { question_id: "unspecified", text: "Choose a framework", header: "Unspecified" },
          ],
        })}
        result={JSON.stringify({
          timedOut: true,
          answers: [
            { question: "Choose a framework", selected: ["React"] },
            { question: "Choose a framework", selected: [], customInput: "Axum" },
            { question: "Choose a framework", selected: [] },
          ],
        })}
      />,
    )
    const frontend = within(screen.getByText("Frontend").closest("section")!)
    const backend = within(screen.getByText("Backend").closest("section")!)
    const unspecified = within(screen.getByText("Unspecified").closest("section")!)
    expect(frontend.getByText("React")).toBeInTheDocument()
    expect(frontend.queryByText("Axum")).not.toBeInTheDocument()
    expect(backend.getByText("Axum")).toBeInTheDocument()
    expect(backend.queryByText("React")).not.toBeInTheDocument()
    expect(unspecified.getByText("tools.ask_user.no_answers")).toBeInTheDocument()
    expect(unspecified.queryByText("React")).not.toBeInTheDocument()
  })

  test("empty timeout and structured cancellation remain visible", () => {
    const { rerender } = render(
      <AskUserQuestionResult
        result={JSON.stringify({ status: "timed_out", answers: [], fallback: [] })}
      />,
    )
    expect(screen.getByText(/tools.ask_user.no_answers/)).toBeInTheDocument()
    rerender(
      <AskUserQuestionResult result={JSON.stringify({ status: "cancelled", answers: [] })} />,
    )
    expect(screen.getByText("tools.ask_user.cancelled")).toBeInTheDocument()
  })
})
