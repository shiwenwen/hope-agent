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
  // 等字体/布局稳定。
  await new Promise((r) => setTimeout(r, 300))
  const doc = iframe.contentDocument
  const win = iframe.contentWindow
  if (!doc || !win) {
    iframe.remove()
    URL.revokeObjectURL(url)
    throw new Error("export iframe has no document")
  }
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

/** 导出 PNG（deck/motion 取首屏；其余取整页/画框）。 */
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
      slides.forEach((s, k) => s.classList.toggle("active", k === 0))
      return canvasToPngBlob(await rasterize(slides[0], scale))
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
