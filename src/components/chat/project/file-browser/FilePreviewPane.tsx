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
import { Loader2, Maximize2, Minimize2, X } from "lucide-react"
import { toast } from "sonner"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { fileKindOf, shikiLang } from "@/lib/fileKind"
import { cn } from "@/lib/utils"
import type { FileTextContent } from "@/lib/transport"
import type { PreviewSource } from "@/components/chat/files/previewSource"
import { OfficeRichPreview } from "@/components/chat/files/office/OfficeRichPreview"
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
  | { kind: "image" | "pdf" | "audio" | "video"; url: string | null }
  | { kind: "office" }

export interface FilePreviewPaneProps {
  /** The file to preview (memoize this — it drives the load effect), or `null`. */
  source: PreviewSource | null
  onClose?: () => void
  onQuote?: (payload: QuotePayload) => void
  /** Quoted line range to highlight + scroll to in the preview (from a reveal). */
  highlightLines?: { start: number; end: number; nonce: number } | null
  className?: string
  /** When `onToggleMaximize` is provided, a maximize/restore toggle is shown
   *  next to close (used by the right-side preview panel; the file-browser
   *  split view leaves it unset so no button appears). */
  maximized?: boolean
  onToggleMaximize?: () => void
}

export function FilePreviewPane({
  source,
  onClose,
  onQuote,
  highlightLines,
  className,
  maximized,
  onToggleMaximize,
}: FilePreviewPaneProps) {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [viewSource, setViewSource] = useState(false)

  useEffect(() => {
    let cancelled = false
    setViewSource(false)
    if (!source) {
      setLoaded(null)
      return
    }
    setLoading(true)
    setError(null)
    const kind = fileKindOf(source.name, source.mime)
    void (async () => {
      try {
        if (kind === "image" || kind === "pdf" || kind === "audio" || kind === "video") {
          const url = await source.rawUrl(false)
          if (!cancelled) setLoaded({ kind, url })
        } else if (kind === "office") {
          // Office files are rich-rendered from raw bytes by OfficeRichPreview;
          // text extraction runs lazily only if that falls back.
          if (!cancelled) setLoaded({ kind: "office" })
        } else {
          const data = await source.readText()
          if (cancelled) return
          // code/text/markdown render as text; `other` renders as text when
          // readable, else falls through to the binary placeholder.
          const renderKind = data.isBinary ? "binary" : kind === "other" ? "text" : kind
          setLoaded({ kind: renderKind, data })
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
  }, [source])

  const handleQuoteSelection = useCallback(
    (sel: CodeSelection) => {
      if (!source || !onQuote) return
      onQuote({
        path: source.displayPath ?? source.name,
        name: source.name,
        startLine: sel.startLine,
        endLine: sel.endLine,
        content: sel.text,
      })
      toast.success(t("fileBrowser.quoted", "Added to chat"))
    },
    [source, onQuote, t],
  )

  if (!source) {
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
          <span className="truncate text-sm font-medium leading-tight">{source.name}</span>
          {source.displayPath && source.displayPath !== source.name ? (
            <span
              className="truncate font-mono text-[11px] leading-tight text-muted-foreground"
              title={source.displayPath}
            >
              {source.displayPath}
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
          {onToggleMaximize ? (
            <IconTip
              label={
                maximized
                  ? t("fileBrowser.minimize", "Restore")
                  : t("fileBrowser.maximize", "Maximize")
              }
            >
              <Button
                size="icon"
                variant="ghost"
                className="h-6 w-6"
                onClick={onToggleMaximize}
              >
                {maximized ? (
                  <Minimize2 className="h-3.5 w-3.5" />
                ) : (
                  <Maximize2 className="h-3.5 w-3.5" />
                )}
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

      <div className="min-h-0 flex-1 overflow-auto">
        {loading ? (
          <div className="flex h-full items-center justify-center text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
          </div>
        ) : error ? (
          // Failed preview (remote Office extract, oversized, transient read
          // error) still offers open/download so the file isn't a dead end.
          <BinaryPlaceholder
            name={source.name}
            sizeBytes={source.sizeBytes ?? 0}
            note={error}
            onOpen={() => void source.rawUrl(false).then((u) => u && window.open(u, "_blank"))}
            onDownload={() => void source.rawUrl(true).then((u) => u && window.open(u, "_blank"))}
          />
        ) : (
          <PreviewBody
            loaded={loaded}
            source={source}
            viewSource={viewSource}
            onQuote={onQuote ? handleQuoteSelection : undefined}
            highlightLines={highlightLines}
          />
        )}
      </div>
    </div>
  )
}

function PreviewBody({
  loaded,
  source,
  viewSource,
  onQuote,
  highlightLines,
}: {
  loaded: Loaded | null
  source: PreviewSource
  viewSource: boolean
  onQuote?: (sel: CodeSelection) => void
  highlightLines?: { start: number; end: number; nonce: number } | null
}) {
  const { t } = useTranslation()
  if (!loaded) return null

  if (loaded.kind === "image") {
    return loaded.url ? (
      <div className="flex h-full items-center justify-center bg-[repeating-conic-gradient(theme(colors.muted.DEFAULT)_0%_25%,transparent_0%_50%)] bg-[length:20px_20px] p-4">
        <img src={loaded.url} alt={source.name} className="max-h-full max-w-full object-contain" />
      </div>
    ) : (
      <BinaryPlaceholder name={source.name} sizeBytes={source.sizeBytes ?? 0} />
    )
  }

  if (loaded.kind === "pdf") {
    return loaded.url ? (
      <iframe title={source.name} src={loaded.url} className="h-full w-full border-0" />
    ) : (
      <BinaryPlaceholder name={source.name} sizeBytes={source.sizeBytes ?? 0} />
    )
  }

  if (loaded.kind === "audio") {
    return loaded.url ? (
      <div className="flex h-full items-center justify-center p-6">
        <audio src={loaded.url} controls className="w-full max-w-xl">
          <track kind="captions" />
        </audio>
      </div>
    ) : (
      <BinaryPlaceholder name={source.name} sizeBytes={source.sizeBytes ?? 0} />
    )
  }

  if (loaded.kind === "video") {
    return loaded.url ? (
      <div className="flex h-full items-center justify-center bg-black/90 p-2">
        <video src={loaded.url} controls className="max-h-full max-w-full">
          <track kind="captions" />
        </video>
      </div>
    ) : (
      <BinaryPlaceholder name={source.name} sizeBytes={source.sizeBytes ?? 0} />
    )
  }

  if (loaded.kind === "office") {
    // key on the file so a new source remounts (resets fetch/fail state) —
    // OfficeRichPreview's effect only fetches, it never resets synchronously.
    return <OfficeRichPreview key={source.displayPath ?? source.name} source={source} />
  }

  if (loaded.kind === "binary") {
    return (
      <BinaryPlaceholder
        name={source.name}
        sizeBytes={loaded.data.sizeBytes}
        note={loaded.data.truncated ? t("fileBrowser.fileTooLarge", "File too large to preview") : undefined}
        onOpen={() => void source.rawUrl(false).then((u) => u && window.open(u, "_blank"))}
        onDownload={() => void source.rawUrl(true).then((u) => u && window.open(u, "_blank"))}
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
        key={source.displayPath ?? source.name}
        content={loaded.data.content}
        lang={shikiLang(source.name)}
        onQuote={onQuote}
        highlightLines={highlightLines}
        className="text-sm"
      />
    )
  }

  return null
}
