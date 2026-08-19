// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import { RightPanelShell } from "./RightPanelShell"

describe("RightPanelShell", () => {
  it("stacks its content absolutely so tabs can share one surface", () => {
    const { container } = render(
      <RightPanelShell>
        <div>Panel body</div>
      </RightPanelShell>,
    )

    const shell = container.firstElementChild as HTMLElement
    expect(shell.className.split(" ")).toContain("absolute")
    expect(screen.getByText("Panel body")).toBeTruthy()
  })

  it("stops painting a surface and leaves the a11y tree when collapsed", () => {
    const { container: active } = render(
      <RightPanelShell>
        <div>Active</div>
      </RightPanelShell>,
    )
    const { container: collapsed } = render(
      <RightPanelShell collapsed>
        <div>Collapsed</div>
      </RightPanelShell>,
    )

    const activeShell = active.firstElementChild as HTMLElement
    const collapsedShell = collapsed.firstElementChild as HTMLElement
    expect(activeShell.className.split(" ")).not.toContain("bg-transparent")
    expect(collapsedShell.className.split(" ")).toContain("bg-transparent")
    expect(collapsedShell.className.split(" ")).toContain("pointer-events-none")
    expect(collapsedShell).toHaveAttribute("aria-hidden", "true")
    expect(collapsedShell.hasAttribute("inert")).toBe(true)
  })

  it("holds the first mount back one frame so it can fade in", () => {
    const { container } = render(
      <RightPanelShell animateOnMount>
        <div>Body</div>
      </RightPanelShell>,
    )

    // Before the entry frame lands the shell is treated exactly like a
    // collapsed one, so it never flashes at full opacity.
    const shell = container.firstElementChild as HTMLElement
    expect(shell.hasAttribute("inert")).toBe(true)
  })
})
