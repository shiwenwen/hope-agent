// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { IconTip, TooltipProvider } from "./tooltip"

function renderWithTooltipProvider(ui: React.ReactNode) {
  return render(<TooltipProvider>{ui}</TooltipProvider>)
}

function mockVisibleRect() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    bottom: 60,
    height: 32,
    left: 20,
    right: 120,
    top: 28,
    width: 100,
    x: 20,
    y: 28,
    toJSON: () => ({}),
  })
}

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe("IconTip", () => {
  it("adds tooltip metadata without adding a duplicate native title", () => {
    renderWithTooltipProvider(
      <IconTip label="Open workspace" side="right">
        <button type="button">Workspace</button>
      </IconTip>,
    )

    const button = screen.getByRole("button", { name: "Workspace" })
    expect(button.getAttribute("title")).toBeNull()
    expect(button.getAttribute("data-ha-tip")).toBe("Open workspace")
    expect(button.getAttribute("data-ha-tip-side")).toBe("right")
    expect(button.className).toContain("ha-icon-tip")
  })

  it("suppresses an explicit native title owned by the wrapped element", () => {
    renderWithTooltipProvider(
      <IconTip label="Generated tooltip">
        <button type="button" title="Explicit title">
          Action
        </button>
      </IconTip>,
    )

    const button = screen.getByRole("button", { name: "Action" })
    expect(button.getAttribute("title")).toBeNull()
    expect(button.getAttribute("data-ha-tip")).toBe("Generated tooltip")
  })

  it("preserves an explicit accessible label", () => {
    renderWithTooltipProvider(
      <IconTip label="Visual hint">
        <button type="button" aria-label="Accessible action" />
      </IconTip>,
    )

    const button = screen.getByRole("button", { name: "Accessible action" })
    expect(button.getAttribute("aria-label")).toBe("Accessible action")
    expect(button.getAttribute("data-ha-tip")).toBe("Visual hint")
  })

  it("adds an accessible label to an icon-only control", () => {
    renderWithTooltipProvider(
      <IconTip label="Icon action">
        <button type="button">
          <svg aria-hidden="true" />
        </button>
      </IconTip>,
    )

    expect(screen.getByRole("button", { name: "Icon action" })).toBeTruthy()
  })

  it("renders data-ha-title-tip through the standard delegated tooltip", async () => {
    mockVisibleRect()
    renderWithTooltipProvider(
      <span data-ha-title-tip="Complete truncated value">Short value</span>,
    )

    fireEvent.pointerOver(screen.getByText("Short value"))

    expect((await screen.findByRole("tooltip")).textContent).toContain("Complete truncated value")
  })

  it("shows delegated tooltips for pointer-events-none disabled controls", async () => {
    mockVisibleRect()
    renderWithTooltipProvider(
      <button
        type="button"
        disabled
        aria-label="Unavailable action"
        data-ha-title-tip="Unavailable until setup is complete"
      />,
    )

    fireEvent.pointerMove(document.body, { clientX: 40, clientY: 40 })

    expect((await screen.findByRole("tooltip")).textContent).toContain(
      "Unavailable until setup is complete",
    )
  })

  it("shows delegated tooltips immediately for keyboard focus", async () => {
    mockVisibleRect()
    renderWithTooltipProvider(
      <button type="button" data-ha-title-tip="Keyboard hint">
        Focusable action
      </button>,
    )

    fireEvent.focus(screen.getByRole("button", { name: "Focusable action" }))

    expect((await screen.findByRole("tooltip")).textContent).toContain("Keyboard hint")
  })

  it("tracks dynamic labels and closes when the anchor is removed", async () => {
    mockVisibleRect()
    const view = renderWithTooltipProvider(
      <span data-ha-title-tip="Initial label">Dynamic value</span>,
    )
    fireEvent.pointerOver(screen.getByText("Dynamic value"))
    expect((await screen.findByRole("tooltip")).textContent).toContain("Initial label")

    view.rerender(
      <TooltipProvider>
        <span data-ha-title-tip="Updated label">Dynamic value</span>
      </TooltipProvider>,
    )
    await waitFor(() => {
      expect(screen.getByRole("tooltip").textContent).toContain("Updated label")
    })

    view.rerender(<TooltipProvider>{null}</TooltipProvider>)
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull())
  })
})
