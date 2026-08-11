// @vitest-environment jsdom

import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AgentMentionIcon } from "./AgentMentionChip"

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({
    resolveAssetUrl: (path: string) => `asset://${path}`,
  }),
}))

afterEach(cleanup)

describe("AgentMentionIcon", () => {
  it("prefers the configured avatar over emoji", () => {
    const { container } = render(
      <AgentMentionIcon
        agent={{ id: "reviewer", name: "评审", avatar: "reviewer.png", emoji: "🧐" }}
      />,
    )

    expect(container.querySelector("img")?.getAttribute("src")).toBe("asset://reviewer.png")
    expect(container.textContent).not.toContain("🧐")
  })

  it("uses emoji when there is no avatar", () => {
    const { container } = render(
      <AgentMentionIcon agent={{ id: "reviewer", name: "评审", emoji: "🧐" }} />,
    )

    expect(container.textContent).toBe("🧐")
    expect(container.querySelector("svg")).toBeNull()
  })

  it("uses the fixed icon only as the final fallback", () => {
    const { container } = render(<AgentMentionIcon agent={{ id: "reviewer", name: "评审" }} />)

    expect(container.querySelector("svg.lucide-bot")).not.toBeNull()
  })
})
