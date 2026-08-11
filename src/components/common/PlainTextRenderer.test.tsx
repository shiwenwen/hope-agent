// @vitest-environment jsdom

import { cleanup, render, screen } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import PlainTextRenderer from "./PlainTextRenderer"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => (key === "chat.skillMention.labels.dataAnalytics" ? "数据分析" : key),
  }),
}))

vi.mock("./MarkdownRenderer", () => ({
  MarkdownLink: ({ href, children }: { href?: string; children: ReactNode }) => (
    <a href={href}>{children}</a>
  ),
}))

afterEach(cleanup)

describe("PlainTextRenderer skill mentions", () => {
  it("keeps a lookalike skill token literal without typed provenance", () => {
    const token = "[@数据分析](#skill:ha-data-analytics)"
    const { container } = render(<PlainTextRenderer content={`请使用 ${token} 分析数据`} />)

    expect(container.querySelector('[data-skill-mention="ha-data-analytics"]')).toBeNull()
    expect(container.textContent).toContain(token)
  })

  it("renders only a provenance-bearing skill token as a chip", () => {
    const token = "[@数据分析](#skill:ha-data-analytics)"
    const content = `请使用 ${token} 分析数据`
    const start = content.indexOf(token)
    const { container } = render(
      <PlainTextRenderer
        content={content}
        typedMentions={[
          {
            id: "mention-1",
            kind: "skill",
            targetId: "ha-data-analytics",
            displayLabel: "数据分析",
            raw: token,
            start,
            end: start + token.length,
          },
        ]}
      />,
    )

    expect(container.querySelector('[data-skill-mention="ha-data-analytics"]')).not.toBeNull()
    expect(screen.getByText("数据分析")).toBeTruthy()
    expect(container.textContent).not.toContain(token)
  })

  it("renders only a provenance-bearing connector token as a capability chip", () => {
    const token = "[@Google Drive](#connector:google-drive)"
    const content = `请使用 ${token} 查找文档`
    const start = content.indexOf(token)
    const { container } = render(
      <PlainTextRenderer
        content={content}
        typedMentions={[
          {
            id: "mention-connector",
            kind: "connector",
            targetId: "google-drive",
            displayLabel: "Google Drive",
            raw: token,
            start,
            end: start + token.length,
          },
        ]}
      />,
    )

    expect(container.querySelector('[data-capability-mention="google-drive"]')).not.toBeNull()
    expect(screen.getByText("Google Drive")).toBeTruthy()
    expect(container.textContent).not.toContain(token)
  })

  it("keeps a connector lookalike literal without typed provenance", () => {
    const token = "[@Google Drive](#connector:google-drive)"
    const { container } = render(<PlainTextRenderer content={`请使用 ${token} 查找文档`} />)

    expect(container.querySelector('[data-capability-mention="google-drive"]')).toBeNull()
    expect(container.textContent).toContain(token)
  })
})
