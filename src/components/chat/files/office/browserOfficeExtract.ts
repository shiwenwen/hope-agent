import type { ExtractedContent } from "@/lib/transport"
import { officeFormatOf } from "./officeFormat"

const MAX_EXTRACTED_TEXT_CHARS = 200_000

function finishText(value: string): string | null {
  const normalized = value
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
  if (!normalized) return null
  if (normalized.length <= MAX_EXTRACTED_TEXT_CHARS) return normalized
  return `${normalized.slice(0, MAX_EXTRACTED_TEXT_CHARS)}…\n[Content truncated]`
}

interface DocxNode {
  type?: string
  text?: string
  children?: DocxNode[]
}

function appendDocxNode(node: DocxNode | null | undefined, output: string[]): void {
  if (!node) return
  if ((node.type === "text" || node.type === "deletedText") && node.text) {
    output.push(node.text)
  } else if (node.type === "tab") {
    output.push("\t")
  } else if (node.type === "break") {
    output.push("\n")
  }
  for (const child of node.children ?? []) appendDocxNode(child, output)
  if (node.type === "cell") output.push("\t")
  else if (node.type === "paragraph" || node.type === "row") output.push("\n")
}

async function extractDocx(file: File): Promise<string | null> {
  const { parseAsync } = await import("docx-preview")
  const document = (await parseAsync(file)) as { documentPart?: { body?: DocxNode } }
  const output: string[] = []
  appendDocxNode(document.documentPart?.body, output)
  return finishText(output.join(""))
}

async function extractWorkbook(data: ArrayBuffer): Promise<string | null> {
  const XLSX = await import("xlsx")
  const workbook = XLSX.read(new Uint8Array(data), { type: "array" })
  const sheets: string[] = []
  for (const name of workbook.SheetNames) {
    const sheet = workbook.Sheets[name]
    if (!sheet) continue
    const csv = XLSX.utils.sheet_to_csv(sheet).trim()
    if (csv) sheets.push(`## ${name}\n${csv}`)
  }
  return finishText(sheets.join("\n\n"))
}

interface PptTextBody {
  paragraphs?: Array<{ runs?: Array<{ text?: string }> }>
}

function textBodyText(body: PptTextBody): string | null {
  const paragraphs = (body.paragraphs ?? [])
    .map((paragraph) => (paragraph.runs ?? []).map((run) => run.text ?? "").join(""))
    .filter(Boolean)
  return paragraphs.length > 0 ? paragraphs.join("\n") : null
}

function collectPptText(value: unknown, seen: WeakSet<object>, output: string[]): void {
  if (!value || typeof value !== "object") return
  if (seen.has(value)) return
  seen.add(value)
  if (Array.isArray(value)) {
    for (const item of value) collectPptText(item, seen, output)
    return
  }
  const object = value as Record<string, unknown>
  if (object.textBody && typeof object.textBody === "object") {
    const text = textBodyText(object.textBody as PptTextBody)
    if (text) output.push(text)
  }
  for (const key of ["commonSlideData", "shapeTree", "children", "rows", "cells"]) {
    collectPptText(object[key], seen, output)
  }
}

async function extractPresentation(data: ArrayBuffer): Promise<string | null> {
  const { PPTXViewer } = await import("pptxviewjs")
  const viewer = new PPTXViewer({ slideSizeMode: "fit" })
  try {
    await viewer.loadFile(data)
    const presentation = (viewer as unknown as { presentation?: { slides?: unknown[] } })
      .presentation
    const slides: string[] = []
    for (const [index, slide] of (presentation?.slides ?? []).entries()) {
      const content: string[] = []
      collectPptText(slide, new WeakSet<object>(), content)
      if (content.length > 0) slides.push(`## Slide ${index + 1}\n${content.join("\n")}`)
    }
    return finishText(slides.join("\n\n"))
  } finally {
    viewer.destroy()
  }
}

/**
 * Text fallback for a client-local draft. It never uploads the draft: modern
 * Office formats are parsed from the browser File/ArrayBuffer in memory.
 */
export async function extractOfficeFileInBrowser(
  file: File,
  maxDocumentPreviewBytes: number,
): Promise<ExtractedContent> {
  if (file.size > maxDocumentPreviewBytes) {
    throw new Error(
      `file too large to preview: ${file.size} bytes (max ${maxDocumentPreviewBytes} bytes)`,
    )
  }
  const format = officeFormatOf(file.name, file.type)
  if (!format) throw new Error("this legacy Office format does not support in-browser extraction")
  const data = format === "docx" ? null : await file.arrayBuffer()
  const text =
    format === "docx"
      ? await extractDocx(file)
      : format === "xlsx"
        ? await extractWorkbook(data!)
        : await extractPresentation(data!)
  return { relPath: file.name, kind: "office", text, images: [] }
}
