// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import PlainTextRenderer from "./PlainTextRenderer"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) =>
      key === "chat.skillMention.labels.dataAnalytics" ? "数据分析" : key,
  }),
}))

vi.mock("./MarkdownRenderer", () => ({
  MarkdownLink: ({ href, children }: { href?: string; children: ReactNode }) => (
    <a href={href}>{children}</a>
  ),
}))

afterEach(cleanup)

describe("PlainTextRenderer skill mentions", () => {
  it("renders Data Analytics as a chip instead of its markdown token", () => {
    const token = "[@数据分析](#skill:ha-data-analytics)"
    const { container } = render(<PlainTextRenderer content={`请使用 ${token} 分析数据`} />)

    expect(container.querySelector('[data-skill-mention="ha-data-analytics"]')).not.toBeNull()
    expect(screen.getByText("数据分析")).toBeTruthy()
    expect(container.textContent).not.toContain(token)
  })
})
