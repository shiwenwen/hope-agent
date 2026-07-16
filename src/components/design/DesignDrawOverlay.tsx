/**
 * 设计空间「画框批注」叠层（B4-1）。
 *
 * 参考竞品把自由绘制批注做成核心能力；我们据源码级复刻但换了更稳的底座：
 * - 叠层是浮在预览 iframe **之上的父层 canvas**（iframe 跨源 + sandboxed，父层本就无法读进去，
 *   所以绘制只在父层、零沙箱依赖）。
 * - **笔画/框一律归一化存 0..1**（相对本 canvas 矩形），与分辨率无关 —— 父层拿到后按屏上视口
 *   度量映射到离屏整页渲染像素再合成，无需任何 DPR/scale 记账（这是可靠合成的关键不变量）。
 * - 本组件只管画 + 收集 marks + note，**不做捕获/合成**（那需要 iframe/bridge/HTML，归父层 DesignView）。
 *
 * 高可用：捕获底图失败时父层仍可只发「区域 + 文字」——绘制这半永不依赖截图成功。
 */
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react"
import { createPortal } from "react-dom"
import { useTranslation } from "react-i18next"
import { Square, Pen, Undo2, Redo2, Trash2, Send, X, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { IconTip } from "@/components/ui/tooltip"

export interface Point {
  x: number
  y: number
}
export interface NormalizedRect {
  x: number
  y: number
  width: number
  height: number
}
export type DrawMark = { kind: "box"; rect: NormalizedRect } | { kind: "stroke"; points: Point[] }
export interface DesignDrawSubmit {
  boxes: NormalizedRect[]
  strokes: Point[][]
  note: string
}

interface Props {
  /** 捕获/合成在途：禁用提交并转圈。 */
  busy?: boolean
  /** 演示期间保持草稿挂载，但隐藏叠层并暂停全部交互。 */
  suspended?: boolean
  onExit: () => void
  onSubmit: (payload: DesignDrawSubmit) => void
  /** 把 canvas 上的滚轮转发给 iframe（父层 postMessage ds_scroll_by），让用户能滚到目标区再画。 */
  onWheelScroll?: (dx: number, dy: number) => void
  /**
   * 工具坞的 portal 宿主（通常传预览 pane）。frame 包裹层 `overflow-hidden` 会在窄设备框
   *（如 mobile 390px）裁掉工具坞，故 portal 到未裁剪的 pane；缺省则内联渲染（宽视图不裁）。
   */
  toolbarHost?: HTMLElement | null
  /**
   * canvas 尺寸样式 = iframe 可视 footprint（纯宽高、无 transform）。让 canvas 与 iframe 屏上
   * 占位逐像素一致（含同溢出同裁剪），归一化坐标才能正确映射到底图（review 坐标漂移修复）。
   */
  frameStyle: CSSProperties
}

const STROKE_COLOR = "#ff3b30"
const STROKE_WIDTH = 3
const BOX_FILL = "rgba(255,59,48,0.10)"
const MIN_BOX = 0.006 // 归一化最小边长，拒「点击未拖拽」的噪声框

type Tool = "box" | "pen"

// 组件仅在 drawMode 期由父层条件挂载（卸载即天然复位全部 marks/note/counts —— 无需
// setState-in-effect 复位，规避 cascading-render lint 且逻辑更干净）。
export default function DesignDrawOverlay({
  busy,
  suspended = false,
  onExit,
  onSubmit,
  onWheelScroll,
  toolbarHost,
  frameStyle,
}: Props) {
  const { t } = useTranslation()
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [tool, setTool] = useState<Tool>("box")
  const [note, setNote] = useState("")

  // marks/redo 用 ref 存（热路径不触发重渲染），counts 作镜像 state 驱动 toolbar 可用态。
  const marksRef = useRef<DrawMark[]>([])
  const redoRef = useRef<DrawMark[]>([])
  const drawingRef = useRef<Point[] | null>(null) // 在途 pen
  const boxDraftRef = useRef<{ start: Point; current: Point } | null>(null)
  const rafRef = useRef<number | null>(null)
  const [markCount, setMarkCount] = useState(0)
  const [redoCount, setRedoCount] = useState(0)
  const syncCounts = useCallback(() => {
    setMarkCount(marksRef.current.length + (drawingRef.current || boxDraftRef.current ? 1 : 0))
    setRedoCount(redoRef.current.length)
  }, [])

  // ── 归一化坐标：clientXY → 0..1（相对 canvas 矩形），钳 [0,1]。
  const pointFromEvent = useCallback((e: PointerEvent | React.PointerEvent): Point => {
    const cvs = canvasRef.current
    if (!cvs) return { x: 0, y: 0 }
    const r = cvs.getBoundingClientRect()
    const x = r.width > 0 ? (e.clientX - r.left) / r.width : 0
    const y = r.height > 0 ? (e.clientY - r.top) / r.height : 0
    return { x: Math.min(1, Math.max(0, x)), y: Math.min(1, Math.max(0, y)) }
  }, [])

  // ── 绘制（device px backing store）。
  const redraw = useCallback(() => {
    const cvs = canvasRef.current
    if (!cvs) return
    const ctx = cvs.getContext("2d")
    if (!ctx) return
    const W = cvs.width
    const H = cvs.height
    ctx.clearRect(0, 0, W, H)
    const dpr = window.devicePixelRatio || 1
    ctx.lineJoin = "round"
    ctx.lineCap = "round"
    const drawBox = (rect: NormalizedRect) => {
      const x = rect.x * W
      const y = rect.y * H
      const w = rect.width * W
      const h = rect.height * H
      ctx.fillStyle = BOX_FILL
      ctx.fillRect(x, y, w, h)
      ctx.strokeStyle = STROKE_COLOR
      ctx.lineWidth = STROKE_WIDTH * dpr
      ctx.setLineDash([10 * dpr, 6 * dpr])
      ctx.strokeRect(x, y, w, h)
      ctx.setLineDash([])
    }
    const drawStroke = (pts: Point[]) => {
      if (pts.length < 2) return
      ctx.strokeStyle = STROKE_COLOR
      ctx.lineWidth = STROKE_WIDTH * dpr
      ctx.beginPath()
      ctx.moveTo(pts[0].x * W, pts[0].y * H)
      for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x * W, pts[i].y * H)
      ctx.stroke()
    }
    for (const m of marksRef.current) {
      if (m.kind === "box") drawBox(m.rect)
      else drawStroke(m.points)
    }
    if (drawingRef.current) drawStroke(drawingRef.current)
    if (boxDraftRef.current) {
      const { start, current } = boxDraftRef.current
      drawBox({
        x: Math.min(start.x, current.x),
        y: Math.min(start.y, current.y),
        width: Math.abs(current.x - start.x),
        height: Math.abs(current.y - start.y),
      })
    }
  }, [])

  const scheduleRedraw = useCallback(() => {
    if (rafRef.current != null) return
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null
      redraw()
    })
  }, [redraw])

  // ── backing store 随尺寸/DPR 重置，并重绘。
  useEffect(() => {
    const cvs = canvasRef.current
    if (!cvs) return
    const ro = new ResizeObserver(() => {
      const rect = cvs.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      cvs.width = Math.max(1, Math.floor(rect.width * dpr))
      cvs.height = Math.max(1, Math.floor(rect.height * dpr))
      redraw()
    })
    ro.observe(cvs)
    return () => ro.disconnect()
  }, [redraw])

  const commitStroke = useCallback(() => {
    const pts = drawingRef.current
    drawingRef.current = null
    if (pts && pts.length > 1) {
      marksRef.current.push({ kind: "stroke", points: pts })
      redoRef.current = [] // 新 mark 清 redo 栈
    }
    syncCounts()
    redraw()
  }, [syncCounts, redraw])

  const commitBox = useCallback(() => {
    const draft = boxDraftRef.current
    boxDraftRef.current = null
    if (draft) {
      const rect = {
        x: Math.min(draft.start.x, draft.current.x),
        y: Math.min(draft.start.y, draft.current.y),
        width: Math.abs(draft.current.x - draft.start.x),
        height: Math.abs(draft.current.y - draft.start.y),
      }
      if (rect.width >= MIN_BOX && rect.height >= MIN_BOX) {
        marksRef.current.push({ kind: "box", rect })
        redoRef.current = []
      }
    }
    syncCounts()
    redraw()
  }, [syncCounts, redraw])

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
      const p = pointFromEvent(e)
      if (tool === "pen") drawingRef.current = [p]
      else boxDraftRef.current = { start: p, current: p }
      scheduleRedraw()
    },
    [tool, pointFromEvent, scheduleRedraw],
  )
  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const p = pointFromEvent(e)
      if (drawingRef.current) drawingRef.current.push(p)
      else if (boxDraftRef.current) boxDraftRef.current.current = p
      else return
      scheduleRedraw()
    },
    [pointFromEvent, scheduleRedraw],
  )
  const onPointerUp = useCallback(() => {
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current)
      rafRef.current = null
    }
    if (drawingRef.current) commitStroke()
    else if (boxDraftRef.current) commitBox()
  }, [commitStroke, commitBox])

  const undo = useCallback(() => {
    // 在途草稿优先丢弃；否则回退最近 mark 到 redo 栈。
    if (boxDraftRef.current || drawingRef.current) {
      boxDraftRef.current = null
      drawingRef.current = null
    } else {
      const m = marksRef.current.pop()
      if (m) redoRef.current.push(m)
    }
    syncCounts()
    redraw()
  }, [syncCounts, redraw])
  const redo = useCallback(() => {
    const m = redoRef.current.pop()
    if (m) marksRef.current.push(m)
    syncCounts()
    redraw()
  }, [syncCounts, redraw])
  const clearAll = useCallback(() => {
    marksRef.current = []
    redoRef.current = []
    drawingRef.current = null
    boxDraftRef.current = null
    syncCounts()
    redraw()
  }, [syncCounts, redraw])

  // 键盘：Cmd/Ctrl+Z 撤销 / +Shift 重做；Escape 退出（焦点在 note 输入框时让位原生）。
  useEffect(() => {
    if (suspended) return
    const onKey = (e: KeyboardEvent) => {
      const ae = document.activeElement as HTMLElement | null
      const inField = ae?.tagName === "INPUT" || ae?.tagName === "TEXTAREA"
      if (e.key === "Escape" && !inField) {
        e.preventDefault()
        onExit()
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !inField) {
        e.preventDefault()
        if (e.shiftKey) redo()
        else undo()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [onExit, redo, suspended, undo])

  // 滚轮转发给 iframe 内容滚动。**必须原生 non-passive 监听**（React onWheel 是 passive，
  // preventDefault 无效）——否则非 fit 模式下 overflow-auto 预览面会同时原生滚动，产生双重滚动
  // （review LOW UX 修复）。preventDefault 抑制原生面滚动，只留 ds_scroll_by 单一滚动源。
  useEffect(() => {
    if (suspended) return
    const cvs = canvasRef.current
    if (!cvs || !onWheelScroll) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      onWheelScroll(e.deltaX, e.deltaY)
    }
    cvs.addEventListener("wheel", onWheel, { passive: false })
    return () => cvs.removeEventListener("wheel", onWheel)
  }, [onWheelScroll, suspended])

  const submit = useCallback(() => {
    if (busy) return
    const boxes: NormalizedRect[] = []
    const strokes: Point[][] = []
    for (const m of marksRef.current) {
      if (m.kind === "box") boxes.push(m.rect)
      else strokes.push(m.points)
    }
    const hasMarks = boxes.length > 0 || strokes.length > 0
    if (!hasMarks && !note.trim()) return // 无标记且无文字 → 不发
    onSubmit({ boxes, strokes, note: note.trim() })
  }, [busy, note, onSubmit])

  const canSubmit = !busy && (markCount > 0 || note.trim().length > 0)

  // 工具坞：底部居中，pointer-events 自持（不穿透 canvas）。portal 到未裁剪的 pane 避免窄设备框裁掉。
  const dock = (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 bottom-3 z-30 flex justify-center",
        suspended && "invisible",
      )}
    >
      <div className="pointer-events-auto flex items-center gap-1 rounded-full border border-border/60 bg-background/95 px-2 py-1.5 shadow-lg backdrop-blur">
          <div className="flex items-center gap-0.5">
            <IconTip label={t("design.draw.toolBox", "画框")}>
              <Button
                variant={tool === "box" ? "secondary" : "ghost"}
                size="icon"
                className="h-8 w-8"
                onClick={() => setTool("box")}
              >
                <Square className="h-4 w-4" />
              </Button>
            </IconTip>
            <IconTip label={t("design.draw.toolPen", "自由笔")}>
              <Button
                variant={tool === "pen" ? "secondary" : "ghost"}
                size="icon"
                className="h-8 w-8"
                onClick={() => setTool("pen")}
              >
                <Pen className="h-4 w-4" />
              </Button>
            </IconTip>
          </div>
          <div className="mx-0.5 h-5 w-px bg-border/60" />
          <IconTip label={t("design.draw.undo", "撤销")}>
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={undo} disabled={markCount === 0}>
              <Undo2 className="h-4 w-4" />
            </Button>
          </IconTip>
          <IconTip label={t("design.draw.redo", "重做")}>
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={redo} disabled={redoCount === 0}>
              <Redo2 className="h-4 w-4" />
            </Button>
          </IconTip>
          <IconTip label={t("design.draw.clear", "清空")}>
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={clearAll} disabled={markCount === 0}>
              <Trash2 className="h-4 w-4" />
            </Button>
          </IconTip>
          <div className="mx-0.5 h-5 w-px bg-border/60" />
          <Input
            value={note}
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault()
                submit()
              }
            }}
            placeholder={t("design.draw.notePlaceholder", "说明要改什么…")}
            className="h-8 w-48 text-xs"
          />
          <Button size="sm" className="h-8 gap-1" onClick={submit} disabled={!canSubmit}>
            {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Send className="h-3.5 w-3.5" />}
            {t("design.draw.send", "带到对话")}
          </Button>
          <IconTip label={t("design.draw.exit", "退出批注")}>
            <Button variant="ghost" size="icon" className="h-8 w-8" onClick={onExit}>
              <X className="h-4 w-4" />
            </Button>
          </IconTip>
        </div>
      </div>
  )

  return (
    <div
      className={cn(
        "absolute inset-0 z-20",
        suspended && "invisible pointer-events-none",
      )}
    >
      <canvas
        ref={canvasRef}
        className="absolute left-0 top-0 cursor-crosshair touch-none"
        style={frameStyle}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      />
      {toolbarHost ? createPortal(dock, toolbarHost) : dock}
    </div>
  )
}
