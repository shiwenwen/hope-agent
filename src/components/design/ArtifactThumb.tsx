/**
 * 设计产物静态缩略图：懒挂载（IntersectionObserver）+ sandbox=""（**不跑 JS**，画廊零动画
 * 开销、性能稳定）+ ResizeObserver 等比缩放。复用产物 index.html 的 asset 服务，无需另建
 * 缩略图存储管线。DesignView 产物墙与 DesignFilesPanel 文件管理面共用。
 */
import { useEffect, useRef, useState } from "react"
import { Palette } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import type { DesignArtifactView } from "@/types/design"

const THUMB_DESIGN_W = 1280

export function ArtifactThumb({ artifactId }: { artifactId: string }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const [src, setSrc] = useState<string | null>(null)
  const [scale, setScale] = useState(0.2)

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) setScale(el.clientWidth / THUMB_DESIGN_W)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    let done = false
    const io = new IntersectionObserver(
      (entries) => {
        if (done || !entries.some((e) => e.isIntersecting)) return
        done = true
        io.disconnect()
        getTransport()
          .call<DesignArtifactView | null>("get_design_artifact_cmd", { id: artifactId })
          .then((v) => {
            const p = v?.artifactPath
            if (p) {
              const url = getTransport().resolveAssetUrl(`${p}/index.html`)
              if (url) setSrc(url)
            }
          })
          .catch(() => {})
      },
      { rootMargin: "300px" },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [artifactId])

  return (
    <div
      ref={wrapRef}
      className="relative h-full w-full overflow-hidden bg-gradient-to-br from-muted to-muted/40"
    >
      {src ? (
        <iframe
          src={src}
          sandbox=""
          scrolling="no"
          tabIndex={-1}
          aria-hidden="true"
          title=""
          className="pointer-events-none absolute left-0 top-0 origin-top-left border-0"
          style={{
            width: THUMB_DESIGN_W,
            height: THUMB_DESIGN_W * 0.75,
            transform: `scale(${scale})`,
          }}
        />
      ) : (
        <div className="flex h-full items-center justify-center">
          <Palette className="h-6 w-6 text-muted-foreground/25" />
        </div>
      )}
    </div>
  )
}
