// @vitest-environment jsdom

import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import { WorkbenchSurface } from "./WorkbenchSurface"

describe("WorkbenchSurface", () => {
  it("renders docked workbench content", () => {
    const { rerender } = render(
      <WorkbenchSurface width={720} layoutMode="docked">
        <div>Workbench content</div>
      </WorkbenchSurface>,
    )

    const surface = screen.getByText("Workbench content").closest("section")
    expect(surface).toHaveStyle({ width: "720px" })
    expect(surface?.className).toContain("transition-[width,opacity,border-color]")

    rerender(
      <WorkbenchSurface width={720} layoutMode="docked" collapsed>
        <div>Workbench content</div>
      </WorkbenchSurface>,
    )
    expect(surface).toHaveStyle({ width: "0px" })
    expect(surface).toHaveAttribute("aria-hidden", "true")

    rerender(
      <WorkbenchSurface width={720} layoutMode="docked" maximized>
        <div>Workbench content</div>
      </WorkbenchSurface>,
    )
    expect(surface).toHaveStyle({ width: "100%" })
    expect(surface?.className).toContain("fixed")
    expect(surface?.className).toContain("top-[72px]")
  })
})
