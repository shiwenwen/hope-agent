// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"

import { TooltipProvider } from "@/components/ui/tooltip"
import { openExternalUrl } from "@/lib/openExternalUrl"
import MarkdownRenderer from "./MarkdownRenderer"

const runFileAction = vi.hoisted(() => vi.fn())

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))
vi.mock("@/lib/openExternalUrl", () => ({ openExternalUrl: vi.fn() }))
vi.mock("@/hooks/useSafeFavicon", () => ({ useSafeFavicon: () => null }))
vi.mock("@/components/chat/files/useFileResource", () => ({
  useFileResource: () => ({ primary: "preview", menu: ["preview"], run: runFileAction }),
}))

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

it.each([
  { kind: "web", href: "https://example.com/docs", local: false },
  { kind: "document", href: "/tmp/report.md", local: true },
  { kind: "mail", href: "mailto:help@example.com", local: false },
])("renders $kind link titles through the shared tooltip and preserves clicks", async (row) => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    bottom: 60,
    height: 32,
    left: 20,
    right: 120,
    top: 28,
    width: 100,
    x: 20,
    y: 28,
    toJSON: () => ({}),
  })
  render(
    <TooltipProvider delayDuration={0}>
      <MarkdownRenderer content={`[文档](${row.href} "补充说明")`} />
    </TooltipProvider>,
  )

  const link = await screen.findByRole("link", { name: "文档" })
  expect(link.getAttribute("href")).toBe(row.href)
  expect(link.getAttribute("data-link-kind")).toBe(row.kind)
  expect(link.getAttribute("title")).toBeNull()
  expect(link.getAttribute("data-ha-title-tip")).toBe("补充说明")

  fireEvent.pointerOver(link)
  expect((await screen.findByRole("tooltip")).textContent).toContain("补充说明")
  fireEvent.click(link)
  if (row.local) {
    expect(runFileAction).toHaveBeenCalledWith("preview")
    expect(openExternalUrl).not.toHaveBeenCalled()
  } else {
    expect(openExternalUrl).toHaveBeenCalledWith(row.href)
    expect(runFileAction).not.toHaveBeenCalled()
  }
})

it("does not add a tooltip when the Markdown link has no title", async () => {
  render(<MarkdownRenderer content="[文档](https://example.com/docs)" />)

  const link = await screen.findByRole("link", { name: "文档" })
  expect(link.getAttribute("title")).toBeNull()
  expect(link.getAttribute("data-ha-title-tip")).toBeNull()
})
