// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
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
})
