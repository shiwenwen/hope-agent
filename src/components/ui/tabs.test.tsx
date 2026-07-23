// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
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

    expect(selected.getAttribute("data-state")).toBe("active")
    expect(selected.className).toContain("data-[state=active]:bg-background")
    expect(selected.className).not.toMatch(/data-\[state=active\]:shadow/)
    expect(selected.className).not.toContain("data-[state=active]:bg-secondary")
  })
})
