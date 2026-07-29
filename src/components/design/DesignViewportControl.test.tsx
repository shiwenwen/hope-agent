// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import {
  DesignViewportControl,
  DesignViewportResizeHandles,
} from "@/components/design/DesignViewportControl"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}))

afterEach(cleanup)

describe("DesignViewportControl", () => {
  it("shows a named device and its logical viewport size", () => {
    render(
      <DesignViewportControl
        value="iphone-15-pro"
        customSize={{ width: 390, height: 844 }}
        onValueChange={vi.fn()}
        onCustomSizeCommit={vi.fn()}
      />,
    )

    const picker = screen.getByRole("combobox", { name: "桌面 / 平板 / 手机" })
    expect(picker).toHaveTextContent("iPhone 15 Pro")
    expect(picker).toHaveTextContent("393×852")
  })

  it("commits custom width and height inputs", () => {
    const onCustomSizeCommit = vi.fn()
    render(
      <DesignViewportControl
        value="custom"
        customSize={{ width: 390, height: 844 }}
        onValueChange={vi.fn()}
        onCustomSizeCommit={onCustomSizeCommit}
      />,
    )

    const width = screen.getByRole("spinbutton", { name: "宽" })
    fireEvent.focus(width)
    fireEvent.change(width, { target: { value: "412" } })
    fireEvent.blur(width)
    expect(onCustomSizeCommit).toHaveBeenCalledWith({ width: 412, height: 844 })
  })

  it("keeps resize handles keyboard accessible", () => {
    const onSizeCommit = vi.fn()
    render(
      <div className="relative">
        <DesignViewportResizeHandles
          size={{ width: 390, height: 844 }}
          onResizeStart={vi.fn()}
          onSizeCommit={onSizeCommit}
        />
      </div>,
    )

    fireEvent.keyDown(screen.getByRole("separator", { name: "自定义 · 宽" }), {
      key: "ArrowRight",
    })
    expect(onSizeCommit).toHaveBeenCalledWith({ width: 400, height: 844 })
  })
})
