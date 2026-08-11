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

  it("renders only a provenance-bearing file token with the file mention style", () => {
    const token = "@AGENTS.md"
    const content = `${token} 中有哪些红线内容`
    const { container } = render(
      <PlainTextRenderer
        content={content}
        typedMentions={[
          {
            id: "mention-file",
            kind: "file",
            targetId: "AGENTS.md",
            displayLabel: "AGENTS.md",
            raw: token,
            start: 0,
            end: token.length,
          },
        ]}
      />,
    )

    expect(container.querySelector('[data-file-mention="AGENTS.md"]')).not.toBeNull()
    expect(container.textContent).toBe("AGENTS.md 中有哪些红线内容")
  })

  it("keeps a file lookalike literal without typed provenance", () => {
    const { container } = render(<PlainTextRenderer content="@AGENTS.md 中有哪些红线内容" />)

    expect(container.querySelector("[data-file-mention]")).toBeNull()
    expect(container.textContent).toContain("@AGENTS.md")
  })

  it("renders a provenance-bearing note with the note mention style", () => {
    const token = "[[Welcome to your knowledge space]]"
    const content = `${token} 讲了什么`
    const { container } = render(
      <PlainTextRenderer
        content={content}
        typedMentions={[
          {
            id: "mention-note",
            kind: "note",
            targetId: "kb-1::Welcome.md",
            displayLabel: "Welcome to your knowledge space",
            raw: token,
            start: 0,
            end: token.length,
          },
        ]}
      />,
    )

    expect(container.querySelector('[data-note-mention="kb-1::Welcome.md"]')).not.toBeNull()
    expect(container.textContent).toBe("Welcome to your knowledge space 讲了什么")
  })

  it("renders a provenance-bearing plan with the shared borderless mention style", () => {
    const token = "@plan:abcd1234:v2"
    const content = `${token} 还有哪些任务`
    const { container } = render(
      <PlainTextRenderer
        content={content}
        typedMentions={[
          {
            id: "mention-plan",
            kind: "plan",
            targetId: "abcd1234:v2",
            displayLabel: "发布计划",
            raw: token,
            start: 0,
            end: token.length,
          },
        ]}
      />,
    )

    const mention = container.querySelector('[data-plan-mention="abcd1234:v2"]')
    expect(mention).not.toBeNull()
    expect(mention?.className).not.toContain("border")
    expect(mention?.className).not.toContain("bg-")
    expect(container.textContent).toBe("发布计划 还有哪些任务")
  })
})
