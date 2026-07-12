// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import NoteMentionMenu from "./NoteMentionMenu"

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

describe("NoteMentionMenu", () => {
  it("renders referenceable-note load failures instead of the empty note state", () => {
    render(
      <NoteMentionMenu
        isOpen
        entries={[]}
        selectedIndex={0}
        loading={false}
        loadErrorDetail="Authorization: Bearer [redacted]"
        onSelect={() => {}}
        onHover={() => {}}
      />,
    )

    expect(screen.getByText("无法加载知识笔记候选")).toBeTruthy()
    expect(screen.getByText("详细信息：Authorization: Bearer [redacted]")).toBeTruthy()
    expect(screen.queryByText(/没有可引用的笔记/)).toBeNull()
  })
})
