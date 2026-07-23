// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"

import { TogglePills } from "./toggle-pills"

afterEach(cleanup)

describe("TogglePills", () => {
  it("uses flat high contrast without adding a check mark", () => {
    const onToggle = vi.fn()

    render(
      <TogglePills
        values={new Set(["text"])}
        onToggle={onToggle}
        ariaLabel="支持的输入类型"
        options={[
          { value: "text", label: "文本", icon: <span aria-hidden>T</span> },
          { value: "image", label: "图片", icon: <span aria-hidden>I</span> },
        ]}
      />,
    )

    const group = screen.getByRole("group", { name: "支持的输入类型" })
    const selected = screen.getByRole("button", { name: "文本" })
    const unselected = screen.getByRole("button", { name: "图片" })

    expect(group).toBeTruthy()
    expect(selected.getAttribute("aria-pressed")).toBe("true")
    expect(selected.className).toContain("bg-primary")
    expect(selected.className).toContain("text-primary-foreground")
    expect(selected.className).not.toMatch(/\b(?:border|shadow)(?:-|\b)/)
    expect(selected.querySelector("svg")).toBeNull()
    expect(unselected.getAttribute("aria-pressed")).toBe("false")
    expect(unselected.className).toContain("bg-secondary")
    expect(unselected.className).toContain("text-secondary-foreground")
    expect(unselected.className).toContain("hover:bg-foreground/15")

    fireEvent.click(unselected)
    expect(onToggle).toHaveBeenCalledWith("image")
  })
})
