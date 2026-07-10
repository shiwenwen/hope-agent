/**
 * 设计产物静态缩略图：懒挂载 + sandbox=""（**不跑 JS**，画廊零动画开销）+ ResizeObserver 等比缩放。
 * 复用产物 index.html 的 asset 服务，无需另建缩略图存储管线。DesignView 产物墙与 DesignFilesPanel
 * 文件管理面共用。
 *
 * 性能（Wave 2-⑦）：**keep-alive 池 + arm-linger** 把峰值 iframe 数钉死——进视口 350ms 仍可见
 * 才向池申请活体槽并挂 iframe（快速滚过不挂，消除滚动掉帧）；池超上限 LRU 逐出，被逐者回退占位。
 * URL 按 id 模块级缓存，重挂 / 逐后再入视即刻恢复、不再 fetch。
 */
import { useEffect, useRef, useState } from "react"
import { Palette } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { acquireThumb, setThumbVisible, releaseThumb } from "@/lib/designThumbPool"
import type { DesignArtifactView } from "@/types/design"

const THUMB_DESIGN_W = 1280
const ARM_LINGER_MS = 350

// 已解析的产物预览 URL（id → url|null）。跨挂载缓存，逐出后再入视即刻恢复。
const urlCache = new Map<string, string | null>()

export function ArtifactThumb({ artifactId }: { artifactId: string }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const [live, setLive] = useState(false)
  const [src, setSrc] = useState<string | null>(() => urlCache.get(artifactId) ?? null)
  const [scale, setScale] = useState(0.2)

  // 等比缩放（全页宽 → 卡片宽）。
  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) setScale(el.clientWidth / THUMB_DESIGN_W)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // 可见性 + arm-linger + 池申请。
  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    let armTimer: number | null = null
    let cancelled = false // 卸载后异步 fetch 回来不再 acquire（防幽灵活体槽，review LOW）
    const clearArm = () => {
      if (armTimer != null) {
        window.clearTimeout(armTimer)
        armTimer = null
      }
    }
    const goLive = () => {
      // 已有缓存 URL → 直接用；否则拉一次。拿到后向池申请槽。
      const mount = (url: string | null) => {
        if (cancelled) return
        urlCache.set(artifactId, url)
        if (url) setSrc(url)
        acquireThumb(artifactId, () => setLive(false), true)
        setLive(true)
      }
      const cached = urlCache.get(artifactId)
      if (cached !== undefined) {
        mount(cached)
        return
      }
      getTransport()
        .call<DesignArtifactView | null>("get_design_artifact_cmd", { id: artifactId })
        .then((v) => {
          const p = v?.artifactPath
          mount(p ? (getTransport().resolveAssetUrl(`${p}/index.html`) ?? null) : null)
        })
        .catch(() => {
          urlCache.set(artifactId, null)
        })
    }
    const io = new IntersectionObserver(
      (entries) => {
        const vis = entries.some((e) => e.isIntersecting)
        if (vis) {
          setThumbVisible(artifactId, true) // 若已在池：刷新触达
          // arm-linger：连续可见 350ms 才挂，快速滚过取消。
          clearArm()
          armTimer = window.setTimeout(goLive, ARM_LINGER_MS)
        } else {
          clearArm()
          setThumbVisible(artifactId, false) // keep-alive：标记可优先逐出，暂不卸载
        }
      },
      { rootMargin: "300px" },
    )
    io.observe(el)
    return () => {
      cancelled = true
      clearArm()
      io.disconnect()
      releaseThumb(artifactId)
    }
  }, [artifactId])

  return (
    <div
      ref={wrapRef}
      className="relative h-full w-full overflow-hidden bg-gradient-to-br from-muted to-muted/40"
    >
      {live && src ? (
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
