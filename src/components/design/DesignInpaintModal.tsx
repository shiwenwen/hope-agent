/**
 * 蒙版局部重绘（inpaint）对话框：在 image 产物上涂画要重绘的区域 + 一句 prompt →
 * 后端走 OpenAI `/images/edits`（image + mask）落新版本。
 *
 * 蒙版画布：加载产物预览图为底，覆盖一层可涂画 canvas；用户涂白=重绘区。导出时把涂画区
 * 转成**透明**（OpenAI edits 约定：蒙版透明处 = 重绘），其余不透明黑保留。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Eraser, Brush, RotateCcw, TriangleAlert } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import { toast } from "sonner"
import type { MediaGenOverview } from "@/components/settings/media-gen/types"
import { openMediaModelSettings } from "@/components/settings/media-gen/types"
import { fetchMediaGenOverview } from "@/components/settings/media-gen/useMediaGenData"

interface Props {
  open: boolean
  onClose: () => void
  artifactId: string | null
  /** 产物预览 index.html URL；modal 内部 fetch 并提取内嵌 data-uri 图作底图。 */
  indexUrl: string | null
  /** 重绘成功回调（前端可刷新预览）。 */
  onDone?: () => void
}

const CANVAS_MAX = 512

export function DesignInpaintModal({ open, onClose, artifactId, indexUrl, onDone }: Props) {
  const { t } = useTranslation()
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const imgRef = useRef<HTMLImageElement | null>(null)
  const drawingRef = useRef(false)
  const [prompt, setPrompt] = useState("")
  const [brush, setBrush] = useState(28)
  const [erasing, setErasing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [dims, setDims] = useState({ w: CANVAS_MAX, h: CANVAS_MAX })
  const [hasStroke, setHasStroke] = useState(false)
  const [imageUrl, setImageUrl] = useState<string | null>(null)
  // 能力驱动 hint：打开时拉 sanitized overview，找首个支持蒙版重绘的图像候选。
  const [overview, setOverview] = useState<MediaGenOverview | null>(null)

  useEffect(() => {
    if (!open) return
    let cancelled = false
    setOverview(null)
    void fetchMediaGenOverview().then((ov) => {
      if (!cancelled) setOverview(ov)
    })
    return () => {
      cancelled = true
    }
  }, [open])

  const maskCand = useMemo(
    () => overview?.image.candidates.find((c) => c.image?.supportsMask) ?? null,
    [overview],
  )

  // 打开时 fetch index.html → 提取第一个 data:image URI → 载底图，按长边 ≤512 缩放画布。
  useEffect(() => {
    if (!open || !indexUrl) return
    setPrompt("")
    setHasStroke(false)
    setImageUrl(null)
    let cancelled = false
    void (async () => {
      let dataUri: string | null = null
      try {
        const html = await (await fetch(indexUrl)).text()
        const m = html.match(/data:image\/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=]+/)
        dataUri = m?.[0] ?? null
      } catch {
        /* 取不到 index.html → 画布空白仍可涂 */
      }
      if (cancelled || !dataUri) return
      setImageUrl(dataUri)
      const img = new Image()
      img.onload = () => {
        if (cancelled) return
        const scale = Math.min(1, CANVAS_MAX / Math.max(img.width, img.height))
        const w = Math.max(1, Math.round(img.width * scale))
        const h = Math.max(1, Math.round(img.height * scale))
        imgRef.current = img
        setDims({ w, h })
        requestAnimationFrame(() => {
          canvasRef.current?.getContext("2d")?.clearRect(0, 0, w, h)
        })
      }
      img.src = dataUri
    })()
    return () => {
      cancelled = true
    }
  }, [open, indexUrl])

  const paintAt = useCallback(
    (x: number, y: number) => {
      const c = canvasRef.current
      const ctx = c?.getContext("2d")
      if (!ctx) return
      ctx.globalCompositeOperation = erasing ? "destination-out" : "source-over"
      ctx.fillStyle = "rgba(239,68,68,0.55)" // 半透明红标记重绘区
      ctx.beginPath()
      ctx.arc(x, y, brush / 2, 0, Math.PI * 2)
      ctx.fill()
      if (!erasing) setHasStroke(true)
    },
    [brush, erasing],
  )

  const relPos = (e: React.PointerEvent) => {
    const c = canvasRef.current!
    const r = c.getBoundingClientRect()
    return {
      x: ((e.clientX - r.left) / r.width) * dims.w,
      y: ((e.clientY - r.top) / r.height) * dims.h,
    }
  }

  const clearMask = () => {
    const ctx = canvasRef.current?.getContext("2d")
    if (ctx) ctx.clearRect(0, 0, dims.w, dims.h)
    setHasStroke(false)
  }

  // 导出蒙版：涂画区（画布有像素）→ 透明；其余 → 不透明黑。得到 OpenAI edits 约定的 mask PNG。
  const exportMaskB64 = (): string | null => {
    const src = canvasRef.current
    if (!src) return null
    const w = dims.w
    const h = dims.h
    const srcCtx = src.getContext("2d")
    if (!srcCtx) return null
    const data = srcCtx.getImageData(0, 0, w, h).data
    const out = document.createElement("canvas")
    out.width = w
    out.height = h
    const octx = out.getContext("2d")
    if (!octx) return null
    const maskImg = octx.createImageData(w, h)
    for (let i = 0; i < w * h; i++) {
      const painted = data[i * 4 + 3] > 10 // 涂画处 alpha>0
      // 透明=重绘，其余黑不透明=保留。
      maskImg.data[i * 4] = 0
      maskImg.data[i * 4 + 1] = 0
      maskImg.data[i * 4 + 2] = 0
      maskImg.data[i * 4 + 3] = painted ? 0 : 255
    }
    octx.putImageData(maskImg, 0, 0)
    return out.toDataURL("image/png").split(",")[1] ?? null
  }

  const run = async () => {
    if (!artifactId || busy || !hasStroke) return
    const maskB64 = exportMaskB64()
    if (!maskB64) {
      toast.error(t("design.inpaint.noMask", "请先涂出要重绘的区域"))
      return
    }
    setBusy(true)
    try {
      await getTransport().call("inpaint_design_image_cmd", {
        id: artifactId,
        prompt: prompt.trim(),
        maskB64,
      })
      toast.success(t("design.inpaint.done", "已按蒙版重绘"))
      onDone?.()
      onClose()
    } catch (e) {
      logger.error("design", "DesignInpaintModal", "inpaint failed", e)
      const msg = String((e as Error)?.message || e).slice(0, 160)
      toast.error(t("design.inpaint.failed", "重绘失败：{{msg}}", { msg }))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Brush className="h-4 w-4" />
            {t("design.inpaint.title", "蒙版局部重绘")}
          </DialogTitle>
        </DialogHeader>
        <p className="text-xs text-muted-foreground">
          {t("design.gen.inpaintHint", "在图上涂出要重绘的区域，再描述想要的内容。")}
        </p>
        {overview &&
          (maskCand ? (
            <p className="text-xs text-muted-foreground">
              {t("design.gen.inpaintUsing", "将使用 {{provider}} / {{model}} 进行局部重绘", {
                provider: maskCand.providerName,
                model: maskCand.modelName || maskCand.modelId,
              })}
            </p>
          ) : (
            /* 无 mask-capable 候选：警示但不阻断（后端会给出明确错误）。 */
            <div className="flex items-center gap-2 rounded-md bg-amber-500/10 px-2.5 py-1.5 text-xs text-amber-600 dark:text-amber-400">
              <TriangleAlert className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1">
                {t("design.gen.inpaintNoMaskModel", "当前没有支持蒙版重绘的图像模型")}
              </span>
              <button
                type="button"
                className="shrink-0 underline underline-offset-2 transition-colors hover:text-foreground"
                onClick={() => {
                  openMediaModelSettings()
                  onClose()
                }}
              >
                {t("design.gen.goConfigure", "去配置媒体生成模型")}
              </button>
            </div>
          ))}

        <div className="flex justify-center">
          <div
            className="relative touch-none overflow-hidden rounded-lg border bg-[repeating-conic-gradient(#e5e7eb_0_25%,transparent_0_50%)] bg-[length:16px_16px]"
            style={{ width: dims.w, height: dims.h, maxWidth: "100%" }}
          >
            {imageUrl && (
              <img
                src={imageUrl}
                alt=""
                className="pointer-events-none absolute inset-0 h-full w-full object-contain"
                draggable={false}
              />
            )}
            <canvas
              ref={canvasRef}
              width={dims.w}
              height={dims.h}
              className="absolute inset-0 h-full w-full cursor-crosshair"
              onPointerDown={(e) => {
                drawingRef.current = true
                ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
                const p = relPos(e)
                paintAt(p.x, p.y)
              }}
              onPointerMove={(e) => {
                if (!drawingRef.current) return
                const p = relPos(e)
                paintAt(p.x, p.y)
              }}
              onPointerUp={() => {
                drawingRef.current = false
              }}
            />
          </div>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant={erasing ? "secondary" : "outline"}
            size="sm"
            className="h-8 gap-1"
            onClick={() => setErasing((v) => !v)}
          >
            <Eraser className="h-3.5 w-3.5" />
            {erasing ? t("design.inpaint.erasing", "擦除中") : t("design.inpaint.erase", "擦除")}
          </Button>
          <input
            type="range"
            min={8}
            max={64}
            value={brush}
            onChange={(e) => setBrush(Number(e.target.value))}
            className="flex-1 accent-primary"
            aria-label={t("design.inpaint.brush", "笔刷大小")}
          />
          <Button variant="ghost" size="sm" className="h-8 gap-1" onClick={clearMask}>
            <RotateCcw className="h-3.5 w-3.5" />
            {t("design.inpaint.clear", "清空")}
          </Button>
        </div>

        <Input
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={t("design.inpaint.promptPh", "描述这块区域要重绘成什么…")}
          className="text-sm"
        />

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel", "取消")}
          </Button>
          <Button onClick={() => void run()} disabled={busy || !hasStroke}>
            {busy && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
            {t("design.inpaint.run", "重绘")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export default DesignInpaintModal
