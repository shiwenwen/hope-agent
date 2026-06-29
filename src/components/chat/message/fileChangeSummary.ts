import { diffLines } from "diff"

import type { FileChangeMetadata, FileChangesMetadata, ToolCall } from "@/types/chat"

export interface FileChangeSummary {
  linesAdded: number
  linesRemoved: number
  estimated: boolean
  payload?: FileChangeMetadata | FileChangesMetadata
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

function countDiffLines(value: string): number {
  if (!value) return 0
  const parts = value.split("\n")
  return value.endsWith("\n") ? parts.length - 1 : parts.length
}

function computeLineDelta(
  before: string,
  after: string,
): Pick<FileChangeSummary, "linesAdded" | "linesRemoved"> {
  let linesAdded = 0
  let linesRemoved = 0
  for (const part of diffLines(before, after)) {
    const count = countDiffLines(part.value)
    if (part.added) linesAdded += count
    else if (part.removed) linesRemoved += count
  }
  return { linesAdded, linesRemoved }
}

function fromMetadata(tool: ToolCall): FileChangeSummary | null {
  const meta = tool.metadata
  if (!meta) return null
  if (meta.kind === "file_change") {
    return {
      linesAdded: meta.linesAdded,
      linesRemoved: meta.linesRemoved,
      estimated: false,
      payload: meta,
    }
  }
  if (meta.kind === "file_changes") {
    const totals = meta.changes.reduce(
      (acc, c) => {
        acc.linesAdded += c.linesAdded
        acc.linesRemoved += c.linesRemoved
        return acc
      },
      { linesAdded: 0, linesRemoved: 0 },
    )
    return { ...totals, estimated: false, payload: meta }
  }
  return null
}

function estimateEdit(args: Record<string, unknown>): FileChangeSummary | null {
  const oldText = extractStringParam(args.old_text ?? args.oldText ?? args.old_string)
  const newText = extractStringParam(args.new_text ?? args.newText ?? args.new_string) ?? ""
  if (oldText == null) return null
  const delta = computeLineDelta(oldText, newText)
  if (delta.linesAdded === 0 && delta.linesRemoved === 0) return null
  return { ...delta, estimated: true }
}

function estimateApplyPatch(input: string): FileChangeSummary | null {
  let linesAdded = 0
  let linesRemoved = 0
  let mode: "add" | "update" | null = null

  for (const rawLine of input.split(/\r?\n/)) {
    const trimmed = rawLine.trim()

    if (trimmed === "*** Begin Patch" || trimmed === "*** End Patch") {
      mode = null
      continue
    }
    if (trimmed.startsWith("*** Delete File: ")) {
      // The removed line count depends on the current file content; wait for the
      // backend's real metadata instead of showing a misleading lower bound.
      return null
    }
    if (trimmed.startsWith("*** Add File: ")) {
      mode = "add"
      continue
    }
    if (trimmed.startsWith("*** Update File: ")) {
      mode = "update"
      continue
    }
    if (trimmed === "*** End of File" || trimmed.startsWith("*** Move to: ")) {
      continue
    }
    if (trimmed.startsWith("*** ")) {
      mode = null
      continue
    }

    if (mode === "add") {
      if (rawLine.startsWith("+")) linesAdded += 1
      continue
    }
    if (mode === "update") {
      if (rawLine.startsWith("+")) linesAdded += 1
      else if (rawLine.startsWith("-")) linesRemoved += 1
    }
  }

  if (linesAdded === 0 && linesRemoved === 0) return null
  return { linesAdded, linesRemoved, estimated: true }
}

function estimateFromArguments(tool: ToolCall): FileChangeSummary | null {
  const args = parseArgs(tool.arguments)
  if (!args) return null

  if (tool.name === "edit") return estimateEdit(args)
  if (tool.name === "apply_patch") {
    const input = extractStringParam(args.input ?? args.patch)
    return input ? estimateApplyPatch(input) : null
  }

  return null
}

export function getFileChangeSummary(tool: ToolCall): FileChangeSummary | null {
  const metadataSummary = fromMetadata(tool)
  if (metadataSummary) return metadataSummary

  // Argument-based deltas are only in-flight previews. Completed tools without
  // backend metadata may have failed or no-op'd, so do not render them as edits.
  if (tool.result !== undefined) return null

  return estimateFromArguments(tool)
}
