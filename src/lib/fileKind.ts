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

export type FileKind = "code" | "markdown" | "image" | "pdf" | "office" | "text"

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
  if (ext in CODE_LANG) return "code"
  return "text"
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
