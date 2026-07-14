// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import ModelPickerCard from "./ModelPickerCard"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}))

vi.mock("@/components/common/ProviderIcon", () => ({
  default: () => <span data-testid="provider-icon" />,
}))

afterEach(cleanup)

describe("ModelPickerCard", () => {
  const models = [
    {
      providerId: "openai",
      providerName: "OpenAI",
      modelId: "gpt-media",
      modelName: "GPT Media",
      inputTypes: ["text", "image", "video"],
    },
    {
      providerId: "openai",
      providerName: "OpenAI",
      modelId: "gpt-text",
      modelName: "GPT Text",
      inputTypes: ["text"],
    },
  ]

  it("shows image and video capabilities and keeps model selection working", () => {
    const onSelect = vi.fn()
    render(
      <ModelPickerCard
        data={{
          models,
        }}
        onSelect={onSelect}
      />,
    )

    expect(screen.getByLabelText("model.supportsImageMultimodal")).toBeTruthy()
    expect(screen.getByLabelText("model.supportsVideoMultimodal")).toBeTruthy()
    expect(screen.getByTestId("provider-icon")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /GPT Media/ }))
    expect(onSelect).toHaveBeenCalledWith("openai", "gpt-media")
  })

  it("disables models missing required input types by default", () => {
    const onSelect = vi.fn()
    render(
      <ModelPickerCard
        data={{ models }}
        onSelect={onSelect}
        requiredInputTypes={["image"]}
      />,
    )

    const unsupportedModel = screen.getByRole("button", { name: "GPT Text" })
    expect((unsupportedModel as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(unsupportedModel)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it("can hide models missing required input types", () => {
    render(
      <ModelPickerCard
        data={{ models }}
        onSelect={vi.fn()}
        requiredInputTypes={["image"]}
        unsupportedBehavior="hide"
      />,
    )

    expect(screen.queryByRole("button", { name: "GPT Text" })).toBeNull()
  })
})
