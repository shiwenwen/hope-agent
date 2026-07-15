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
