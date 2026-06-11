import { useCallback, useEffect, useRef, useState } from "react"

import { logger } from "@/lib/logger"
import { OfficeLoading } from "./OfficeLoading"
import { OfficeZoomBar } from "./OfficeZoomBar"
import type { OfficeViewProps } from "./types"
import { useFitZoom } from "./useFitZoom"

interface SheetHtml {
  name: string
  html: string
}

/**
 * Renders an `.xlsx` / `.xls` as HTML tables via SheetJS (lazy-loaded).
 * `sheet_to_html` preserves merged cells (colspan/rowspan) and escapes cell
 * text; table chrome is applied with Tailwind arbitrary-descendant utilities.
 * Wide sheets are scaled to fit the panel by default (see {@link useFitZoom});
 * the bottom bar offers manual zoom / fit-width. Cell fill/font colors are not
 * preserved (SheetJS community edition limitation).
 */
export function XlsxView({ data, onError }: OfficeViewProps) {
  const outerRef = useRef<HTMLDivElement>(null)
  const innerRef = useRef<HTMLDivElement>(null)
  const [sheets, setSheets] = useState<SheetHtml[] | null>(null)

  const measure = useCallback(() => innerRef.current?.scrollWidth ?? 0, [])
  const { scale, fitMode, zoomIn, zoomOut, fitWidth, onContentReady } = useFitZoom(outerRef, measure)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const XLSX = await import("xlsx")
        if (cancelled) return
        const wb = XLSX.read(new Uint8Array(data), { type: "array" })
        const out: SheetHtml[] = []
        for (const name of wb.SheetNames) {
          const ws = wb.Sheets[name]
          if (ws) out.push({ name, html: XLSX.utils.sheet_to_html(ws, { editable: false }) })
        }
        if (!cancelled) setSheets(out)
      } catch (e) {
        if (!cancelled) {
          logger.error(
            "ui",
            "XlsxView::render",
            `SheetJS parse failed: ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`,
          )
          onError(e)
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [data, onError])

  // Fit once the sheets are in the DOM (zoom is still 1 on first paint).
  useEffect(() => {
    if (sheets) onContentReady()
  }, [sheets, onContentReady])

  if (!sheets) return <OfficeLoading />

  return (
    <div className="flex h-full flex-col">
      <div ref={outerRef} className="flex-1 overflow-auto">
        <div ref={innerRef} className="w-fit space-y-6 p-3" style={{ zoom: scale }}>
          {sheets.map((s) => (
            <section key={s.name} className="space-y-2">
              {sheets.length > 1 && (
                <h3 className="text-sm font-semibold text-foreground">{s.name}</h3>
              )}
              <div
                className="text-sm [&_table]:border-collapse [&_td]:whitespace-nowrap [&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1 [&_td]:align-top [&_tr:first-child]:bg-muted [&_tr:first-child]:font-medium"
                // SheetJS escapes cell text; the file is the user's own local document.
                dangerouslySetInnerHTML={{ __html: s.html }}
              />
            </section>
          ))}
        </div>
      </div>
      <OfficeZoomBar
        scale={scale}
        fitMode={fitMode}
        zoomIn={zoomIn}
        zoomOut={zoomOut}
        fitWidth={fitWidth}
      />
    </div>
  )
}
