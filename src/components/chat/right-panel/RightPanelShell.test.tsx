// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import { RightPanelShell } from "./RightPanelShell"

describe("RightPanelShell", () => {
  it("uses a fixed overlay surface on narrow user-expanded layouts", () => {
    const { container } = render(
      <RightPanelShell
        width={520}
        resizeLabel="Resize panel"
        reservedMainWidth={420}
        overlay
      >
        <div>Workspace Control Panel</div>
      </RightPanelShell>,
    )

    const shell = container.firstElementChild
    expect(shell?.className).toContain("fixed")
    expect(shell?.className).toContain("inset-0")
    expect(screen.getByText("Workspace Control Panel")).toBeTruthy()
  })

  it("suspends the width transition while resizing", () => {
    const { container } = render(
      <RightPanelShell width={520} onWidthChange={vi.fn()} resizeLabel="Resize panel">
        <div>Workspace Control Panel</div>
      </RightPanelShell>,
    )

    const shell = container.firstElementChild as HTMLElement
    expect(shell.className).toContain("transition-[width,min-width,max-width,padding]")

    fireEvent.mouseDown(screen.getByRole("separator", { name: "Resize panel" }), {
      clientX: 500,
    })
    expect(shell.className).not.toContain("transition-[width,min-width,max-width,padding]")

    fireEvent.mouseUp(document)
    expect(shell.className).toContain("transition-[width,min-width,max-width,padding]")
  })
})
