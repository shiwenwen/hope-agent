// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, test, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AttachmentPreview } from "./AttachmentBar"
import { createPastedTextAttachment, getPastedTextFileMeta } from "./pastedTextAttachment"
import { createDraftAttachment } from "@/components/chat/files/types"

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

vi.mock("@/components/chat/files/StagedFilePreviewPane", () => ({
  StagedFilePreviewPane: ({ target }: { target: { draft: { file: File } } }) => (
    <div data-testid="staged-file-preview">preview:{target.draft.file.name}</div>
  ),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("AttachmentPreview", () => {
  test("shows file metadata and previews a regular staged attachment on click", async () => {
    const file = new File([new Uint8Array(2048)], "report.pdf", { type: "application/pdf" })

    render(
      <TooltipProvider>
        <AttachmentPreview
          attachedFiles={[createDraftAttachment(file, "picker")]}
          onRemoveFile={vi.fn()}
          onUpdateFile={vi.fn()}
        />
      </TooltipProvider>,
    )

    expect(screen.getByText("report.pdf")).toBeTruthy()
    expect(screen.getByText("2.0 KB")).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "report.pdf" }))

    expect((await screen.findByTestId("staged-file-preview")).textContent).toBe(
      "preview:report.pdf",
    )
  })

  test("opens the shared file action menu on right click", async () => {
    const file = new File(["hello"], "notes.txt", { type: "text/plain" })

    render(
      <TooltipProvider>
        <AttachmentPreview
          attachedFiles={[createDraftAttachment(file, "picker")]}
          onRemoveFile={vi.fn()}
          onUpdateFile={vi.fn()}
        />
      </TooltipProvider>,
    )

    fireEvent.contextMenu(screen.getByRole("button", { name: "notes.txt" }))

    expect(await screen.findByText("fileActions.preview")).toBeTruthy()
    expect(screen.getByText("fileActions.open")).toBeTruthy()
    expect(screen.getByText("fileActions.download")).toBeTruthy()
  })

  test("edits a pasted text attachment and replaces the staged file", async () => {
    const onUpdateFile = vi.fn()
    const file = createPastedTextAttachment("title\n" + "body\n".repeat(35))

    render(
      <TooltipProvider>
        <AttachmentPreview
          attachedFiles={[createDraftAttachment(file, "paste", "pasted_text")]}
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
