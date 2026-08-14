/**
 * Read-only preview for a workspace file, dispatched by file kind:
 * code/text (Shiki, direct), markdown and HTML (rendered + view-source), image
 * (`<img>`), PDF (`<iframe>`), Office (extracted text + images), and a binary
 * placeholder for everything else. Ordinary HTML renders in a script-free
 * sandbox; managed HTML artifacts use their dedicated script-enabled path.
 *
 * Selecting text in a code/text, rendered/source Markdown, or selectable Office
 * preview automatically reveals copy and optional "quote to chat" actions.
 * Source previews retain exact line ranges; rendered surfaces report lines only
 * when they can map the literal selection without ambiguity.
 */

import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Download, ExternalLink, Loader2, Maximize2, Minimize2, Pencil, X } from "lucide-react"
import { toast } from "sonner"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { fileKindOf, shikiLang } from "@/lib/fileKind"
import { cn } from "@/lib/utils"
import type { FileTextContent } from "@/lib/transport"
import type { PreviewSource } from "@/components/chat/files/previewSource"
import type { PendingFileQuote } from "@/types/chat"
import { OfficeRichPreview } from "@/components/chat/files/office/OfficeRichPreview"
import { ArtifactSelectionIframe } from "@/components/artifacts/ArtifactViewer"
import { BinaryPlaceholder } from "./BinaryPlaceholder"
import { buildOfflineHtmlPreview } from "./offlineHtmlPreview"
import { SelectableDomPreview } from "./SelectableDomPreview"
import { ShikiCodeView, type CodeSelection } from "./ShikiCodeView"

export type QuotePayload = PendingFileQuote

type Loaded =
  | { kind: "code" | "text" | "markdown" | "binary"; data: FileTextContent }
  | { kind: "image" | "pdf" | "audio" | "video" | "managed_html"; url: string | null }
  | { kind: "office" }

