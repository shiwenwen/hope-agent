// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"
import { Tabs, TabsList, TabsTrigger } from "./tabs"

afterEach(cleanup)

describe("Tabs", () => {
  it("keeps the selected tab distinct from the muted tab list", () => {
    render(
      <Tabs defaultValue="models">
        <TabsList>
          <TabsTrigger value="providers">服务商</TabsTrigger>
          <TabsTrigger value="models">模型设置</TabsTrigger>
        </TabsList>
      </Tabs>,
    )

    const selected = screen.getByRole("tab", { name: "模型设置" })
    const unselected = screen.getByRole("tab", { name: "服务商" })

    expect(selected.getAttribute("data-state")).toBe("active")
    expect(selected.querySelector("[data-tabs-indicator]")?.className).toContain("bg-background")
    expect(unselected.querySelector("[data-tabs-indicator]")).toBeNull()
    expect(selected.className).not.toMatch(/data-\[state=active\]:shadow/)
    expect(selected.className).not.toContain("data-[state=active]:bg-secondary")

    fireEvent.mouseDown(unselected, { button: 0, ctrlKey: false })

    expect(unselected.querySelector("[data-tabs-indicator]")).not.toBeNull()
    expect(selected.querySelector("[data-tabs-indicator]")).toBeNull()
  })
})
