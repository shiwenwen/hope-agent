// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import { IconTip, TooltipProvider } from "./tooltip"

function renderWithTooltipProvider(ui: React.ReactNode) {
  return render(<TooltipProvider>{ui}</TooltipProvider>)
}

describe("IconTip", () => {
  it("adds tooltip metadata to the original element without changing its role", () => {
    renderWithTooltipProvider(
      <IconTip label="Open workspace" side="right">
        <button type="button">Workspace</button>
      </IconTip>,
    )

    const button = screen.getByRole("button", { name: "Workspace" })
    expect(button.getAttribute("title")).toBe("Open workspace")
    expect(button.getAttribute("data-ha-tip")).toBe("Open workspace")
    expect(button.getAttribute("data-ha-tip-side")).toBe("right")
    expect(button.className).toContain("ha-icon-tip")
  })

  it("does not overwrite an explicit title", () => {
    renderWithTooltipProvider(
      <IconTip label="Generated tooltip">
        <button type="button" title="Explicit title">
          Action
        </button>
      </IconTip>,
    )

    const button = screen.getByRole("button", { name: "Action" })
    expect(button.getAttribute("title")).toBe("Explicit title")
    expect(button.getAttribute("data-ha-tip")).toBe("Generated tooltip")
  })
})
