/**
 * 设计空间客户端导出引擎。
 *
 * 关键：用**自包含 HTML → Blob URL → 同源隐藏 iframe** 栅格化（绕开 asset:// 跨域，
 * Tauri + HTTP 两模式通用、非打断、无需 Chrome）。PNG/PDF 纯前端（html2canvas + jspdf），
 * PPTX 前端栅格化 + 后端 zip 组装（见 crates/ha-core/src/design/export.rs）。
 */

import html2canvas from "html2canvas"
import { jsPDF } from "jspdf"
import type { ArtifactKind } from "@/types/design"
import { getTransport } from "@/lib/transport-provider"

interface RenderHandle {
  doc: Document
  win: Window
  cleanup: () => void
}

/** 各形态的自然渲染宽度（无显式视口时的兜底）。 */
function kindWidth(kind: ArtifactKind, vw?: number): number {
  if (vw && vw > 0) return vw
  switch (kind) {
    case "mobile":
      return 390
    case "deck":
    case "motion":
      return 1280
    case "poster":
      return 1080
    case "document":
      return 820
    case "email":
      return 600
    default:
      return 1440
  }
}

/** 把自包含 HTML 载入一个离屏同源 iframe，等待布局稳定。 */
async function renderHtml(html: string, width: number): Promise<RenderHandle> {
  const blob = new Blob([html], { type: "text/html" })
  const url = URL.createObjectURL(blob)
  const iframe = document.createElement("iframe")
  iframe.setAttribute("aria-hidden", "true")
  iframe.style.cssText = `position:fixed;left:-99999px;top:0;width:${width}px;height:1200px;border:0;background:#fff;visibility:hidden`
  document.body.appendChild(iframe)
  try {
    await new Promise<void>((resolve, reject) => {
      iframe.onload = () => resolve()
      iframe.onerror = () => reject(new Error("export iframe failed to load"))
      iframe.src = url
    })
  } catch (e) {
    // Don't leak the hidden iframe + Blob URL when load fails before we return the
    // handle (the caller's `finally { h.cleanup() }` never runs in that case).
    iframe.remove()
    URL.revokeObjectURL(url)
    throw e
  }
  const doc = iframe.contentDocument
  const win = iframe.contentWindow
  if (!doc || !win) {
    iframe.remove()
    URL.revokeObjectURL(url)
    throw new Error("export iframe has no document")
  }
  // 等字体**真正就绪**再抓帧（fonts.ready 比固定延时可靠，慢字体不会未加载就被栅格化）；
  // 3s 兜底防字体 hang 无限等，再留一小段布局 settle。
  try {
    await Promise.race([
      doc.fonts?.ready ?? Promise.resolve(),
      new Promise((r) => setTimeout(r, 3000)),
    ])
  } catch {
    /* fonts API 缺失 / reject → 直接往下走 */
  }
  await new Promise((r) => setTimeout(r, 80))
  // iframe 高度贴合内容，保证 full-page 捕获。
  const h = Math.max(doc.body.scrollHeight, doc.documentElement.scrollHeight, 720)
  iframe.style.height = `${h}px`
  await new Promise((r) => setTimeout(r, 50))
  return {
    doc,
    win: win as Window,
    cleanup: () => {
      iframe.remove()
      URL.revokeObjectURL(url)
    },
  }
}

function pickTarget(doc: Document): HTMLElement {
  const frame = doc.querySelector(".ds-frame, .ds-stage") as HTMLElement | null
  return frame ?? doc.body
}

function slidesOf(doc: Document): HTMLElement[] {
  return Array.from(doc.querySelectorAll(".ds-slide")) as HTMLElement[]
}

/** 导出选项（配置驱动，全部可选；缺省用好默认）。 */
export interface ExportOpts {
  /** 栅格化倍率（清晰度），钳 [1,4]。默认 2（retina）。 */
  scale?: number
  /** PDF 页 JPEG 压缩质量（1–100），钳 [40,100]。默认 92。 */
  jpegQuality?: number
  onProgress?: (done: number, total: number) => void
}

