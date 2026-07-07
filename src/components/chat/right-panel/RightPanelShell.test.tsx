// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
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
})
