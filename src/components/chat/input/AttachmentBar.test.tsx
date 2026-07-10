// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AttachmentPreview } from "./AttachmentBar"
import { createPastedTextAttachment, getPastedTextFileMeta } from "./pastedTextAttachment"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
  },
}))

vi.mock("@/components/common/ImageLightbox", () => ({
  useLightbox: () => ({ openLightbox: vi.fn() }),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("AttachmentPreview", () => {
  test("edits a pasted text attachment and replaces the staged file", async () => {
    const onUpdateFile = vi.fn()
    const file = createPastedTextAttachment("title\n" + "body\n".repeat(35))

    render(
      <TooltipProvider>
        <AttachmentPreview
          attachedFiles={[file]}
          onRemoveFile={vi.fn()}
          onUpdateFile={onUpdateFile}
        />
      </TooltipProvider>,
    )

    fireEvent.click(screen.getByRole("button", { name: "chat.pastedTextPreviewOpen" }))

    const editor = await screen.findByRole("textbox", { name: "chat.pastedTextPreviewTitle" })
    fireEvent.change(editor, { target: { value: "edited\ncontent" } })
    fireEvent.click(screen.getByRole("button", { name: "common.save" }))

    expect(onUpdateFile).toHaveBeenCalledTimes(1)
    expect(onUpdateFile).toHaveBeenCalledWith(0, expect.any(File))
    const updatedFile = onUpdateFile.mock.calls[0]?.[1] as File
    expect(updatedFile.name).toBe(file.name)
    expect(await updatedFile.text()).toBe("edited\ncontent")
    expect(getPastedTextFileMeta(updatedFile)?.lineCount).toBe(2)
  })
})
