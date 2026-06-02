/**
 * Colorful, format-specific file icons (the vscode-icons set) shared by every
 * place a file appears — workspace panel, message attachments, file browser.
 * Icons are inlined at build time by `unplugin-icons` (offline, CSP-safe; only
 * the icons imported here are bundled). Resolution is extension-first with a
 * MIME fallback for the broad categories, reusing `extOf` so it stays aligned
 * with the rest of the file classifiers.
 */

import { createElement } from "react"
import { extOf } from "@/lib/fileKind"

import IconWord from "~icons/vscode-icons/file-type-word"
import IconExcel from "~icons/vscode-icons/file-type-excel"
import IconPowerpoint from "~icons/vscode-icons/file-type-powerpoint"
import IconPdf from "~icons/vscode-icons/file-type-pdf2"
import IconImage from "~icons/vscode-icons/file-type-image"
import IconSvg from "~icons/vscode-icons/file-type-svg"
import IconVideo from "~icons/vscode-icons/file-type-video"
import IconAudio from "~icons/vscode-icons/file-type-audio"
import IconZip from "~icons/vscode-icons/file-type-zip"
import IconMarkdown from "~icons/vscode-icons/file-type-markdown"
import IconJson from "~icons/vscode-icons/file-type-json"
import IconText from "~icons/vscode-icons/file-type-text"
import IconTypescript from "~icons/vscode-icons/file-type-typescript"
import IconReactTs from "~icons/vscode-icons/file-type-reactts"
import IconJs from "~icons/vscode-icons/file-type-js"
import IconReactJs from "~icons/vscode-icons/file-type-reactjs"
import IconRust from "~icons/vscode-icons/file-type-rust"
import IconPython from "~icons/vscode-icons/file-type-python"
import IconGo from "~icons/vscode-icons/file-type-go"
import IconJava from "~icons/vscode-icons/file-type-java"
import IconCpp from "~icons/vscode-icons/file-type-cpp"
import IconC from "~icons/vscode-icons/file-type-c"
import IconCsharp from "~icons/vscode-icons/file-type-csharp"
import IconRuby from "~icons/vscode-icons/file-type-ruby"
import IconPhp from "~icons/vscode-icons/file-type-php"
import IconSwift from "~icons/vscode-icons/file-type-swift"
import IconKotlin from "~icons/vscode-icons/file-type-kotlin"
import IconHtml from "~icons/vscode-icons/file-type-html"
import IconCss from "~icons/vscode-icons/file-type-css"
import IconScss from "~icons/vscode-icons/file-type-scss"
import IconVue from "~icons/vscode-icons/file-type-vue"
import IconSvelte from "~icons/vscode-icons/file-type-svelte"
import IconSql from "~icons/vscode-icons/file-type-sql"
import IconYaml from "~icons/vscode-icons/file-type-yaml"
import IconToml from "~icons/vscode-icons/file-type-toml"
import IconXml from "~icons/vscode-icons/file-type-xml"
import IconShell from "~icons/vscode-icons/file-type-shell"
import IconLua from "~icons/vscode-icons/file-type-lua"
import IconDefault from "~icons/vscode-icons/default-file"

type IconComponent = typeof IconDefault

/** Extension (lowercase, no dot) → colorful icon. */
const EXT_ICON: Record<string, IconComponent> = {
  // Office
  doc: IconWord,
  docx: IconWord,
  dot: IconWord,
  dotx: IconWord,
  rtf: IconWord,
  xls: IconExcel,
  xlsx: IconExcel,
  xlsm: IconExcel,
  csv: IconExcel,
  tsv: IconExcel,
  ppt: IconPowerpoint,
  pptx: IconPowerpoint,
  pdf: IconPdf,
  // Media
  png: IconImage,
  jpg: IconImage,
  jpeg: IconImage,
  gif: IconImage,
  webp: IconImage,
  bmp: IconImage,
  ico: IconImage,
  avif: IconImage,
  tiff: IconImage,
  svg: IconSvg,
  mp4: IconVideo,
  mov: IconVideo,
  webm: IconVideo,
  mkv: IconVideo,
  avi: IconVideo,
  m4v: IconVideo,
  flv: IconVideo,
  mp3: IconAudio,
  wav: IconAudio,
  flac: IconAudio,
  ogg: IconAudio,
  m4a: IconAudio,
  aac: IconAudio,
  // Archives
  zip: IconZip,
  tar: IconZip,
  gz: IconZip,
  tgz: IconZip,
  rar: IconZip,
  "7z": IconZip,
  bz2: IconZip,
  xz: IconZip,
  // Docs / data
  md: IconMarkdown,
  markdown: IconMarkdown,
  mdx: IconMarkdown,
  json: IconJson,
  jsonc: IconJson,
  txt: IconText,
  text: IconText,
  log: IconText,
  // Code
  ts: IconTypescript,
  mts: IconTypescript,
  cts: IconTypescript,
  tsx: IconReactTs,
  js: IconJs,
  mjs: IconJs,
  cjs: IconJs,
  jsx: IconReactJs,
  rs: IconRust,
  py: IconPython,
  go: IconGo,
  java: IconJava,
  cpp: IconCpp,
  cc: IconCpp,
  cxx: IconCpp,
  hpp: IconCpp,
  hh: IconCpp,
  c: IconC,
  h: IconC,
  cs: IconCsharp,
  rb: IconRuby,
  php: IconPhp,
  swift: IconSwift,
  kt: IconKotlin,
  kts: IconKotlin,
  html: IconHtml,
  htm: IconHtml,
  css: IconCss,
  scss: IconScss,
  sass: IconScss,
  less: IconScss,
  vue: IconVue,
  svelte: IconSvelte,
  sql: IconSql,
  yaml: IconYaml,
  yml: IconYaml,
  toml: IconToml,
  xml: IconXml,
  sh: IconShell,
  bash: IconShell,
  zsh: IconShell,
  fish: IconShell,
  lua: IconLua,
}

/** MIME fallback for the broad categories (extensionless / unknown-ext files). */
function iconForMime(mime: string): IconComponent | null {
  const m = mime.toLowerCase()
  if (m === "application/pdf") return IconPdf
  if (m === "image/svg+xml") return IconSvg
  if (m.startsWith("image/")) return IconImage
  if (m.startsWith("audio/")) return IconAudio
  if (m.startsWith("video/")) return IconVideo
  if (
    m === "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    m === "application/msword"
  )
    return IconWord
  if (
    m === "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" ||
    m === "application/vnd.ms-excel" ||
    m === "text/csv"
  )
    return IconExcel
  if (
    m === "application/vnd.openxmlformats-officedocument.presentationml.presentation" ||
    m === "application/vnd.ms-powerpoint"
  )
    return IconPowerpoint
  if (m === "application/json") return IconJson
  if (m.startsWith("text/")) return IconText
  return null
}

function resolveIcon(name: string, mime?: string | null): IconComponent {
  const byExt = EXT_ICON[extOf(name)]
  if (byExt) return byExt
  if (mime) {
    const byMime = iconForMime(mime)
    if (byMime) return byMime
  }
  return IconDefault
}

/** Colorful file-format icon for `name` (+ optional `mime`). Size via `className`. */
export function FileTypeIcon({
  name,
  mime,
  className,
}: {
  name: string
  mime?: string | null
  className?: string
}) {
  // `createElement` (vs `<Icon/>`) — the icon type is selected at runtime;
  // the JSX form trips react-hooks/static-components.
  return createElement(resolveIcon(name, mime), { className })
}