const DEFAULT_SCALE = 2
const DEFAULT_JPEG_Q = 92
const scaleOf = (o?: ExportOpts) => Math.min(4, Math.max(1, o?.scale ?? DEFAULT_SCALE))
/** JPEG quality as a 0–1 fraction for canvas.toDataURL. */
const jpegQ = (o?: ExportOpts) => Math.min(1, Math.max(0.4, (o?.jpegQuality ?? DEFAULT_JPEG_Q) / 100))

async function rasterize(el: HTMLElement, scale: number): Promise<HTMLCanvasElement> {
  return html2canvas(el, {
    backgroundColor: "#ffffff",
    scale,
    useCORS: true,
    logging: false,
  })
}

function canvasToPngBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("canvas toBlob failed"))), "image/png"),
  )
}

/**
 * B4-1 画框批注底图：把自包含 HTML 按 `viewportWidth` 离屏整页栅格化（复用 export 同款
 * 跨源/无 Chrome 底座）。**渲染宽度必须 = 屏上内容视口宽（bridge `clientWidth`）**，否则
 * 响应式重排会与用户所见错位。返回整页 canvas（documentElement，含全滚动高度）+ 实际倍率，
 * 供父层把归一化笔画按 `px = (scrollX + nx*clientWidth) * scale` 合成到画布再裁剪。
 */
export async function rasterizeArtifactFull(
  html: string,
  viewportWidth: number,
  opts?: ExportOpts,
): Promise<{ canvas: HTMLCanvasElement; scale: number }> {
  const reqScale = scaleOf(opts)
  const h = await renderHtml(html, Math.max(1, Math.round(viewportWidth)))
  try {
    const de = h.doc.documentElement
    // **钳倍率使 canvas 单边 ≤ MAX_CANVAS_PX（review MED 修复）**：超限时 WKWebView 静默返回
    // 空白 canvas 且不抛错 → 会绕过「捕获失败降级文字」，把空白图当截图发给模型。按内容真实尺寸
    // 反推安全倍率（长产物自动降清晰度换取完整、不空白）；返回的 scale 供合成侧 1:1 映射。
    const w = Math.max(1, de.scrollWidth)
    const contentH = Math.max(1, de.scrollHeight)
    const scale = Math.max(0.1, Math.min(reqScale, MAX_CANVAS_PX / w, MAX_CANVAS_PX / contentH))
    const canvas = await rasterize(de, scale)
    return { canvas, scale }
  } finally {
    h.cleanup()
  }
}

/** 单 canvas 安全边长上限（WKWebView ~16384 / Chromium 更高，取保守值）。 */
const MAX_CANVAS_PX = 16000

/**
 * 把多张画布纵向拼成一张长图（居中、白底）。整幅超单 canvas 上限时**等比缩小**保证
 * 所有页都进一张图且导出不失败（大 deck 请优先用 PDF/PPTX）；分配失败**抛错**而非
 * 静默只出首页伪装成功。
 */
function stitchVertical(canvases: HTMLCanvasElement[]): HTMLCanvasElement {
  const rawWidth = Math.max(...canvases.map((c) => c.width))
  const rawHeight = canvases.reduce((s, c) => s + c.height, 0)
  const fit = Math.min(1, MAX_CANVAS_PX / rawWidth, MAX_CANVAS_PX / rawHeight)
  const width = Math.max(1, Math.floor(rawWidth * fit))
  const height = Math.max(1, Math.floor(rawHeight * fit))
  const out = document.createElement("canvas")
  out.width = width
  out.height = height
  const ctx = out.getContext("2d")
  if (!ctx) {
    throw new Error("multi-page PNG too large to allocate — export as PDF or PPTX instead")
  }
  ctx.fillStyle = "#ffffff"
  ctx.fillRect(0, 0, width, height)
  let y = 0
  for (const c of canvases) {
    const w = c.width * fit
    const h = c.height * fit
    ctx.drawImage(c, Math.round((width - w) / 2), Math.round(y), Math.round(w), Math.round(h))
    y += h
  }
  return out
}

