/**
 * 设计产物静态缩略图：懒挂载 + sandbox=""（**不跑 JS**，画廊零动画开销）+ ResizeObserver 等比缩放。
 * 复用产物 index.html 的 asset 服务，无需另建缩略图存储管线。DesignView 产物墙与 DesignFilesPanel
 * 文件管理面共用。
 *
 * 性能（Wave 2-⑦）：**keep-alive 池 + arm-linger** 把峰值 iframe 数钉死——进视口 350ms 仍可见
 * 才向池申请活体槽并挂 iframe（快速滚过不挂，消除滚动掉帧）；池超上限 LRU 逐出，被逐者回退占位。
 * URL 按 id 模块级缓存，重挂 / 逐后再入视即刻恢复、不再 fetch。
 */
import { useCallback, useEffect, useRef, useState } from "react"
import { Palette } from "lucide-react"
import { getTransport } from "@/lib/transport-provider"
import { acquireThumb, setThumbVisible, releaseThumb } from "@/lib/designThumbPool"
import type { DesignArtifactView } from "@/types/design"

const THUMB_DESIGN_W = 1280
const ARM_LINGER_MS = 350

// 已解析的产物预览 URL（id → url|null）。URL 带 ?v=currentVersion 做 cache-bust，故内容更新后
// 经 design:reload/generate_done 失效本缓存 + 重取即得新版本 URL、已挂 iframe 随 src 变化重载
// （对齐主预览 previewKey；此前无版本参数 + 无失效 → AI 改完稿墙上仍旧稿，audit）。
const urlCache = new Map<string, string | null>()

function eventArtifactId(raw: unknown): string | undefined {
  try {
    const o = typeof raw === "string" ? JSON.parse(raw) : raw
    return (o as { artifactId?: string })?.artifactId
  } catch {
    return undefined
  }
}

export function ArtifactThumb({ artifactId }: { artifactId: string }) {
  const wrapRef = useRef<HTMLDivElement>(null)
  const [live, setLive] = useState(false)
  const [src, setSrc] = useState<string | null>(() => urlCache.get(artifactId) ?? null)
  const [scale, setScale] = useState(0.2)
  const liveRef = useRef(false)
  useEffect(() => {
    liveRef.current = live
  }, [live])

  // 解析带版本 cache-bust 的预览 URL；force=清缓存重拉（内容更新后拿新 currentVersion）。
  const resolveSrc = useCallback(
    async (force: boolean): Promise<string | null> => {
      if (!force) {
        const cached = urlCache.get(artifactId)
        if (cached !== undefined) return cached
      }
      try {
        const v = await getTransport().call<DesignArtifactView | null>("get_design_artifact_cmd", {
          id: artifactId,
        })
        const p = v?.artifactPath
        // cache-bust `v=` 必须加在**已解析 URL** 上（对齐主预览 `iframeSrc`）——`resolveAssetUrl` 两侧
        // 都会把整条文件路径编码（Tauri `convertFileSrc` 把 `?`→`%3F`、HTTP `encodeURIComponent`
        // 同样），塞进路径的 `?v=` 会变成文件名一部分 → asset:// / 静态路由 404 → 缩略图全白（audit 根因）。
        const base = p ? getTransport().resolveAssetUrl(`${p}/index.html`) : null
        const url = base ? `${base}${base.includes("?") ? "&" : "?"}v=${v?.currentVersion ?? 0}` : null
        urlCache.set(artifactId, url)
        return url
      } catch {
        urlCache.set(artifactId, null)
        return null
      }
    },
    [artifactId],
  )

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

  // 内容更新失效缩略图（F 修复）：本产物 design:reload / generate_done → 清缓存；若已挂则重取带新版本
  // URL，iframe 随 src 变化重载。未挂时仅清缓存，下次 goLive 自然取新版本。
  useEffect(() => {
    const tx = getTransport()
    const onChange = (raw: unknown) => {
      const id = eventArtifactId(raw)
      if (id && id !== artifactId) return
      urlCache.delete(artifactId)
      if (liveRef.current) void resolveSrc(true).then((url) => url && setSrc(url))
    }
    const off = [tx.listen("design:reload", onChange), tx.listen("design:generate_done", onChange)]
    return () => off.forEach((f) => f())
  }, [artifactId, resolveSrc])

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
      void resolveSrc(false).then((url) => {
        if (cancelled) return
        if (url) setSrc(url)
        acquireThumb(artifactId, () => setLive(false), true)
        setLive(true)
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
  }, [artifactId, resolveSrc])

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
