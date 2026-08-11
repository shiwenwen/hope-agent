// @vitest-environment jsdom

import { cleanup, render, waitFor } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import type { ComposerMentionBinding } from "@/components/chat/mentions/typedMentions"
import MarkdownRenderer from "./MarkdownRenderer"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

afterEach(cleanup)

it("keeps a typed file treatment after history hydration replaces the binding objects", async () => {
  const content = "@AGENTS.md 中都有哪些红线内容"
  const mention: ComposerMentionBinding = {
    id: "mention-file",
    kind: "file",
    targetId: "AGENTS.md",
    displayLabel: "AGENTS.md",
    raw: "@AGENTS.md",
    start: 0,
    end: "@AGENTS.md".length,
  }
  const view = render(<MarkdownRenderer content={content} typedMentions={[mention]} />)

  await waitFor(() => {
    expect(view.container.querySelector('[data-file-mention="AGENTS.md"]')).not.toBeNull()
  })

  view.rerender(<MarkdownRenderer content={content} typedMentions={[{ ...mention }]} />)

  await waitFor(() => {
    expect(view.container.querySelector('[data-file-mention="AGENTS.md"]')).not.toBeNull()
    expect(view.container.textContent).not.toContain("# file")
  })
})

it("renders a typed note with the shared borderless treatment", async () => {
  const raw = "[[Welcome to your knowledge space]]"
  const view = render(
    <MarkdownRenderer
      content={`${raw} 讲了什么`}
      typedMentions={[
        {
          id: "mention-note",
          kind: "note",
          targetId: "kb-1::Welcome.md",
          displayLabel: "Welcome to your knowledge space",
          raw,
          start: 0,
          end: raw.length,
        },
      ]}
    />,
  )

  await waitFor(() => {
    const mention = view.container.querySelector('[data-note-mention="kb-1::Welcome.md"]')
    expect(mention).not.toBeNull()
    expect(mention?.className).not.toContain("border")
    expect(mention?.className).not.toContain("bg-")
  })
})

it.each([
  {
    kind: "file" as const,
    targetId: "AGENTS.md",
    displayLabel: "AGENTS.md",
    raw: "@AGENTS.md",
    selector: '[data-file-mention="AGENTS.md"]',
  },
  {
    kind: "note" as const,
    targetId: "kb-1::Welcome.md",
    displayLabel: "Welcome to your knowledge space",
    raw: "[[Welcome to your knowledge space]]",
    selector: '[data-note-mention="kb-1::Welcome.md"]',
  },
  {
    kind: "skill" as const,
    targetId: "ha-data-analytics",
    displayLabel: "数据分析",
    raw: "[@数据分析](#skill:ha-data-analytics)",
    selector: '[data-skill-mention="ha-data-analytics"]',
  },
  {
    kind: "agent" as const,
    targetId: "ha-main",
    displayLabel: "主 Agent",
    raw: "[@主 Agent](#agent:ha-main)",
    selector: '[data-agent-mention="ha-main"]',
  },
  {
    kind: "plugin" as const,
    targetId: "example-plugin",
    displayLabel: "示例插件",
    raw: "[@示例插件](#plugin:example-plugin)",
    selector: '[data-capability-mention="example-plugin"]',
  },
  {
    kind: "connector" as const,
    targetId: "google-drive",
    displayLabel: "Google Drive",
    raw: "[@Google Drive](#connector:google-drive)",
    selector: '[data-capability-mention="google-drive"]',
  },
  {
    kind: "plan" as const,
    targetId: "abcd1234:v2",
    displayLabel: "发布计划",
    raw: "@plan:abcd1234:v2",
    selector: '[data-plan-mention="abcd1234:v2"]',
  },
])("renders typed $kind mentions without a background or border", async (row) => {
  const view = render(
    <MarkdownRenderer
      content={`${row.raw} 请继续`}
      typedMentions={[
        {
          id: `mention-${row.kind}`,
          kind: row.kind,
          targetId: row.targetId,
          displayLabel: row.displayLabel,
          raw: row.raw,
          start: 0,
          end: row.raw.length,
        },
      ]}
    />,
  )

  await waitFor(() => {
    const mention = view.container.querySelector(row.selector)
    expect(mention).not.toBeNull()
    expect(mention?.className).not.toContain("border")
    expect(mention?.className).not.toContain("bg-")
    expect(mention?.className).not.toContain("shadow")
    expect(mention?.className).toContain("items-baseline")
  })
})
