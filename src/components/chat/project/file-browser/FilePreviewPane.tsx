/**
 * Read-only preview for a workspace file, dispatched by file kind:
 * code/text (Shiki, direct), markdown (rendered + view-source), image
 * (`<img>`), PDF (`<iframe>`), Office (extracted text + images), and a binary
 * placeholder for everything else.
 *
 * Selecting text in a code/text/markdown-source preview reveals a "quote to
 * chat" action capturing the file path + exact line range + content.
 */

import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, X } from "lucide-react"
import { toast } from "sonner"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { fileKind, shikiLang } from "@/lib/fileKind"
import { cn } from "@/lib/utils"
import type { ExtractedContent, FileTextContent, WorkspaceEntry } from "@/lib/transport"
import type { ProjectFsApi } from "../hooks/useProjectFs"
import { BinaryPlaceholder } from "./BinaryPlaceholder"
import { ShikiCodeView, type CodeSelection } from "./ShikiCodeView"

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

  useEffect(() => {
    let cancelled = false
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

  const handleQuoteSelection = useCallback(
    (sel: CodeSelection) => {
      if (!entry || !onQuote) return
      onQuote({
        path: entry.relPath,
        name: entry.name,
        startLine: sel.startLine,
        endLine: sel.endLine,
        content: sel.text,
      })
      toast.success(t("fileBrowser.quoted", "Added to chat"))
    },
    [entry, onQuote, t],
  )

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
    <div className={cn("flex h-full min-w-0 flex-col", className)}>
      <div className="flex items-center gap-1.5 border-b px-3 py-1.5">
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium leading-tight">{entry.name}</span>
          {entry.relPath && entry.relPath !== entry.name ? (
            <span
              className="truncate font-mono text-[11px] leading-tight text-muted-foreground"
              title={entry.relPath}
            >
              {entry.relPath}
            </span>
          ) : null}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          {loaded?.kind === "markdown" ? (
            <div className="inline-flex items-center rounded-md border border-border/60 p-0.5">
              <button
                type="button"
                onClick={() => setViewSource(false)}
                className={cn(
                  "rounded px-2 py-0.5 text-xs transition-colors",
                  !viewSource
                    ? "bg-secondary text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {t("fileBrowser.rendered", "Rendered")}
              </button>
              <button
                type="button"
                onClick={() => setViewSource(true)}
                className={cn(
                  "rounded px-2 py-0.5 text-xs transition-colors",
                  viewSource
                    ? "bg-secondary text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {t("fileBrowser.viewSource", "View source")}
              </button>
            </div>
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

      <div className="min-h-0 flex-1 overflow-auto">
        {loading ? (
          <div className="flex h-full items-center justify-center text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        ) : error ? (
          <div className="px-4 py-3 text-sm text-destructive">{error}</div>
        ) : (
          <PreviewBody
            loaded={loaded}
            entry={entry}
            viewSource={viewSource}
            fs={fs}
            onQuote={onQuote ? handleQuoteSelection : undefined}
          />
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
  onQuote,
}: {
  loaded: Loaded | null
  entry: WorkspaceEntry
  viewSource: boolean
  fs: ProjectFsApi
  onQuote?: (sel: CodeSelection) => void
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

  // code / text / markdown-source: render directly with Shiki (no Markdown
  // round-trip). Selection → exact line numbers via per-line `data-line`.
  if (loaded.kind === "code" || loaded.kind === "text" || loaded.kind === "markdown") {
    return (
      <ShikiCodeView
        key={entry.relPath}
        content={loaded.data.content}
        lang={shikiLang(entry.name)}
        onQuote={onQuote}
        className="text-sm"
      />
    )
  }

  return null
}
