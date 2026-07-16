import { useCallback, useEffect, useState } from "react"

import { logger } from "@/lib/logger"
import type { PreviewSource } from "../previewSource"
import { DocxView } from "./DocxView"
import { officeFormatOf } from "./officeFormat"
import { OfficeLoading } from "./OfficeLoading"
import { OfficeTextFallback } from "./OfficeTextFallback"
import { PptxView } from "./PptxView"
import { XlsxView } from "./XlsxView"
import { MEBIBYTE_BYTES, useFilesystemConfig } from "@/lib/filesystemConfig"
import { readResponseArrayBufferWithLimit } from "../readResponseWithLimit"

/** Above this, skip rich rendering (parse/DOM cost) and fall back to text. */
const MAX_RICH_BYTES = 30 * 1024 * 1024

/**
 * Rich preview for office files: resolve the sub-format, fetch the raw bytes via
 * the source's authorized `rawUrl` (Tauri asset / HTTP by-path — the same
 * channel image/pdf previews use), and hand them to the matching lazy-loaded
 * renderer (docx-preview / SheetJS / pptxviewjs). Anything that can't render
 * richly — unsupported format, oversized file, fetch/lib failure — flips to
 * {@link OfficeTextFallback} (the backend's plain-text extraction), so this is
 * never worse than the previous text-only preview.
 *
 * Mount with a `key` tied to the file (the caller does) so a new file resets
 * state via remount — the effect only fetches, it never resets synchronously.
 */
export function OfficeRichPreview({
  source,
  onOpen,
  onDownload,
}: {
  source: PreviewSource
  onOpen?: () => void
  onDownload?: () => void
}) {
  const { config: filesystemConfig } = useFilesystemConfig()
  const format = officeFormatOf(source.name, source.mime)
  const effectiveLimit = Math.min(
    MAX_RICH_BYTES,
    filesystemConfig.maxDocumentPreviewMb * MEBIBYTE_BYTES,
  )
  const tooBig = source.sizeBytes != null && source.sizeBytes > effectiveLimit
  const eligible = format !== null && !tooBig

  const [data, setData] = useState<ArrayBuffer | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    if (!eligible) return
    let cancelled = false
    void (async () => {
      try {
        const url = await source.rawUrl(false)
        if (cancelled) return
        // Clear any prior failure/bytes now that a new source is loading. Done
        // post-await (not synchronously in the effect body) to stay clear of the
        // set-state-in-effect lint; this covers a `key` collision where React
        // reuses this instance instead of remounting it.
        setFailed(false)
        setData(null)
        if (!url) {
          logger.warn(
            "ui",
            "OfficeRichPreview::fetch",
            "no preview URL for source — falling back to text",
          )
          setFailed(true)
          return
        }
        const res = await fetch(url)
        if (!res.ok) throw new Error(`fetch failed: ${res.status}`)
        const buf = await readResponseArrayBufferWithLimit(res, effectiveLimit)
        if (cancelled) return
        setData(buf)
      } catch (e) {
        if (!cancelled) {
          logger.error(
            "ui",
            "OfficeRichPreview::fetch",
            `office bytes fetch failed (e.g. CSP connect-src / network): ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
          )
          setFailed(true)
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [source, eligible, effectiveLimit])

  const onError = useCallback(() => setFailed(true), [])

  if (!eligible || failed) {
    return <OfficeTextFallback source={source} onOpen={onOpen} onDownload={onDownload} />
  }
  if (!data) return <OfficeLoading />
  if (format === "docx") return <DocxView data={data} onError={onError} />
  if (format === "xlsx") return <XlsxView data={data} onError={onError} />
  return <PptxView data={data} onError={onError} />
}
