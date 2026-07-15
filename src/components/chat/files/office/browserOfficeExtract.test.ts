import { describe, expect, it } from "vitest"

import { extractOfficeFileInBrowser } from "./browserOfficeExtract"

describe("extractOfficeFileInBrowser", () => {
  it("enforces the configured document-preview limit before parsing", async () => {
    const file = new File(["too large"], "draft.docx", {
      type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    })

    await expect(extractOfficeFileInBrowser(file, file.size - 1)).rejects.toThrow(
      "file too large to preview",
    )
  })

  it("extracts spreadsheet text entirely from the client File", async () => {
    const XLSX = await import("xlsx")
    const workbook = XLSX.utils.book_new()
    XLSX.utils.book_append_sheet(
      workbook,
      XLSX.utils.aoa_to_sheet([
        ["name", "count"],
        ["local draft", 2],
      ]),
      "Items",
    )
    const bytes = XLSX.write(workbook, { type: "array", bookType: "xlsx" }) as ArrayBuffer
    const file = new File([bytes], "draft.xlsx", {
      type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    })

    await expect(extractOfficeFileInBrowser(file, file.size)).resolves.toMatchObject({
      relPath: "draft.xlsx",
      kind: "office",
      text: expect.stringContaining("local draft,2"),
      images: [],
    })
  })
})
