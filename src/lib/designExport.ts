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
  await new Promise<void>((resolve, reject) => {
    iframe.onload = () => resolve()
    iframe.onerror = () => reject(new Error("export iframe failed to load"))
    iframe.src = url
  })
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

async function rasterize(el: HTMLElement): Promise<HTMLCanvasElement> {
  return html2canvas(el, {
    backgroundColor: "#ffffff",
    scale: 2,
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
export async function exportPng(html: string, kind: ArtifactKind, vw?: number): Promise<Blob> {
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = slidesOf(h.doc)
    if (slides.length > 0) {
      slides.forEach((s, k) => s.classList.toggle("active", k === 0))
      return canvasToPngBlob(await rasterize(slides[0]))
    }
    return canvasToPngBlob(await rasterize(pickTarget(h.doc)))
  } finally {
    h.cleanup()
  }
}

/** 导出 PDF（deck 每片一页 16:9；其余整页单页，按内容尺寸）。 */
export async function exportPdf(html: string, kind: ArtifactKind, vw?: number): Promise<Blob> {
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = kind === "deck" ? slidesOf(h.doc) : []
    if (slides.length > 0) {
      const pdf = new jsPDF({ orientation: "landscape", unit: "px", format: [1280, 720] })
      for (let i = 0; i < slides.length; i++) {
        slides.forEach((s, k) => s.classList.toggle("active", k === i))
        const canvas = await rasterize(slides[i])
        const img = canvas.toDataURL("image/jpeg", 0.92)
        if (i > 0) pdf.addPage([1280, 720], "landscape")
        pdf.addImage(img, "JPEG", 0, 0, 1280, 720)
      }
      return pdf.output("blob")
    }
    const canvas = await rasterize(pickTarget(h.doc))
    const w = canvas.width
    const ht = canvas.height
    const pdf = new jsPDF({ orientation: w > ht ? "landscape" : "portrait", unit: "px", format: [w, ht] })
    pdf.addImage(canvas.toDataURL("image/jpeg", 0.92), "JPEG", 0, 0, w, ht)
    return pdf.output("blob")
  } finally {
    h.cleanup()
  }
}

/** 栅格化各页为 PNG dataURL（供后端组装 PPTX）。 */
async function rasterizeSlideImages(html: string, kind: ArtifactKind, vw?: number): Promise<string[]> {
  const h = await renderHtml(html, kindWidth(kind, vw))
  try {
    const slides = kind === "deck" ? slidesOf(h.doc) : []
    const out: string[] = []
    if (slides.length > 0) {
      for (let i = 0; i < slides.length; i++) {
        slides.forEach((s, k) => s.classList.toggle("active", k === i))
        out.push((await rasterize(slides[i])).toDataURL("image/png"))
      }
    } else {
      out.push((await rasterize(pickTarget(h.doc))).toDataURL("image/png"))
    }
    return out
  } finally {
    h.cleanup()
  }
}

/** 导出 PPTX：前端栅格化 → 后端组装 zip → 返回 Blob。 */
export async function exportPptx(html: string, kind: ArtifactKind, title: string, vw?: number): Promise<Blob> {
  const slides = await rasterizeSlideImages(html, kind, vw)
  const res = await getTransport().call<{ pptx: string }>("export_design_pptx_cmd", { slides, title })
  const bin = atob(res.pptx)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return new Blob([bytes], {
    type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  })
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
