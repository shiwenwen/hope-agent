/**
 * File-type inference shared by the project file browser: maps a filename to a
 * preview "kind", a Shiki language for syntax highlighting, and a lucide icon.
 */

import {
  File,
  FileText,
  FileCode,
  FileImage,
  FileArchive,
  FileVideo,
  FileAudio,
  FileSpreadsheet,
  Folder,
  FolderOpen,
  type LucideIcon,
} from "lucide-react"
import { basename } from "@/lib/path"

export type FileKind =
  | "code"
  | "markdown"
  | "image"
  | "pdf"
  | "office"
  | "text"
  | "audio"
  | "video"
  | "other"

/** Extension → Shiki language id (also doubles as the "is code" set). */
const CODE_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  jsonc: "jsonc",
  rs: "rust",
  py: "python",
  go: "go",
  java: "java",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  hpp: "cpp",
  cs: "csharp",
  rb: "ruby",
  php: "php",
  swift: "swift",
  kt: "kotlin",
  scala: "scala",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "bash",
  ps1: "powershell",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  less: "less",
  vue: "vue",
  svelte: "svelte",
  sql: "sql",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  xml: "xml",
  lua: "lua",
  r: "r",
  dart: "dart",
  ex: "elixir",
  exs: "elixir",
  clj: "clojure",
  hs: "haskell",
  ml: "ocaml",
  ini: "ini",
  graphql: "graphql",
  proto: "protobuf",
  diff: "diff",
  patch: "diff",
}

const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif"])
const OFFICE_EXT = new Set(["docx", "doc", "xlsx", "xls", "pptx", "ppt"])
const MARKDOWN_EXT = new Set(["md", "markdown", "mdx"])
const ARCHIVE_EXT = new Set(["zip", "tar", "gz", "tgz", "rar", "7z", "bz2", "xz"])
const VIDEO_EXT = new Set(["mp4", "mov", "avi", "mkv", "webm", "m4v"])
const AUDIO_EXT = new Set(["mp3", "wav", "flac", "ogg", "m4a", "aac"])

/** Plain-text / config / data extensions that aren't code but render as text. */
const TEXT_EXT = new Set([
  "txt",
  "text",
  "log",
  "csv",
  "tsv",
  "conf",
  "config",
  "env",
  "properties",
  "lock",
  "plist",
  "jsonl",
  "ndjson",
])

/** Extensionless filenames conventionally treated as plain text. */
const TEXT_FILENAMES = new Set([
  "dockerfile",
  "makefile",
  "license",
  "licence",
  "readme",
  "changelog",
  "authors",
  "notice",
  "copying",
  "gemfile",
  "rakefile",
  "procfile",
  ".gitignore",
  ".gitattributes",
  ".dockerignore",
  ".env",
  ".npmrc",
  ".editorconfig",
  ".prettierrc",
  ".eslintrc",
  ".babelrc",
])

function baseLower(name: string): string {
  // `basename` handles both `/` and Windows `\` separators (a `\`-path's
  // extensionless filename would otherwise be mis-bucketed as `other`).
  return basename(name).toLowerCase()
}

/** Lowercase extension without the dot (`"a.tar.gz"` → `"gz"`, `"README"` → `""`). */
export function extOf(name: string): string {
  const base = name.slice(name.lastIndexOf("/") + 1)
  const i = base.lastIndexOf(".")
  return i > 0 ? base.slice(i + 1).toLowerCase() : ""
}

export function fileKind(name: string): FileKind {
  const ext = extOf(name)
  if (ext === "pdf") return "pdf"
  if (IMAGE_EXT.has(ext)) return "image"
  if (OFFICE_EXT.has(ext)) return "office"
  if (MARKDOWN_EXT.has(ext)) return "markdown"
  if (AUDIO_EXT.has(ext)) return "audio"
  if (VIDEO_EXT.has(ext)) return "video"
  if (ext in CODE_LANG) return "code"
  if (TEXT_EXT.has(ext)) return "text"
  if (ext === "" && TEXT_FILENAMES.has(baseLower(name))) return "text"
  // Unknown / archive / binary — not auto-previewed; click → open (local) or
  // download (remote). The preview pane, if forced open, falls back to a
  // text attempt then a binary placeholder.
  return "other"
}

/** Kinds the in-app right-side preview panel can render. `other` is excluded. */
const PREVIEWABLE_KINDS: ReadonlySet<FileKind> = new Set<FileKind>([
  "code",
  "markdown",
  "text",
  "image",
  "pdf",
  "office",
  "audio",
  "video",
])

/** Whether a kind should click-to-preview (vs. open/download). */
export function isPreviewableKind(kind: FileKind): boolean {
  return PREVIEWABLE_KINDS.has(kind)
}

const OFFICE_MIME = new Set([
  "application/msword",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.ms-excel",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.ms-powerpoint",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation",
])

/**
 * Like {@link fileKind} but trusts a known MIME type first (attachments carry
 * a reliable `mime`), falling back to the filename extension. Use this wherever
 * a MIME is available; otherwise call {@link fileKind}.
 */
export function fileKindOf(name: string, mime?: string | null): FileKind {
  if (mime) {
    const m = mime.toLowerCase()
    if (m.startsWith("image/")) return "image"
    if (m.startsWith("audio/")) return "audio"
    if (m.startsWith("video/")) return "video"
    if (m === "application/pdf") return "pdf"
    if (OFFICE_MIME.has(m)) return "office"
    if (
      m.startsWith("text/") ||
      m === "application/json" ||
      m === "application/xml" ||
      m === "application/javascript"
    ) {
      const byExt = fileKind(name)
      return byExt === "other" ? "text" : byExt
    }
  }
  return fileKind(name)
}

/** Shiki language id for a code/text file. Falls back to plain `"text"`. */
export function shikiLang(name: string): string {
  return CODE_LANG[extOf(name)] ?? "text"
}

export function iconForEntry(name: string, isDir: boolean, expanded = false): LucideIcon {
  if (isDir) return expanded ? FolderOpen : Folder
  const ext = extOf(name)
  if (IMAGE_EXT.has(ext)) return FileImage
  if (ext === "pdf") return FileText
  if (OFFICE_EXT.has(ext)) return ext.startsWith("xls") ? FileSpreadsheet : FileText
  if (MARKDOWN_EXT.has(ext)) return FileText
  if (ext in CODE_LANG) return FileCode
  if (ARCHIVE_EXT.has(ext)) return FileArchive
  if (VIDEO_EXT.has(ext)) return FileVideo
  if (AUDIO_EXT.has(ext)) return FileAudio
  return File
}
