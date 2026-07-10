/**
 * Deck 幻灯片缩略图（缩略图轨用）：无 JS 的 `sandbox=""` iframe 加载 `index.html#ds-slide-N`，
 * 靠后端注入的 `.ds-slide:target{display:block}` 纯 CSS 点亮该页（零脚本、零动画开销）。
 * 复用 ArtifactThumb 同款 keep-alive 池 + arm-linger（峰值 iframe 数钉死），点击跳页。
 */
import { useEffect, useRef, useState } from "react"
import { acquireThumb, setThumbVisible, releaseThumb } from "@/lib/designThumbPool"

const THUMB_W = 1280
const THUMB_H = 720
const ARM_LINGER_MS = 250

export function DeckSlideThumb({
  poolKey,
  src,
  index,
  active,
  onSelect,
}: {
  poolKey: string
  src: string
  index: number
  active: boolean
  onSelect: (index: number) => void
}) {
  const wrapRef = useRef<HTMLButtonElement>(null)
  const [live, setLive] = useState(false)
  const [scale, setScale] = useState(0.1)

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) setScale(el.clientWidth / THUMB_W)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    let armTimer: number | null = null
    const clearArm = () => {
      if (armTimer != null) {
        window.clearTimeout(armTimer)
        armTimer = null
      }
    }
    const io = new IntersectionObserver(
      (entries) => {
        const vis = entries.some((e) => e.isIntersecting)
        if (vis) {
          setThumbVisible(poolKey, true)
          clearArm()
          armTimer = window.setTimeout(() => {
            acquireThumb(poolKey, () => setLive(false), true)
            setLive(true)
          }, ARM_LINGER_MS)
        } else {
          clearArm()
          setThumbVisible(poolKey, false)
        }
      },
      { root: el.parentElement, rootMargin: "200px" },
    )
    io.observe(el)
    return () => {
      clearArm()
      io.disconnect()
      releaseThumb(poolKey)
    }
  }, [poolKey])

  return (
    <button
      ref={wrapRef}
      type="button"
      onClick={() => onSelect(index)}
      aria-label={`${index + 1}`}
      aria-current={active ? "true" : undefined}
      className={`group/slide relative aspect-video w-24 shrink-0 overflow-hidden rounded border bg-muted transition-shadow ${
        active ? "border-primary ring-2 ring-primary" : "border-border hover:border-primary/50"
      }`}
    >
      {live ? (
        <iframe
          src={src}
          sandbox=""
          scrolling="no"
          tabIndex={-1}
          aria-hidden="true"
          title=""
          className="pointer-events-none absolute left-0 top-0 origin-top-left border-0"
          style={{ width: THUMB_W, height: THUMB_H, transform: `scale(${scale})` }}
        />
      ) : null}
      <span className="pointer-events-none absolute bottom-0.5 left-0.5 rounded bg-background/80 px-1 text-[9px] font-medium tabular-nums text-muted-foreground">
        {index + 1}
      </span>
    </button>
  )
}
