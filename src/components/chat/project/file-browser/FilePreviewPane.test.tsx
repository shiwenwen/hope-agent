// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"

import { TooltipProvider } from "@/components/ui/tooltip"
import type { PreviewSource } from "@/components/chat/files/previewSource"
import { FilePreviewPane } from "./FilePreviewPane"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("FilePreviewPane", () => {
  test("renders code previews with inline syntax colors", async () => {
    const source = {
      name: "example.ts",
      mime: "text/typescript",
      readText: vi.fn(async () => ({
        relPath: "example.ts",
        content: "const answer = 42",
        isBinary: false,
        mime: "text/typescript",
        totalLines: 1,
        sizeBytes: 17,
        truncated: false,
        contentHash: null,
        isUtf8: true,
        lineEnding: "lf" as const,
        hasUtf8Bom: false,
      })),
      extractDoc: vi.fn(),
      rawUrl: vi.fn(),
    } satisfies PreviewSource

    const { container } = render(
      <TooltipProvider>
        <FilePreviewPane source={source} />
      </TooltipProvider>,
    )

    await waitFor(() => {
      const tokens = [...container.querySelectorAll<HTMLElement>(".shiki .line > span")]
      expect(tokens.length).toBeGreaterThan(1)
      expect(new Set(tokens.map((token) => token.style.color)).size).toBeGreaterThan(1)
    })
  })

  test("shows a persistent open action in the preview header", () => {
    const onOpen = vi.fn()
    const source: PreviewSource = {
      name: "archive.zip",
      sizeBytes: 10,
      readText: vi.fn(async () => ({
        relPath: "archive.zip",
        content: "",
        isBinary: true,
        mime: "application/zip",
        totalLines: 0,
        sizeBytes: 10,
        truncated: false,
        contentHash: null,
        isUtf8: false,
        lineEnding: "lf" as const,
        hasUtf8Bom: false,
      })),
      extractDoc: vi.fn(async () => {
        throw new Error("not available")
      }),
      rawUrl: vi.fn(async () => "blob:archive"),
    }

    render(
      <TooltipProvider>
        <FilePreviewPane source={source} onOpen={onOpen} />
      </TooltipProvider>,
    )

    fireEvent.click(screen.getByRole("button", { name: "fileActions.open" }))
    expect(onOpen).toHaveBeenCalledTimes(1)
  })

  test("toggles ordinary HTML between highlighted source and an offline preview", async () => {
    const content = '<main id="preview">Hello</main><script>window.previewRan = true</script>'
    const source = {
      name: "preview.html",
      mime: "text/html",
      readText: vi.fn(async () => ({
        relPath: "preview.html",
        content,
        isBinary: false,
        mime: "text/html",
        totalLines: 1,
        sizeBytes: content.length,
        truncated: false,
        contentHash: null,
        isUtf8: true,
        lineEnding: "lf" as const,
        hasUtf8Bom: false,
      })),
      extractDoc: vi.fn(),
      rawUrl: vi.fn(),
    } satisfies PreviewSource

    const { container } = render(
      <TooltipProvider>
        <FilePreviewPane source={source} />
      </TooltipProvider>,
    )

    await waitFor(() => expect(container.querySelector(".shiki")).not.toBeNull())
    expect(screen.queryByTitle("preview.html")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "fileBrowser.rendered" }))

    const frame = screen.getByTitle("preview.html")
    const srcDoc = frame.getAttribute("srcdoc") ?? ""
    expect(srcDoc).toContain(content)
    expect(srcDoc).toContain("default-src 'none'")
    expect(srcDoc).toContain("script-src 'none'")
    expect(srcDoc).toContain("connect-src 'none'")
    expect(srcDoc.indexOf("Content-Security-Policy")).toBeLessThan(srcDoc.indexOf(content))
    expect(frame.getAttribute("sandbox")).toBe("")
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer")

    fireEvent.click(screen.getByRole("button", { name: "fileBrowser.viewSource" }))
    await waitFor(() => expect(container.querySelector(".shiki")).not.toBeNull())
  })

  test("renders managed Artifact HTML from its raw URL without reading it as code", async () => {
    const readText = vi.fn()
    const rawUrl = vi.fn(async () => "https://server.test/api/canvas/projects/a/index.html")
    const source = {
      name: "Report.html",
      mime: "text/html",
      presentation: "managed_html" as const,
      readText,
      extractDoc: vi.fn(),
      rawUrl,
    } satisfies PreviewSource

    render(
      <TooltipProvider>
        <FilePreviewPane source={source} />
      </TooltipProvider>,
    )

    await waitFor(() => expect(rawUrl).toHaveBeenCalledWith(false))
    const frame = screen.getByTitle("Report.html")
    expect(frame.getAttribute("src")).toBe(
      "https://server.test/api/canvas/projects/a/index.html",
    )
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts")
    expect(readText).not.toHaveBeenCalled()
  })
})
