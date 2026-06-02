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
import { basename } from "@/lib/path"
import type { ExtractedContent, FileTextContent, WorkspaceEntry } from "@/lib/transport"
import type { ProjectFsApi } from "@/components/chat/project/hooks/useProjectFs"
import type { MediaItem } from "@/types/chat"

export interface PreviewSource {
  /** File name (drives the preview kind + Shiki language). */
  name: string
  /** MIME, when known (attachments). The pane categorizes via `fileKindOf(name,
   *  mime)` — the SAME function the action layer uses — so the render kind never
   *  disagrees with the click decision (e.g. a pdf attachment named without a
   *  `.pdf` extension). */
  mime?: string | null
  /** Path/identifier shown under the title and embedded in quote payloads. */
  displayPath?: string
  sizeBytes?: number
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
): PreviewSource {
  const transport = getTransport()
  return {
    name,
    mime,
    displayPath: path,
    readText: () => transport.previewReadText(path, { sessionId }),
    extractDoc: () => transport.previewExtractDoc(path, { sessionId }),
    rawUrl: (download) => transport.previewRawUrl(path, { sessionId }, download),
  }
}

/** Adapter: a chat attachment `MediaItem` (image / audio / video / pdf / text). */
export function mediaPreviewSource(
  item: MediaItem,
  sessionId: string | null | undefined,
): PreviewSource {
  const transport = getTransport()
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
      // 5MB cap the server-side reader enforces so a huge attachment can't be
      // pulled into a string and freeze the tab.
      const MAX_TEXT_PREVIEW_BYTES = 5 * 1024 * 1024
      if (item.sizeBytes > MAX_TEXT_PREVIEW_BYTES) {
        return {
          relPath: name,
          content: "",
          isBinary: true,
          mime: item.mimeType || null,
          totalLines: 0,
          sizeBytes: item.sizeBytes,
          truncated: true,
        }
      }
      const url = transport.resolveMediaUrl(item)
      if (!url) throw new Error("attachment not reachable")
      const res = await fetch(url)
      if (!res.ok) throw new Error(`fetch attachment: ${res.status}`)
      const content = await res.text()
      return {
        relPath: name,
        content,
        isBinary: false,
        mime: item.mimeType || null,
        totalLines: content.split("\n").length,
        sizeBytes: item.sizeBytes || content.length,
        truncated: false,
      }
    },
    extractDoc: async () => {
      // Document extraction needs a server-side path; only available on desktop
      // for media (the local file). Remote attachments fall back to download.
      if (item.localPath && transport.supportsLocalFileOps()) {
        return transport.previewExtractDoc(item.localPath, { sessionId })
      }
      throw new Error("document preview is not available for this attachment")
    },
  }
}
