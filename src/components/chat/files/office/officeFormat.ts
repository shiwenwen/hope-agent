/**
 * Narrow a generic "office" file (see {@link fileKindOf}) down to the concrete
 * sub-format we can **rich-render** in the browser, or `null` when we can't —
 * the caller then falls back to the backend's plain-text extraction.
 *
 * Only the modern OOXML formats (+ legacy `.xls`, which SheetJS still reads) map
 * to a renderer; the old OLE binaries `.doc` / `.ppt` return `null` because
 * docx-preview / pptxviewjs only handle `.docx` / `.pptx`.
 */

import { extOf } from "@/lib/fileKind"

export type OfficeFormat = "docx" | "xlsx" | "pptx"

const DOCX_MIME = "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
const XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
const XLS_MIME = "application/vnd.ms-excel"
const PPTX_MIME = "application/vnd.openxmlformats-officedocument.presentationml.presentation"

/**
 * Resolve the rich-render sub-format for an office file.
 *
 * MIME (reliable on attachments) wins; otherwise the filename extension decides.
 * Legacy `application/msword` / `application/vnd.ms-powerpoint` and the `.doc` /
 * `.ppt` extensions are intentionally NOT matched → `null` → text fallback.
 */
export function officeFormatOf(name: string, mime?: string | null): OfficeFormat | null {
  const m = mime?.toLowerCase()
  if (m === DOCX_MIME) return "docx"
  if (m === XLSX_MIME || m === XLS_MIME) return "xlsx"
  if (m === PPTX_MIME) return "pptx"

  const ext = extOf(name)
  if (ext === "docx") return "docx"
  if (ext === "xlsx" || ext === "xls") return "xlsx"
  if (ext === "pptx") return "pptx"
  return null
}
