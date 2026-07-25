// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import "@testing-library/jest-dom/vitest"
import { afterEach, describe, expect, test, vi } from "vitest"
import type { AskUserQuestionGroup } from "@/components/chat/ask-user/AskUserQuestionBlock"
import { PetAskUserCard } from "./PetAskUserCard"

const mocks = vi.hoisted(() => ({ call: vi.fn(), warn: vi.fn() }))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: mocks.call }),
}))

vi.mock("@/lib/logger", () => ({ logger: { warn: mocks.warn } }))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const group: AskUserQuestionGroup = {
  requestId: "ask-pet",
  sessionId: "session-pet",
  context: "A short decision is needed.",
  questions: [
    {
      questionId: "color",
      text: "Choose a color",
      options: [
        { value: "red", label: "Red" },
        { value: "blue", label: "Blue" },
      ],
      allowCustom: true,
      multiSelect: false,
    },
    {
      questionId: "reason",
      text: "Explain the choice",
      options: [],
      allowCustom: true,
      multiSelect: false,
      inputKind: "text",
    },
    {
      questionId: "features",
      text: "Choose features",
      options: [
        { value: "compact", label: "Compact" },
        { value: "quiet", label: "Quiet" },
      ],
      allowCustom: true,
      multiSelect: true,
    },
  ],
}

describe("PetAskUserCard", () => {
  test("shows one question at a time and preserves answers across back/next navigation", () => {
    render(<PetAskUserCard group={group} />)

    expect(screen.getByText("Choose a color")).toBeInTheDocument()
    expect(screen.queryByText("Explain the choice")).not.toBeInTheDocument()
    const next = screen.getByRole("button", { name: "Next" })
    expect(next).toBeDisabled()

    fireEvent.click(screen.getByRole("button", { name: "Red" }))
    expect(next).toBeEnabled()
    fireEvent.click(next)
    expect(screen.getByText("Explain the choice")).toBeInTheDocument()
    expect(screen.queryByText("Choose a color")).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Previous" }))
    expect(screen.getByRole("button", { name: "Red" })).toHaveAttribute("aria-pressed", "true")
  })

  test("submits all paged answers together from the final question", async () => {
    mocks.call.mockResolvedValue(undefined)
    render(<PetAskUserCard group={group} />)

    fireEvent.click(screen.getByRole("button", { name: "Blue" }))
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    fireEvent.change(screen.getByPlaceholderText("Explain the choice"), {
      target: { value: "It matches the pet" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    fireEvent.click(screen.getByRole("button", { name: "Compact" }))
    fireEvent.click(screen.getByRole("button", { name: "Quiet" }))
    fireEvent.click(screen.getByRole("button", { name: "Submit" }))

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("respond_ask_user_question", {
        requestId: "ask-pet",
        answers: [
          { questionId: "color", selected: ["blue"], customInput: undefined },
          { questionId: "reason", selected: [], customInput: "It matches the pet" },
          { questionId: "features", selected: ["compact", "quiet"], customInput: undefined },
        ],
      })
    })
  })
})
