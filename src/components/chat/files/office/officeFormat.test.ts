import { describe, expect, test } from "vitest"

import { officeFormatOf } from "./officeFormat"

describe("officeFormatOf", () => {
  test("maps modern OOXML extensions", () => {
    expect(officeFormatOf("report.docx")).toBe("docx")
    expect(officeFormatOf("budget.xlsx")).toBe("xlsx")
    expect(officeFormatOf("deck.pptx")).toBe("pptx")
  })

  test("legacy .xls maps to xlsx (SheetJS reads BIFF), but .doc/.ppt do not", () => {
    expect(officeFormatOf("old.xls")).toBe("xlsx")
    expect(officeFormatOf("old.doc")).toBeNull()
    expect(officeFormatOf("old.ppt")).toBeNull()
  })

  test("MIME wins over a misleading/missing extension", () => {
    expect(officeFormatOf("blob.bin", "application/vnd.openxmlformats-officedocument.wordprocessingml.document")).toBe(
      "docx",
    )
    expect(officeFormatOf("blob", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")).toBe("xlsx")
    expect(officeFormatOf("blob", "application/vnd.ms-excel")).toBe("xlsx")
    expect(officeFormatOf("blob", "application/vnd.openxmlformats-officedocument.presentationml.presentation")).toBe(
      "pptx",
    )
  })

  test("legacy binary MIME without a modern extension falls through to null", () => {
    expect(officeFormatOf("old.doc", "application/msword")).toBeNull()
    expect(officeFormatOf("old.ppt", "application/vnd.ms-powerpoint")).toBeNull()
  })

  test("case-insensitive extension and MIME", () => {
    expect(officeFormatOf("REPORT.DOCX")).toBe("docx")
    expect(officeFormatOf("a.PptX")).toBe("pptx")
  })

  test("paths and unrelated files", () => {
    expect(officeFormatOf("/abs/dir/sheet.xlsx")).toBe("xlsx")
    expect(officeFormatOf("notes.txt")).toBeNull()
    expect(officeFormatOf("archive.zip")).toBeNull()
    expect(officeFormatOf("noext")).toBeNull()
  })
})