export interface FilePreviewPaneProps {
  /** The file to preview (memoize this — it drives the load effect), or `null`. */
  source: PreviewSource | null
  onClose?: () => void
  /** Open the current file with the system/browser default handler. */
  onOpen?: () => void
  onDownload?: () => void
  onEdit?: () => void
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
  onOpen,
  onDownload,
  onEdit,
  onQuote,
  highlightLines,
  className,
  maximized,
  onToggleMaximize,
}: FilePreviewPaneProps) {
  const { t } = useTranslation()
  const [loadedResult, setLoadedResult] = useState<{
    source: PreviewSource
    value: Loaded
  } | null>(null)
  // Async resolution for the previous file must never be rendered under the
  // next file's header/quote provenance while its new load effect catches up.
  const loaded = loadedResult?.source === source ? loadedResult.value : null
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [viewSource, setViewSource] = useState(false)

  useEffect(() => {
    let cancelled = false
    let leasedRawUrl: string | null = null
    if (!source) {
      setViewSource(false)
      setLoadedResult(null)
      return
    }
    setLoading(true)
    setError(null)
    const kind = fileKindOf(source.name, source.mime, source.language)
    // Preserve the existing highlighted-source default for ordinary HTML.
    // Managed HTML artifacts follow their dedicated renderer below.
    setViewSource(
      source.presentation !== "managed_html" &&
        kind === "code" &&
        shikiLang(source.name, source.language) === "html",
    )
    void (async () => {
      try {
        if (source.presentation === "managed_html") {
          const url = await source.rawUrl(false)
          if (url && cancelled) {
            source.releaseRawUrl?.(url)
            return
          }
          if (url) leasedRawUrl = url
          if (!cancelled) setLoadedResult({ source, value: { kind: "managed_html", url } })
        } else if (kind === "image" || kind === "pdf" || kind === "audio" || kind === "video") {
          const url = await source.rawUrl(false)
          if (url && cancelled) {
            source.releaseRawUrl?.(url)
            return
          }
          if (url) leasedRawUrl = url
          if (!cancelled) setLoadedResult({ source, value: { kind, url } })
        } else if (kind === "office") {
          // Office files are rich-rendered from raw bytes by OfficeRichPreview;
          // text extraction runs lazily only if that falls back.
          if (!cancelled) setLoadedResult({ source, value: { kind: "office" } })
        } else {
          const data = await source.readText()
          if (cancelled) return
          // code/text/markdown render as text; `other` renders as text when
          // readable, else falls through to the binary placeholder.
          const renderKind = data.isBinary ? "binary" : kind === "other" ? "text" : kind
          setLoadedResult({ source, value: { kind: renderKind, data } })
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e))
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
      if (leasedRawUrl) source.releaseRawUrl?.(leasedRawUrl)
    }
  }, [source])

  const isHtmlPreview =
    loaded?.kind === "code" && shikiLang(source?.name ?? "", source?.language) === "html"
  const supportsRenderedView = loaded?.kind === "markdown" || isHtmlPreview
  const effectiveHighlightLines =
    highlightLines && highlightLines.start > 0 && highlightLines.end > 0 ? highlightLines : null
  const effectiveViewSource =
    viewSource ||
    (!!effectiveHighlightLines && (loaded?.kind === "markdown" || isHtmlPreview))

  const handleQuoteSelection = useCallback(
    (sel: CodeSelection) => {
      if (!source || !onQuote) return
      onQuote({
        path: source.displayPath ?? source.name,
        name: source.name,
        startLine: sel.startLine,
        endLine: sel.endLine,
        content: sel.text,
        ...(source.presentation === "managed_html" ? { revealable: false } : {}),
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
              data-ha-title-tip={source.displayPath}
            >
              {source.displayPath}
            </span>
          ) : null}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          {supportsRenderedView ? (
            <div className="inline-flex items-center rounded-md border border-border/60 p-0.5">
              <button
                type="button"
                onClick={() => setViewSource(false)}
                className={cn(
                  "rounded px-2 py-0.5 text-xs transition-colors",
                  !effectiveViewSource
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
                  effectiveViewSource
                    ? "bg-secondary text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {t("fileBrowser.viewSource", "View source")}
              </button>
            </div>
          ) : null}
          {onOpen ? (
            <IconTip label={t("fileActions.open", "Open")}>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label={t("fileActions.open", "Open")}
                className="h-6 w-6"
                onClick={onOpen}
              >
                <ExternalLink className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          ) : null}
          {onDownload ? (
            <IconTip label={t("fileActions.download", "Download")}>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label={t("fileActions.download", "Download")}
                className="h-6 w-6"
                onClick={onDownload}
              >
                <Download className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          ) : null}
          {onEdit ? (
            <IconTip label={t("fileActions.edit", "Edit")}>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label={t("fileActions.edit", "Edit")}
                className="h-6 w-6"
                onClick={onEdit}
              >
                <Pencil className="h-3.5 w-3.5" />
              </Button>
            </IconTip>
          ) : null}
          {onToggleMaximize ? (
            <IconTip
              label={
                maximized
                  ? t("fileBrowser.minimize", "Restore")
                  : t("fileBrowser.maximize", "Maximize")
              }
            >
              <Button size="icon" variant="ghost" className="h-6 w-6" onClick={onToggleMaximize}>
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
            onOpen={onOpen}
            onDownload={onDownload}
          />
        ) : (
          <PreviewBody
            loaded={loaded}
            source={source}
            viewSource={effectiveViewSource}
            onQuote={onQuote ? handleQuoteSelection : undefined}
            highlightLines={effectiveHighlightLines}
            onOpen={onOpen}
            onDownload={onDownload}
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
  onOpen,
  onDownload,
}: {
  loaded: Loaded | null
  source: PreviewSource
  viewSource: boolean
  onQuote?: (sel: CodeSelection) => void
  highlightLines?: { start: number; end: number; nonce: number } | null
  onOpen?: () => void
  onDownload?: () => void
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

  if (loaded.kind === "managed_html") {
    return loaded.url ? (
      <ArtifactSelectionIframe
        title={source.name}
        src={loaded.url}
        refreshKey={source.displayPath ?? source.name}
        onQuoteSelection={
          onQuote
            ? ({ text }) => {
                onQuote({ startLine: 0, endLine: 0, text })
              }
            : undefined
        }
        className="h-full w-full border-0 bg-white dark:bg-surface-app"
      />
    ) : (
      <BinaryPlaceholder name={source.name} sizeBytes={source.sizeBytes ?? 0} />
    )
  }

  if (loaded.kind === "office") {
    // key on the file so a new source remounts (resets fetch/fail state) —
    // OfficeRichPreview's effect only fetches, it never resets synchronously.
    return (
      <SelectableDomPreview
        key={source.displayPath ?? source.name}
        onQuote={onQuote}
        className="h-full"
      >
        <OfficeRichPreview source={source} onOpen={onOpen} onDownload={onDownload} />
      </SelectableDomPreview>
    )
  }

  if (loaded.kind === "binary") {
    return (
      <BinaryPlaceholder
        name={source.name}
        sizeBytes={loaded.data.sizeBytes}
        note={
          loaded.data.truncated
            ? t("fileBrowser.fileTooLarge", "File too large to preview")
            : undefined
        }
        onOpen={onOpen}
        onDownload={onDownload}
      />
    )
  }

  // markdown (rendered) — view-source falls through to the fenced-code path.
  if (loaded.kind === "markdown" && !viewSource) {
    return (
      <SelectableDomPreview
        key={source.displayPath ?? source.name}
        sourceText={loaded.data.content}
        onQuote={onQuote}
        className="px-4 py-3"
      >
        <MarkdownRenderer content={loaded.data.content} />
      </SelectableDomPreview>
    )
  }

  if (
    loaded.kind === "code" &&
    shikiLang(source.name, source.language) === "html" &&
    !viewSource
  ) {
    return (
      <OfflineHtmlPreview
        name={source.name}
        sizeBytes={loaded.data.sizeBytes}
        content={loaded.data.content}
      />
    )
  }

  // code / text / markdown-source: render directly with Shiki (no Markdown
  // round-trip). Selection → exact line numbers via per-line `data-line`.
  if (loaded.kind === "code" || loaded.kind === "text" || loaded.kind === "markdown") {
    return (
      <ShikiCodeView
        key={source.displayPath ?? source.name}
        content={loaded.data.content}
        lang={shikiLang(source.name, source.language)}
        onQuote={onQuote}
        highlightLines={highlightLines}
        className="text-sm"
      />
    )
  }

  return null
}

function OfflineHtmlPreview({
  name,
  sizeBytes,
  content,
}: {
  name: string
  sizeBytes: number
  content: string
}) {
  const [result, setResult] = useState<{
    content: string
    html: string | null
    error: string | null
  } | null>(null)

  useEffect(() => {
    let cancelled = false
    void buildOfflineHtmlPreview(content).then(
      (html) => {
        if (!cancelled) setResult({ content, html, error: null })
      },
      (error: unknown) => {
        if (!cancelled) {
          setResult({
            content,
            html: null,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      },
    )
    return () => {
      cancelled = true
    }
  }, [content])

  if (result?.content !== content) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
      </div>
    )
  }

  if (!result.html) {
    return <BinaryPlaceholder name={name} sizeBytes={sizeBytes} note={result.error ?? undefined} />
  }

  return (
    <iframe
      title={name}
      srcDoc={result.html}
      sandbox=""
      referrerPolicy="no-referrer"
      className="h-full w-full border-0 bg-white dark:bg-surface-app"
    />
  )
}
