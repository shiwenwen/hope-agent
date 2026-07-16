// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { RadioPills } from "./radio-pills"

afterEach(cleanup)

describe("RadioPills", () => {
  it("renders strong mutually exclusive choices without state borders", () => {
    const onChange = vi.fn()

    render(
      <RadioPills
        value="web"
        onChange={onChange}
        variant="strong"
        layout="wrap"
        ariaLabel="产物类型"
        options={[
          { value: "web", label: "网页", icon: <span aria-hidden>W</span> },
          { value: "mobile", label: "移动端" },
        ]}
      />,
    )

    const group = screen.getByRole("radiogroup", { name: "产物类型" })
    const selected = screen.getByRole("radio", { name: "网页" })
    const unselected = screen.getByRole("radio", { name: "移动端" })

    expect(group.className).toContain("flex")
    expect(selected.getAttribute("aria-checked")).toBe("true")
    expect(selected.className).toContain("bg-primary")
    expect(selected.className).toContain("text-primary-foreground")
    expect(selected.className).not.toMatch(/\bborder(?:-|\b)/)
    expect(unselected.className).toContain("hover:bg-secondary/40")

    fireEvent.click(selected)
    expect(onChange).not.toHaveBeenCalled()

    fireEvent.click(unselected)
    expect(onChange).toHaveBeenCalledWith("mobile")
  })
})
