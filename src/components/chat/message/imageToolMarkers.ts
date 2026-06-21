import type { ToolCall } from "@/types/chat"

const IMAGE_FILE_PREFIX = "__IMAGE_FILE__"

export interface ImageToolMarker {
  path: string
  mime: string
  name: string
}

// Preview-only parser for `image` tool results. It assumes the `__IMAGE_FILE__`
// markers were produced by ha-core after writing a managed preview file. If that
// contract changes, move these previews to structured metadata or mirror the
// backend managed-path whitelist here instead of broadening this parser.
function basename(path: string): string {
  const normalized = path.replace(/\\/g, "/")
  return normalized.split("/").filter(Boolean).pop() || "image"
}

export function extractImageToolMarkers(result: string | undefined): ImageToolMarker[] {
  if (!result) return []

  const markers: ImageToolMarker[] = []
  let searchFrom = 0

  while (searchFrom < result.length) {
    const markerStart = result.indexOf(IMAGE_FILE_PREFIX, searchFrom)
    if (markerStart < 0) break

    const jsonStart = markerStart + IMAGE_FILE_PREFIX.length
    const lineEnd = result.indexOf("\n", jsonStart)
    if (lineEnd < 0) break

    try {
      const spec = JSON.parse(result.slice(jsonStart, lineEnd).trim())
      const path = typeof spec?.path === "string" ? spec.path : ""
      const mime = typeof spec?.mime === "string" ? spec.mime : ""
      if (path && mime.startsWith("image/")) {
        markers.push({ path, mime, name: basename(path) })
      }
    } catch {
      // Ignore malformed markers; the raw result remains available when expanded.
    }

    searchFrom = lineEnd + 1
  }

  return markers
}

export function toolHasImagePreviewMarkers(tool: ToolCall): boolean {
  return tool.name === "image" && extractImageToolMarkers(tool.result).length > 0
}
