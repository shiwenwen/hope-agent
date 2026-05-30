/**
 * Read-only preview for a workspace file, dispatched by file kind:
 * code/text (Shiki via MarkdownRenderer), markdown (rendered + view-source),
 * image (`<img>`), PDF (`<iframe>`), Office (extracted text + images), and a
 * binary placeholder for everything else.
 *
 * Selecting text in a code/text/markdown preview reveals a "quote to chat"
 * action that captures the file path + line range + content.
 */

import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Quote, Code2, FileText as FileTextIcon, X } from "lucide-react"
import { toast } from "sonner"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { fileKind, shikiLang } from "@/lib/fileKind"
import { cn } from "@/lib/utils"
import type { ExtractedContent, FileTextContent, WorkspaceEntry } from "@/lib/transport"
import type { ProjectFsApi } from "../hooks/useProjectFs"
import { BinaryPlaceholder } from "./BinaryPlaceholder"

export interface QuotePayload {
  path: string
  name: string
  startLine: number
  endLine: number
  content: string
}

type Loaded =
  | { kind: "code" | "text" | "markdown" | "binary"; data: FileTextContent }
  | { kind: "image" | "pdf"; url: string | null }
  | { kind: "office"; data: ExtractedContent }

export interface FilePreviewPaneProps {
  fs: ProjectFsApi
  entry: WorkspaceEntry | null
  onClose?: () => void
  onQuote?: (payload: QuotePayload) => void
  className?: string
}

