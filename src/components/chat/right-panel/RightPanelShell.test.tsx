// @vitest-environment jsdom

import { act, fireEvent, render, screen } from "@testing-library/react"
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

  it("animates the first panel mount from zero to its configured width", () => {
    let enterFrame: FrameRequestCallback | null = null
    const requestFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        enterFrame = callback
        return 1
      })
    const cancelFrame = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {})

    const { container } = render(
      <RightPanelShell width={520} resizeLabel="Resize panel" animateOnMount>
        <div>Workspace Control Panel</div>
      </RightPanelShell>,
    )

    const shell = container.firstElementChild as HTMLElement
    expect(shell.style.width).toBe("0px")
    expect(shell.getAttribute("aria-hidden")).toBe("true")

    act(() => enterFrame?.(0))

    expect(shell.getAttribute("aria-hidden")).toBeNull()
    expect(shell.className.split(" ")).toContain("p-3")
    expect(shell.lastElementChild?.className).toContain("opacity-100")
    requestFrame.mockRestore()
    cancelFrame.mockRestore()
  })
})
