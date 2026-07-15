import type { FileTextContent } from "@/lib/transport"

export function editorText(content: string): string {
  return content.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
}

export function dominantLineEnding(
  content: string,
): Exclude<FileTextContent["lineEnding"], "mixed"> {
  const crlf = content.match(/\r\n/g)?.length ?? 0
  const rest = content.replace(/\r\n/g, "")
  const lf = rest.match(/\n/g)?.length ?? 0
  const cr = rest.match(/\r/g)?.length ?? 0
  if (crlf >= lf && crlf >= cr && crlf > 0) return "crlf"
  if (cr > lf && cr > 0) return "cr"
  return "lf"
}

export function serializeText(value: string, data: FileTextContent): string {
  const ending = data.lineEnding === "mixed" ? dominantLineEnding(data.content) : data.lineEnding
  const separator = ending === "crlf" ? "\r\n" : ending === "cr" ? "\r" : "\n"
  const normalized = value.replace(/\r\n/g, "\n").replace(/\r/g, "\n").replace(/\n/g, separator)
  return data.hasUtf8Bom ? `\uFEFF${normalized}` : normalized
}
