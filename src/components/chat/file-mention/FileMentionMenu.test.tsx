// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import FileMentionMenu from "./FileMentionMenu"

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallback?: string, values?: Record<string, unknown>) => {
      const translations: Record<string, string> = {
        "knowledge.mention.heading": "知识空间笔记",
        "knowledge.mention.loadFailed": "无法加载知识笔记候选",
        "knowledge.mention.errorDetail": "详细信息：{{error}}",
      }
      let text = translations[key] ?? fallback ?? key
      for (const [name, value] of Object.entries(values ?? {})) {
        text = text.replace(`{{${name}}}`, String(value))
      }
      return text
    },
  }),
}))

afterEach(cleanup)

describe("FileMentionMenu", () => {
  it("keeps the knowledge-note section visible when note candidates fail to load", () => {
    render(
      <FileMentionMenu
        isOpen
        entries={[]}
        noteEntries={[]}
        notesLoading={false}
        noteLoadErrorDetail="token=[redacted]"
        noteCapable
        skillEntries={[]}
        skillCapable={false}
        agentEntries={[]}
        agentCapable={false}
        selectedIndex={0}
        mode="search"
        dirPath={null}
        workingDir={null}
        loading={false}
        error={null}
        truncated={false}
        hasFileQuery={false}
        onSelect={() => {}}
        onSelectNote={() => {}}
        onSelectSkill={() => {}}
        onSelectAgent={() => {}}
        onHover={() => {}}
      />,
    )

    expect(screen.getByText("知识空间笔记")).toBeTruthy()
    expect(screen.getByText("无法加载知识笔记候选")).toBeTruthy()
    expect(screen.getByText("详细信息：token=[redacted]")).toBeTruthy()
  })
})