export function FilePreviewPane({ fs, entry, onClose, onQuote, className }: FilePreviewPaneProps) {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [viewSource, setViewSource] = useState(false)
  const [selection, setSelection] = useState<string | null>(null)
  const bodyRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let cancelled = false
    setSelection(null)
    setViewSource(false)
    if (!entry) {
      setLoaded(null)
      return
    }
    setLoading(true)
    setError(null)
    const kind = fileKind(entry.name)
    void (async () => {
      try {
        if (kind === "image" || kind === "pdf") {
          const url = await fs.rawUrl(entry.relPath, false)
          if (!cancelled) setLoaded({ kind, url })
        } else if (kind === "office") {
          const data = await fs.extractDoc(entry.relPath)
          if (!cancelled) setLoaded({ kind: "office", data })
        } else {
          const data = await fs.readFile(entry.relPath)
          if (cancelled) return
          setLoaded({ kind: data.isBinary ? "binary" : kind, data })
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [entry, fs])

  // Track text selection inside the preview body for the quote action.
  const onMouseUp = useCallback(() => {
    const sel = window.getSelection()?.toString() ?? ""
    setSelection(sel.trim().length > 0 ? sel : null)
  }, [])

  const handleQuote = useCallback(() => {
    if (!entry || !onQuote || !selection) return
    const content =
      loaded && (loaded.kind === "code" || loaded.kind === "text" || loaded.kind === "markdown")
        ? loaded.data.content
        : ""
    const { startLine, endLine } = locateLineRange(content, selection)
    onQuote({
      path: entry.relPath,
      name: entry.name,
      startLine,
      endLine,
      content: selection,
    })
    toast.success(t("fileBrowser.quoted", "Added to chat"))
    setSelection(null)
    window.getSelection()?.removeAllRanges()
  }, [entry, onQuote, selection, loaded, t])

  if (!entry) {
    return (
      <div className={cn("flex h-full items-center justify-center px-6 text-center", className)}>
        <span className="text-sm text-muted-foreground">
          {t("fileBrowser.selectFile", "Select a file to preview")}
        </span>
      </div>
    )
  }

  return (
    <div className={cn("flex h-full flex-col", className)}>
      <div className="flex items-center gap-1.5 border-b px-3 py-1.5">
        <span className="truncate text-sm font-medium">{entry.name}</span>
        <div className="ml-auto flex items-center gap-0.5">
          {loaded?.kind === "markdown" ? (
            <IconTip label={viewSource ? t("fileBrowser.rendered", "Rendered") : t("fileBrowser.viewSource", "View source")}>
              <Button size="icon" variant="ghost" className="h-6 w-6" onClick={() => setViewSource((v) => !v)}>
                {viewSource ? <FileTextIcon className="h-3.5 w-3.5" /> : <Code2 className="h-3.5 w-3.5" />}
              </Button>
            </IconTip>
          ) : null}
          {onQuote && selection ? (
            <IconTip label={t("fileBrowser.quoteToChat", "Quote to chat")}>
              <Button size="icon" variant="ghost" className="h-6 w-6" onClick={handleQuote}>
                <Quote className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          ) : null}
          {onClose ? (
            <IconTip label={t("common.close", "Close")}>
              <Button size="icon" variant="ghost" className="h-6 w-6" onClick={onClose}>
                <X className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          ) : null}
        </div>
      </div>

      <div ref={bodyRef} className="min-h-0 flex-1 overflow-auto" onMouseUp={onMouseUp}>
        {loading ? (
          <div className="flex h-full items-center justify-center text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        ) : error ? (
          <div className="px-4 py-3 text-sm text-destructive">{error}</div>
        ) : (
          <PreviewBody loaded={loaded} entry={entry} viewSource={viewSource} fs={fs} />
        )}
      </div>
    </div>
  )
}

function PreviewBody({
  loaded,
  entry,
  viewSource,
  fs,
}: {
  loaded: Loaded | null
  entry: WorkspaceEntry
  viewSource: boolean
  fs: ProjectFsApi
}) {
  const { t } = useTranslation()
  if (!loaded) return null

  if (loaded.kind === "image") {
    return loaded.url ? (
      <div className="flex h-full items-center justify-center bg-[repeating-conic-gradient(theme(colors.muted.DEFAULT)_0%_25%,transparent_0%_50%)] bg-[length:20px_20px] p-4">
        <img src={loaded.url} alt={entry.name} className="max-h-full max-w-full object-contain" />
      </div>
    ) : (
      <BinaryPlaceholder name={entry.name} sizeBytes={entry.size ?? 0} />
    )
  }

  if (loaded.kind === "pdf") {
    return loaded.url ? (
      <iframe title={entry.name} src={loaded.url} className="h-full w-full border-0" />
    ) : (
      <BinaryPlaceholder name={entry.name} sizeBytes={entry.size ?? 0} />
    )
  }

  if (loaded.kind === "office") {
    const { text, images } = loaded.data
    return (
      <div className="space-y-4 px-4 py-3">
        <div className="rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
          {t("fileBrowser.extractedPreview", "Extracted content preview — open in system for the original layout.")}
        </div>
        {images.map((img, i) => (
          <img
            key={i}
            src={`data:${img.mime};base64,${img.data}`}
            alt={img.label}
            className="mx-auto max-w-full rounded border"
          />
        ))}
        {text ? <MarkdownRenderer content={text} /> : null}
        {!text && images.length === 0 ? (
          <div className="text-sm text-muted-foreground">{t("fileBrowser.empty", "No content")}</div>
        ) : null}
      </div>
    )
  }

  if (loaded.kind === "binary") {
    return (
      <BinaryPlaceholder
        name={entry.name}
        sizeBytes={loaded.data.sizeBytes}
        note={loaded.data.truncated ? t("fileBrowser.fileTooLarge", "File too large to preview") : undefined}
        onOpen={() => void fs.rawUrl(entry.relPath, false).then((u) => u && window.open(u, "_blank"))}
        onDownload={() => void fs.rawUrl(entry.relPath, true).then((u) => u && window.open(u, "_blank"))}
      />
    )
  }

  // markdown (rendered) — view-source falls through to the fenced-code path.
  if (loaded.kind === "markdown" && !viewSource) {
    return (
      <div className="px-4 py-3">
        <MarkdownRenderer content={loaded.data.content} />
      </div>
    )
  }

  // code / text / markdown-source: render as a fenced code block for Shiki.
  if (loaded.kind === "code" || loaded.kind === "text" || loaded.kind === "markdown") {
    const lang = shikiLang(entry.name)
    const fenced = `\`\`\`${lang}\n${loaded.data.content}\n\`\`\``
    return (
      <div className="px-4 py-3 text-sm">
        <MarkdownRenderer content={fenced} />
      </div>
    )
  }

  return null
}

/** Best-effort 1-based line range of `selection` within `content`. */
function locateLineRange(content: string, selection: string): { startLine: number; endLine: number } {
  if (!content || !selection) return { startLine: 1, endLine: 1 }
  const idx = content.indexOf(selection)
  if (idx < 0) {
    // Fall back to matching the first selected line.
    const firstLine = selection.split("\n")[0]
    const lineIdx = content.split("\n").findIndex((l) => l.includes(firstLine))
    const start = lineIdx >= 0 ? lineIdx + 1 : 1
    return { startLine: start, endLine: start + selection.split("\n").length - 1 }
  }
  const before = content.slice(0, idx)
  const startLine = before.split("\n").length
  const endLine = startLine + selection.split("\n").length - 1
  return { startLine, endLine }
}
