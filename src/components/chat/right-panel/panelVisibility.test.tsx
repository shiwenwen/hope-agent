// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { RightPanelShell } from "./RightPanelShell"
import { usePanelRevealRefresh, usePanelVisible } from "./panelVisibility"

function Probe({ onReveal }: { onReveal?: () => void }) {
  const visible = usePanelVisible()
  usePanelRevealRefresh(() => onReveal?.())
  return <span data-testid="probe">{visible ? "visible" : "hidden"}</span>
}

describe("panel visibility", () => {
  it("defaults to visible outside a workbench shell", () => {
    render(<Probe />)
    expect(screen.getByTestId("probe").textContent).toBe("visible")
  })

  it("reports a collapsed shell as hidden so warm-mounted polls can stand down", () => {
    const { rerender, container } = render(
      <RightPanelShell>
        <Probe />
      </RightPanelShell>,
    )
    expect(container.querySelector('[data-testid="probe"]')?.textContent).toBe("visible")

    rerender(
      <RightPanelShell collapsed>
        <Probe />
      </RightPanelShell>,
    )
    expect(container.querySelector('[data-testid="probe"]')?.textContent).toBe("hidden")
  })

  it("refreshes on the way back into view, never on mount", () => {
    const onReveal = vi.fn()
    const shell = (collapsed: boolean) => (
      <RightPanelShell collapsed={collapsed}>
        <Probe onReveal={onReveal} />
      </RightPanelShell>
    )
    const { rerender } = render(shell(false))
    expect(onReveal).not.toHaveBeenCalled()

    rerender(shell(true))
    expect(onReveal).not.toHaveBeenCalled()

    rerender(shell(false))
    expect(onReveal).toHaveBeenCalledTimes(1)
  })
})
