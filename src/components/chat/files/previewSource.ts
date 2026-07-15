/**
 * `PreviewSource` decouples `FilePreviewPane` from *where* a file's bytes come
 * from. The pane renders by file kind (Shiki / markdown / image / pdf / office /
 * audio / video / binary); a source just knows how to fetch text, extract a
 * document, and resolve a raw URL for one specific file.
 *
 * Three adapters cover every place a file appears in chat:
 *  - {@link projectFsPreviewSource} — file browser (workspace scope + relPath)
 *  - {@link pathPreviewSource} — an absolute path (Markdown links, workspace
 *    panel, attachment "path" entries), secured per-mode by the transport
 *  - {@link mediaPreviewSource} — a chat attachment `MediaItem` (url/localPath)
 */

import { getTransport } from "@/lib/transport-provider"
import { fileKindOf } from "@/lib/fileKind"
import { basename } from "@/lib/path"
import type { ExtractedContent, FileTextContent, WorkspaceEntry } from "@/lib/transport"
import type { Transport } from "@/lib/transport"
import type { ProjectFsApi } from "@/components/chat/project/hooks/useProjectFs"
import type { MediaItem } from "@/types/chat"
import type { FileTarget } from "./types"
import {
  DEFAULT_MAX_DOCUMENT_PREVIEW_MB,
  DEFAULT_MAX_TEXT_PREVIEW_MB,
  MEBIBYTE_BYTES,
} from "@/lib/filesystemConfig"
import { extractOfficeFileInBrowser } from "./office/browserOfficeExtract"
import { readResponseArrayBufferWithLimit } from "./readResponseWithLimit"

export interface PreviewSource {
  /** File name (drives the preview kind + Shiki language). */
  name: string
  /** MIME, when known (attachments). The pane categorizes via `fileKindOf` —
   *  the SAME function the action layer uses — so the render kind never
   *  disagrees with the click decision (e.g. a pdf attachment named without a
   *  `.pdf` extension). */
  mime?: string | null
  /** Optional Shiki language id from file-change metadata. */
  language?: string | null
  /** Path/identifier shown under the title and embedded in quote payloads. */
  displayPath?: string
  sizeBytes?: number
  /** Opt into a renderer that intentionally executes a managed, sandboxed HTML projection. */
  presentation?: "managed_html"
  /** Read text content (binary/oversized → `isBinary: true`). */
  readText: () => Promise<FileTextContent>
  /** Extract a PDF / Office document (text + images). */
  extractDoc: () => Promise<ExtractedContent>
  /** Raw URL for `<img>/<iframe>/<video>/<audio>` (or download). */
  rawUrl: (download?: boolean) => Promise<string | null>
}

/** Adapter: project file-browser scope (relPath within a workspace root). */
export function projectFsPreviewSource(fs: ProjectFsApi, entry: WorkspaceEntry): PreviewSource {
  return {
    name: entry.name,
    displayPath: entry.relPath,
    sizeBytes: entry.size ?? undefined,
    readText: () => fs.readFile(entry.relPath),
    extractDoc: () => fs.extractDoc(entry.relPath),
    rawUrl: (download) => fs.rawUrl(entry.relPath, download),
  }
}

/** Adapter: an arbitrary absolute path, authorized per-mode by the transport. */
export function pathPreviewSource(
  path: string,
  name: string,
  sessionId: string | null | undefined,
  mime?: string | null,
  language?: string | null,
  transport: Transport = getTransport(),
): PreviewSource {
  return {
    name,
    mime,
    language,
    displayPath: path,
    readText: () => transport.previewReadText(path, { sessionId }),
    extractDoc: () => transport.previewExtractDoc(path, { sessionId }),
    rawUrl: (download) => transport.previewRawUrl(path, { sessionId }, download),
  }
}

/** Adapter: a workspace target without coupling the caller to `useProjectFs`. */
export function workspacePreviewSource(
  target: Extract<FileTarget, { kind: "workspace" }>,
  transport: Transport = getTransport(),
): PreviewSource {
  const args = { scope: target.scope, scopeId: target.scopeId }
  return {
    name: target.name,
    mime: target.mime,
    language: target.language,
    displayPath: target.relPath,
    readText: () =>
      transport.call<FileTextContent>("project_fs_read_text", {
        ...args,
        path: target.relPath,
      }),
    extractDoc: () =>
      transport.call<ExtractedContent>("project_fs_extract", {
        ...args,
        path: target.relPath,
      }),
    rawUrl: (download) => transport.projectFsRawUrl({ ...args, path: target.relPath, download }),
  }
}

/** Adapter: a managed Canvas/Artifact HTML projection identified by opaque id. */
export function artifactPreviewSource(
  target: Extract<FileTarget, { kind: "artifact" }>,
  transport: Transport = getTransport(),
): PreviewSource {
  return {
    name: target.name,
    mime: "text/html",
    displayPath: target.name,
    presentation: "managed_html",
    async readText() {
      throw new Error("Artifact previews use the managed HTML viewer")
    },
    async extractDoc() {
      throw new Error("Artifact previews are not document extraction sources")
    },
    async rawUrl() {
      return transport.artifactPreviewUrl(target.artifactId, target.projectPath)
    },
  }
}

