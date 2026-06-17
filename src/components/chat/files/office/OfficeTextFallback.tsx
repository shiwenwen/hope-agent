import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import type { ExtractedContent } from "@/lib/transport"
import type { PreviewSource } from "../previewSource"
import { BinaryPlaceholder } from "../../project/file-browser/BinaryPlaceholder"
import { OfficeLoading } from "./OfficeLoading"

/**
 * Plain-text fallback for office files — used when rich rendering is impossible
 * (unsupported sub-format, corrupt/oversized file, renderer failure). Lazily
 * runs the backend's `extractDoc()` (text + embedded images) only when actually
 * needed, so a successful rich render never pays for extraction.
 */
export function OfficeTextFallback({ source }: { source: PreviewSource }) {
  const { t } = useTranslation()
  const [data, setData] = useState<ExtractedContent | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const d = await source.extractDoc()
        if (!cancelled) setData(d)
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [source])

  // Extraction failed too (e.g. remote attachment with no local bytes) — keep
  // open/download affordances so the file isn't a dead end, matching the
  // pane-level binary fallback.
  if (error)
    return (
      <BinaryPlaceholder
        name={source.name}
        sizeBytes={source.sizeBytes ?? 0}
        note={error}
        onOpen={() => void source.rawUrl(false).then((u) => u && window.open(u, "_blank"))}
        onDownload={() => void source.rawUrl(true).then((u) => u && window.open(u, "_blank"))}
      />
    )
  if (!data) return <OfficeLoading />

  const { text, images } = data
  return (
    <div className="space-y-4 px-4 py-3">
      <div className="rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
        {t(
          "fileBrowser.extractedPreview",
          "Extracted preview — layout may differ from the original. Open the file for the exact formatting.",
        )}
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
        <div className="text-sm text-muted-foreground">
          {t("fileBrowser.officeNoContent", "No content could be extracted from this file.")}
        </div>
      ) : null}
    </div>
  )
}