/** 导出 PNG（多页 deck 纵向拼成一张长图输出全部页；单页/其它取整页或画框）。 */
export async function exportPng(
  html: string,
  kind: ArtifactKind,
  vw?: number,
  opts?: ExportOpts,
): Promise<Blob> {
  const scale = scaleOf(opts)
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = slidesOf(h.doc)
    if (slides.length > 0) {
      // 逐片栅格化后拼成一张长图——不再只出首屏、丢掉其余幻灯片。
      const canvases: HTMLCanvasElement[] = []
      for (let i = 0; i < slides.length; i++) {
        slides.forEach((s, k) => s.classList.toggle("active", k === i))
        canvases.push(await rasterize(slides[i], scale))
        opts?.onProgress?.(i + 1, slides.length)
      }
      return canvasToPngBlob(canvases.length === 1 ? canvases[0] : stitchVertical(canvases))
    }
    return canvasToPngBlob(await rasterize(pickTarget(h.doc), scale))
  } finally {
    h.cleanup()
  }
}

/** 导出 PDF（deck 每片一页 16:9；其余整页单页，按内容尺寸）。 */
export async function exportPdf(
  html: string,
  kind: ArtifactKind,
  vw?: number,
  opts?: ExportOpts,
): Promise<Blob> {
  const scale = scaleOf(opts)
  const q = jpegQ(opts)
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = kind === "deck" ? slidesOf(h.doc) : []
    if (slides.length > 0) {
      const pdf = new jsPDF({ orientation: "landscape", unit: "px", format: [1280, 720] })
      for (let i = 0; i < slides.length; i++) {
        slides.forEach((s, k) => s.classList.toggle("active", k === i))
        const canvas = await rasterize(slides[i], scale)
        const img = canvas.toDataURL("image/jpeg", q)
        if (i > 0) pdf.addPage([1280, 720], "landscape")
        pdf.addImage(img, "JPEG", 0, 0, 1280, 720)
        opts?.onProgress?.(i + 1, slides.length)
      }
      return pdf.output("blob")
    }
    const canvas = await rasterize(pickTarget(h.doc), scale)
    const w = canvas.width
    const ht = canvas.height
    const pdf = new jsPDF({ orientation: w > ht ? "landscape" : "portrait", unit: "px", format: [w, ht] })
    pdf.addImage(canvas.toDataURL("image/jpeg", q), "JPEG", 0, 0, w, ht)
    return pdf.output("blob")
  } finally {
    h.cleanup()
  }
}

/** 栅格化各页为 PNG dataURL（供后端组装 PPTX）。 */
async function rasterizeSlideImages(
  html: string,
  kind: ArtifactKind,
  vw?: number,
  opts?: ExportOpts,
): Promise<string[]> {
  const scale = scaleOf(opts)
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = kind === "deck" ? slidesOf(h.doc) : []
    const out: string[] = []
    if (slides.length > 0) {
      for (let i = 0; i < slides.length; i++) {
        slides.forEach((s, k) => s.classList.toggle("active", k === i))
        out.push((await rasterize(slides[i], scale)).toDataURL("image/png"))
        opts?.onProgress?.(i + 1, slides.length)
      }
    } else {
      out.push((await rasterize(pickTarget(h.doc), scale)).toDataURL("image/png"))
      opts?.onProgress?.(1, 1)
    }
    return out
  } finally {
    h.cleanup()
  }
}

/** 导出 PPTX：前端栅格化 → 后端组装 zip → 返回 Blob。 */
export async function exportPptx(
  html: string,
  kind: ArtifactKind,
  title: string,
  vw?: number,
  opts?: ExportOpts,
): Promise<Blob> {
  const slides = await rasterizeSlideImages(html, kind, vw, opts)
  const res = await getTransport().call<{ pptx: string }>("export_design_pptx_cmd", { slides, title })
  const bin = atob(res.pptx)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return new Blob([bytes], {
    type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  })
}

/** base64（可含 data-uri 前缀）→ Blob。 */
export function base64ToBlob(b64: string, mime: string): Blob {
  const raw = b64.includes(",") ? b64.slice(b64.indexOf(",") + 1) : b64
  const bin = atob(raw)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return new Blob([bytes], { type: mime })
}

/** 触发浏览器下载一个 Blob。 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement("a")
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

/** 文件名安全化。 */
export function safeFilename(title: string): string {
  const s = title.replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-+|-+$/g, "")
  return s || "design"
}