/** Adapter: a chat attachment `MediaItem` (image / audio / video / pdf / text). */
export function mediaPreviewSource(
  item: MediaItem,
  sessionId: string | null | undefined,
  transport: Transport = getTransport(),
  maxTextPreviewBytes: number = DEFAULT_MAX_TEXT_PREVIEW_MB * MEBIBYTE_BYTES,
): PreviewSource {
  const name = item.name || basename(item.localPath || item.url || "") || "file"
  return {
    name,
    mime: item.mimeType,
    displayPath: item.localPath || item.url || name,
    sizeBytes: item.sizeBytes,
    rawUrl: async () => transport.resolveMediaUrl(item),
    readText: async () => {
      // Desktop: read the local file directly (proper binary/size detection).
      if (item.localPath && transport.supportsLocalFileOps()) {
        return transport.previewReadText(item.localPath, { sessionId })
      }
      // Remote: fetch the already-authorized attachment URL as text. Guard the
      // Dynamic cap the server-side reader enforces so a huge attachment can't be
      // pulled into a string and freeze the tab.
      if (item.sizeBytes > maxTextPreviewBytes) {
        return {
          relPath: name,
          content: "",
          isBinary: true,
          mime: item.mimeType || null,
          totalLines: 0,
          sizeBytes: item.sizeBytes,
          truncated: true,
          contentHash: null,
          isUtf8: false,
          lineEnding: "lf",
          hasUtf8Bom: false,
        }
      }
      const url = transport.resolveMediaUrl(item)
      if (!url) throw new Error("attachment not reachable")
      const res = await fetch(url)
      if (!res.ok) throw new Error(`fetch attachment: ${res.status}`)
      const bytes = new Uint8Array(await readResponseArrayBufferWithLimit(res, maxTextPreviewBytes))
      const hasUtf8Bom =
        bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf
      let content = ""
      let isUtf8 = true
      try {
        content = new TextDecoder("utf-8", { fatal: true }).decode(
          hasUtf8Bom ? bytes.subarray(3) : bytes,
        )
      } catch {
        isUtf8 = false
      }
      return {
        relPath: name,
        content,
        isBinary: !isUtf8,
        mime: item.mimeType || null,
        totalLines: content.split("\n").length,
        sizeBytes: item.sizeBytes || content.length,
        truncated: false,
        contentHash: null,
        isUtf8,
        lineEnding: detectLineEnding(content),
        hasUtf8Bom,
      }
    },
    extractDoc: async () => {
      return transport.extractMediaDocument(item, { sessionId })
    },
  }
}

/** Adapter for a browser `File` that is still staged in the composer. */
export function stagedFilePreviewSource(
  file: File,
  objectUrl: string,
  maxTextPreviewBytes: number = DEFAULT_MAX_TEXT_PREVIEW_MB * MEBIBYTE_BYTES,
  maxDocumentPreviewBytes: number = DEFAULT_MAX_DOCUMENT_PREVIEW_MB * MEBIBYTE_BYTES,
): PreviewSource {
  const mime = file.type || "application/octet-stream"
  return {
    name: file.name,
    mime,
    displayPath: file.name,
    sizeBytes: file.size,
    rawUrl: async () => objectUrl,
    readText: async () => {
      const kind = fileKindOf(file.name, mime)
      const tooLarge = file.size > maxTextPreviewBytes
      const likelyBinary = kind === "other" || tooLarge
      let content = ""
      let isUtf8 = false
      let hasUtf8Bom = false
      if (!likelyBinary) {
        const bytes = new Uint8Array(await file.arrayBuffer())
        hasUtf8Bom =
          bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf
        try {
          content = new TextDecoder("utf-8", { fatal: true }).decode(
            hasUtf8Bom ? bytes.subarray(3) : bytes,
          )
          isUtf8 = true
        } catch {
          content = ""
        }
      }
      const isBinary = likelyBinary || !isUtf8
      return {
        relPath: file.name,
        content,
        isBinary,
        mime,
        totalLines: content ? content.split("\n").length : 0,
        sizeBytes: file.size,
        truncated: tooLarge,
        contentHash: null,
        isUtf8,
        lineEnding: detectLineEnding(content),
        hasUtf8Bom,
      }
    },
    extractDoc: () => extractOfficeFileInBrowser(file, maxDocumentPreviewBytes),
  }
}

function detectLineEnding(content: string): FileTextContent["lineEnding"] {
  const crlf = content.match(/\r\n/g)?.length ?? 0
  const withoutCrlf = content.replace(/\r\n/g, "")
  const lf = withoutCrlf.match(/\n/g)?.length ?? 0
  const cr = withoutCrlf.match(/\r/g)?.length ?? 0
  const kinds = [crlf, lf, cr].filter((count) => count > 0).length
  if (kinds > 1) return "mixed"
  if (crlf > 0) return "crlf"
  if (cr > 0) return "cr"
  return "lf"
}
