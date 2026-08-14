// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { SelectionActionMenu } from "./SelectionActionMenu"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}))

afterEach(cleanup)

describe("SelectionActionMenu", () => {
  test("keeps its final position and text mounted during the exit animation", () => {
    const onCopy = vi.fn()
    const onClose = vi.fn()
    const { rerender } = render(
      <SelectionActionMenu
        open
        position={{ x: 24, y: 48 }}
        text="selected text"
        onCopy={onCopy}
        onClose={onClose}
      />,
    )

    const copy = screen.getByRole("button", { name: "Copy selection", hidden: true })
    const surface = copy.closest("[style]") as HTMLElement
    expect(surface.style.left).toBe("24px")
    expect(surface.style.top).toBe("48px")

    rerender(
      <SelectionActionMenu
        open={false}
        position={null}
        text=""
        onCopy={onCopy}
        onClose={onClose}
      />,
    )

    const exitingCopy = screen.getByRole("button", { name: "Copy selection", hidden: true })
    expect((exitingCopy.closest("[style]") as HTMLElement).style.left).toBe("24px")
  })

  test("preserves the DOM selection on pointer-down and dispatches the cached text", () => {
    const onCopy = vi.fn()
    const onClose = vi.fn()
    render(
      <SelectionActionMenu
        open
        position={{ x: 24, y: 48 }}
        text="selected text"
        onCopy={onCopy}
        onClose={onClose}
      />,
    )
    const copy = screen.getByRole("button", { name: "Copy selection", hidden: true })
    const pointerDown = new Event("pointerdown", { bubbles: true, cancelable: true })
    fireEvent(copy, pointerDown)
    expect(pointerDown.defaultPrevented).toBe(true)

    fireEvent.click(copy)
    expect(onCopy).toHaveBeenCalledWith("selected text")
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
