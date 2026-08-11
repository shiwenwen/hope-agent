/**
 * Expand provenance-bearing file selections into the chat-attachment payload.
 *
 * Called at send-time by `useChatStream`. The user message text is preserved
 * verbatim, while only bindings created by a structured picker gesture are
 * appended to `attachments[]`. A pasted or manually typed lookalike `@path`
 * remains ordinary text and cannot authorize a file read.
 *
 * v1 only attaches files; directory mentions stay as text hints so the LLM
 * can call its own `list_dir` tool.
 */

import { joinAbs } from "./types"
import { logger } from "@/lib/logger"
import type { ChatAttachment } from "@/lib/transport"
import type { ComposerMentionBinding } from "../mentions/typedMentions"

/**
 * Mention-derived attachment. Always carries an absolute `file_path`
 * (mentions never inline base64 the way pasted images do).
 */
export interface MentionAttachment extends ChatAttachment {
  file_path: string
}

/**
 * Naive extension → mime-type table. The server side already does its own
 * sniffing so this is mostly a hint for the chat history UI; misclassifying
 * here is non-fatal.
 */
const EXT_MIME: Record<string, string> = {
  md: "text/markdown",
  markdown: "text/markdown",
  txt: "text/plain",
  rs: "text/plain",
  ts: "text/plain",
  tsx: "text/plain",
  js: "text/plain",
  jsx: "text/plain",
  json: "application/json",
  toml: "text/plain",
  yaml: "text/plain",
  yml: "text/plain",
  html: "text/html",
  css: "text/css",
  py: "text/plain",
  go: "text/plain",
  java: "text/plain",
  c: "text/plain",
  cpp: "text/plain",
  h: "text/plain",
  hpp: "text/plain",
  sh: "text/plain",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  pdf: "application/pdf",
}

function inferMimeType(name: string): string {
  const dot = name.lastIndexOf(".")
  if (dot < 0) return "text/plain"
  const ext = name.slice(dot + 1).toLowerCase()
  return EXT_MIME[ext] ?? "application/octet-stream"
}

function isDirectoryRef(rel: string): boolean {
  return rel.endsWith("/")
}

/**
 * Walk the typed sidecar, build attachment entries for every valid file binding.
 * Duplicates are deduped by absolute path so repeating `@foo.md @foo.md` only
 * attaches once.
 *
 * @param input  Raw user text (mentions still embedded as `@...`).
 * @param workingDir Absolute working directory for the session, or null.
 */
export function expandMentionsToAttachments(
  input: string,
  workingDir: string | null,
  bindings: ComposerMentionBinding[],
): MentionAttachment[] {
  if (!workingDir) return []
  const seen = new Set<string>()
  const out: MentionAttachment[] = []
  for (const binding of bindings) {
    if (binding.kind !== "file") continue
    if (input.slice(binding.start, binding.end) !== binding.raw) continue
    const relPath = binding.targetId
    if (isDirectoryRef(relPath)) continue
    if (!relPath) continue
    const abs = joinAbs(workingDir, relPath)
    if (seen.has(abs)) continue
    seen.add(abs)
    const baseName = relPath.split("/").filter(Boolean).pop() ?? relPath
    out.push({
      name: baseName,
      mime_type: inferMimeType(baseName),
      source: "mention",
      file_path: abs,
    })
  }
  if (out.length > 0) {
    logger.info("ui", "expandMentions", `attaching ${out.length} mention file(s) for chat`, {
      count: out.length,
    })
  }
  return out
}
