import { basename } from "@/lib/path"

export interface FileToolTarget {
  path: string
  name: string
  multiple: boolean
}

function extractStringParam(value: unknown): string | null {
  if (typeof value === "string") return value
  if (
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    (value as { type?: unknown }).type === "text"
  ) {
    const text = (value as { text?: unknown }).text
    return typeof text === "string" ? text : null
  }
  if (Array.isArray(value) && value.length > 0) {
    return extractStringParam(value[0])
  }
  return null
}

function parseArgs(args: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(args)
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}

function pathsFromPatch(input: string): string[] {
  const paths: string[] = []
  for (const rawLine of input.split(/\r?\n/)) {
    const line = rawLine.trim()
    for (const marker of ["*** Update File: ", "*** Add File: ", "*** Delete File: "]) {
      const path = line.startsWith(marker) ? line.slice(marker.length).trim() : ""
      if (path) paths.push(path)
    }
  }
  return paths
}

export function getFileToolTarget(name: string, args: string): FileToolTarget | null {
  if (name !== "write" && name !== "edit" && name !== "apply_patch") return null

  const parsed = parseArgs(args)
  if (!parsed) return null

  const directPath = extractStringParam(parsed.path ?? parsed.file_path)
  if (directPath) {
    return { path: directPath, name: basename(directPath), multiple: false }
  }

  if (name === "apply_patch") {
    const input = extractStringParam(parsed.input ?? parsed.patch)
    if (!input) return null
    const paths = pathsFromPatch(input)
    const first = paths[0]
    if (!first) return null
    return { path: first, name: basename(first), multiple: paths.length > 1 }
  }

  return null
}
